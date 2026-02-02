//! Sync Session Management for Offline-First Clients
//!
//! Tracks the sync state of individual clients (devices) including:
//! - Last sync vector (what version they have)
//! - Subscription filters (which documents they care about)
//! - Device authentication

use crate::sync::version_vector::{ConflictInfo, VersionVector};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

type HmacSha256 = Hmac<Sha256>;

/// Sign data with HMAC-SHA256
fn sign_hmac(data: &str, secret: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC can take key of any size");
    mac.update(data.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Verify HMAC signature
fn verify_hmac(data: &str, signature: &str, secret: &[u8]) -> bool {
    let expected = sign_hmac(data, secret);
    // Use constant-time comparison
    if expected.len() != signature.len() {
        return false;
    }
    let mut result = 0u8;
    for (a, b) in expected.bytes().zip(signature.bytes()) {
        result |= a ^ b;
    }
    result == 0
}

/// A sync session represents the state of a client device
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SyncSession {
    /// Unique session ID (device ID + client-generated nonce)
    pub session_id: String,
    /// Client device ID
    pub device_id: String,
    /// User ID this device belongs to
    pub user_id: Option<String>,
    /// API key used for authentication
    pub api_key: String,
    /// Last known sync vector for this device
    pub last_vector: VersionVector,
    /// Last synced sequence number (for sequence-based pull)
    pub last_sequence: u64,
    /// Optional filter query (SDBQL) for partial sync
    pub filter_query: Option<String>,
    /// Collections this device subscribes to
    pub subscriptions: Vec<String>,
    /// When the session was created
    pub created_at: u64,
    /// Last activity timestamp
    pub last_activity: u64,
    /// Is this session currently online?
    pub is_online: bool,
    /// Client capabilities (for feature negotiation)
    pub capabilities: ClientCapabilities,
}

/// Client capabilities for feature negotiation
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ClientCapabilities {
    /// Supports delta sync (JSON Patch)
    pub delta_sync: bool,
    /// Supports CRDT data types
    pub crdt_types: bool,
    /// Supports binary compression
    pub compression: bool,
    /// Maximum batch size in bytes
    pub max_batch_size: usize,
}

impl SyncSession {
    /// Create a new sync session
    pub fn new(
        session_id: impl Into<String>,
        device_id: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            session_id: session_id.into(),
            device_id: device_id.into(),
            user_id: None,
            api_key: api_key.into(),
            last_vector: VersionVector::new(),
            last_sequence: 0,
            filter_query: None,
            subscriptions: Vec::new(),
            created_at: now,
            last_activity: now,
            is_online: true,
            capabilities: ClientCapabilities::default(),
        }
    }

    /// Create a new secure sync session with HMAC-signed session ID
    ///
    /// The session ID format is: `{device_id}-{nonce}-{signature}`
    /// where signature = HMAC-SHA256(device_id + nonce + api_key, secret)
    pub fn new_secure(
        device_id: impl Into<String>,
        api_key: impl Into<String>,
        secret: &[u8],
    ) -> Self {
        let device_id = device_id.into();
        let api_key = api_key.into();
        let nonce = uuid::Uuid::new_v4().to_string();

        // Create signature over device_id + nonce + api_key
        let data = format!("{}{}{}", device_id, nonce, api_key);
        let signature = sign_hmac(&data, secret);

        // Session ID format: device_id-nonce-signature
        let session_id = format!("{}-{}-{}", device_id, nonce, signature);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            session_id,
            device_id,
            user_id: None,
            api_key,
            last_vector: VersionVector::new(),
            last_sequence: 0,
            filter_query: None,
            subscriptions: Vec::new(),
            created_at: now,
            last_activity: now,
            is_online: true,
            capabilities: ClientCapabilities::default(),
        }
    }

    /// Verify a session ID was signed with the correct secret
    ///
    /// Returns true if the session ID is valid and the signature matches.
    /// The api_key must be provided separately for verification.
    pub fn verify_session_id(session_id: &str, api_key: &str, secret: &[u8]) -> bool {
        // Session ID format: device_id-nonce-signature
        // Note: device_id and nonce are UUIDs which contain hyphens
        // So we need to find the signature (last 64 chars - hex encoded SHA256)

        // A SHA256 HMAC signature is 64 hex characters
        if session_id.len() < 65 {
            return false;
        }

        // Find the last hyphen before the signature
        let signature_start = session_id.len() - 64;
        if session_id.as_bytes().get(signature_start.saturating_sub(1)) != Some(&b'-') {
            return false;
        }

        let signature = &session_id[signature_start..];
        let prefix = &session_id[..signature_start.saturating_sub(1)];

        // Find the nonce (36 chars UUID) - it's at the end of prefix
        if prefix.len() < 37 {
            return false;
        }

        let nonce_start = prefix.len() - 36;
        if prefix.as_bytes().get(nonce_start.saturating_sub(1)) != Some(&b'-') {
            return false;
        }

        let nonce = &prefix[nonce_start..];
        let device_id = &prefix[..nonce_start.saturating_sub(1)];

        // Reconstruct the signed data
        let data = format!("{}{}{}", device_id, nonce, api_key);

        verify_hmac(&data, signature, secret)
    }

    /// Extract the device_id from a session ID
    pub fn extract_device_id(session_id: &str) -> Option<String> {
        // Session ID format: device_id-nonce-signature
        // Signature is 64 hex chars, nonce is 36 chars UUID

        if session_id.len() < 65 + 37 {
            return None;
        }

        let signature_start = session_id.len() - 64;
        let prefix = &session_id[..signature_start.saturating_sub(1)];

        if prefix.len() < 37 {
            return None;
        }

        let nonce_start = prefix.len() - 36;
        let device_id = &prefix[..nonce_start.saturating_sub(1)];

        Some(device_id.to_string())
    }

    /// Update the last sync vector
    pub fn update_vector(&mut self, vector: &VersionVector) {
        self.last_vector = vector.clone();
        self.last_activity = current_timestamp();
    }

    /// Update the last synced sequence number
    pub fn update_sequence(&mut self, sequence: u64) {
        self.last_sequence = sequence;
        self.last_activity = current_timestamp();
    }

    /// Add a collection subscription
    pub fn subscribe(&mut self, collection: impl Into<String>) {
        let coll = collection.into();
        if !self.subscriptions.contains(&coll) {
            self.subscriptions.push(coll);
        }
        self.last_activity = current_timestamp();
    }

    /// Remove a collection subscription
    pub fn unsubscribe(&mut self, collection: &str) {
        self.subscriptions.retain(|c| c != collection);
        self.last_activity = current_timestamp();
    }

    /// Mark session as online
    pub fn set_online(&mut self, online: bool) {
        self.is_online = online;
        if online {
            self.last_activity = current_timestamp();
        }
    }

    /// Check if session has expired (no activity for too long)
    pub fn is_expired(&self, max_inactive_ms: u64) -> bool {
        let now = current_timestamp();
        now - self.last_activity > max_inactive_ms
    }

    /// Convert to JSON value for storage
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// Create from JSON value
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

/// Manages all active sync sessions
pub struct SyncSessionManager {
    /// Active sessions by session ID
    sessions: Arc<RwLock<HashMap<String, SyncSession>>>,
    /// Index by device ID for quick lookup
    device_index: Arc<RwLock<HashMap<String, Vec<String>>>>,
    /// Maximum inactive time before session expires (default: 7 days)
    max_inactive_ms: u64,
}

impl SyncSessionManager {
    /// Create a new session manager
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            device_index: Arc::new(RwLock::new(HashMap::new())),
            max_inactive_ms: 7 * 24 * 60 * 60 * 1000, // 7 days
        }
    }

    /// Create a session with custom expiration time
    pub fn with_expiration(max_inactive_ms: u64) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            device_index: Arc::new(RwLock::new(HashMap::new())),
            max_inactive_ms,
        }
    }

    /// Register a new session
    pub async fn register_session(&self, session: SyncSession) {
        let session_id = session.session_id.clone();
        let device_id = session.device_id.clone();

        // Add to sessions map
        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id.clone(), session);
        drop(sessions);

        // Update device index
        let mut index = self.device_index.write().await;
        index
            .entry(device_id)
            .or_insert_with(Vec::new)
            .push(session_id);
    }

    /// Get a session by ID
    pub async fn get_session(&self, session_id: &str) -> Option<SyncSession> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).cloned()
    }

    /// Get all sessions for a device
    pub async fn get_device_sessions(&self, device_id: &str) -> Vec<SyncSession> {
        let index = self.device_index.read().await;
        let session_ids = index.get(device_id).cloned().unwrap_or_default();
        drop(index);

        let sessions = self.sessions.read().await;
        session_ids
            .iter()
            .filter_map(|id| sessions.get(id).cloned())
            .collect()
    }

    /// Update a session
    pub async fn update_session(&self, session: &SyncSession) {
        let mut sessions = self.sessions.write().await;
        if sessions.contains_key(&session.session_id) {
            sessions.insert(session.session_id.clone(), session.clone());
        }
    }

    /// Update just the sync vector for a session
    pub async fn update_session_vector(&self, session_id: &str, vector: &VersionVector) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.update_vector(vector);
        }
    }

    /// Update the last synced sequence for a session
    pub async fn update_session_sequence(&self, session_id: &str, sequence: u64) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.update_sequence(sequence);
        }
    }

    /// Remove a session
    pub async fn remove_session(&self, session_id: &str) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.remove(session_id) {
            drop(sessions);

            // Remove from device index
            let mut index = self.device_index.write().await;
            if let Some(ids) = index.get_mut(&session.device_id) {
                ids.retain(|id| id != session_id);
                if ids.is_empty() {
                    index.remove(&session.device_id);
                }
            }
        }
    }

    /// Remove all sessions for a device
    pub async fn remove_device_sessions(&self, device_id: &str) {
        let index = self.device_index.read().await;
        let session_ids = index.get(device_id).cloned().unwrap_or_default();
        drop(index);

        for session_id in session_ids {
            self.remove_session(&session_id).await;
        }
    }

    /// Mark a session as online/offline
    pub async fn set_session_online(&self, session_id: &str, online: bool) {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.set_online(online);
        }
    }

    /// Get all active (online) sessions
    pub async fn get_active_sessions(&self) -> Vec<SyncSession> {
        let sessions = self.sessions.read().await;
        sessions.values().filter(|s| s.is_online).cloned().collect()
    }

    /// Get all sessions subscribing to a collection
    pub async fn get_subscribers(&self, collection: &str) -> Vec<SyncSession> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|s| s.subscriptions.contains(&collection.to_string()))
            .cloned()
            .collect()
    }

    /// Clean up expired sessions
    pub async fn cleanup_expired(&self) -> usize {
        let expired: Vec<String> = {
            let sessions = self.sessions.read().await;
            sessions
                .values()
                .filter(|s| s.is_expired(self.max_inactive_ms))
                .map(|s| s.session_id.clone())
                .collect()
        };

        for session_id in &expired {
            self.remove_session(session_id).await;
        }

        expired.len()
    }

    /// Get count of active sessions
    pub async fn session_count(&self) -> usize {
        let sessions = self.sessions.read().await;
        sessions.len()
    }

    /// Get count of online sessions
    pub async fn online_count(&self) -> usize {
        let sessions = self.sessions.read().await;
        sessions.values().filter(|s| s.is_online).count()
    }
}

impl Default for SyncSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Request to register a new sync session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterSessionRequest {
    pub device_id: String,
    pub api_key: String,
    pub capabilities: Option<ClientCapabilities>,
    pub subscriptions: Option<Vec<String>>,
}

/// Response to session registration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterSessionResponse {
    pub session_id: String,
    pub server_capabilities: ClientCapabilities,
}

/// Request to pull changes from server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullRequest {
    pub session_id: String,
    /// Version vector of client (what they already have)
    pub client_vector: VersionVector,
    /// Optional filter query (SDBQL)
    pub filter: Option<String>,
    /// Maximum number of changes to return
    pub limit: Option<usize>,
}

/// Response with changes from server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullResponse {
    /// Changes to apply
    pub changes: Vec<SyncChange>,
    /// Server's version vector after these changes
    pub server_vector: VersionVector,
    /// Whether there are more changes to fetch
    pub has_more: bool,
}

/// Request to push changes to server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPushRequest {
    pub session_id: String,
    /// Changes from client
    pub changes: Vec<SyncChange>,
    /// Client's version vector before these changes
    pub client_vector: VersionVector,
}

/// Response to push request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPushResponse {
    /// Server's new version vector after applying changes
    pub server_vector: VersionVector,
    /// Conflicts that occurred (if any)
    pub conflicts: Vec<ConflictInfo>,
    /// Number of changes accepted
    pub accepted: usize,
    /// Number of changes rejected
    pub rejected: usize,
}

/// A single change entry in sync
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncChange {
    /// Database name
    pub database: String,
    /// Collection name
    pub collection: String,
    /// Document key
    pub document_key: String,
    /// Operation type
    pub operation: ChangeOperation,
    /// Document data (for insert/update)
    pub document_data: Option<serde_json::Value>,
    /// Parent version vectors (causal history)
    pub parent_vectors: Vec<VersionVector>,
    /// Vector clock after this change
    pub vector: VersionVector,
    /// HLC timestamp
    pub timestamp: u64,
    /// Is this a delta (patch) or full document?
    pub is_delta: bool,
    /// Delta patch (if is_delta is true)
    pub delta_patch: Option<serde_json::Value>,
}

/// Type of change operation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeOperation {
    Insert,
    Update,
    Delete,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_lifecycle() {
        let manager = SyncSessionManager::new();

        // Register session
        let session = SyncSession::new("sess-1", "device-1", "api-key-1");
        manager.register_session(session.clone()).await;

        assert_eq!(manager.session_count().await, 1);

        // Get session
        let retrieved = manager.get_session("sess-1").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().device_id, "device-1");

        // Update vector
        let mut vector = VersionVector::new();
        vector.increment("device-1");
        manager.update_session_vector("sess-1", &vector).await;

        let updated = manager.get_session("sess-1").await.unwrap();
        assert_eq!(updated.last_vector.get("device-1"), 1);

        // Remove session
        manager.remove_session("sess-1").await;
        assert_eq!(manager.session_count().await, 0);
    }

    #[tokio::test]
    async fn test_subscriptions() {
        let manager = SyncSessionManager::new();

        let mut session = SyncSession::new("sess-1", "device-1", "api-key-1");
        session.subscribe("orders");
        session.subscribe("products");
        manager.register_session(session).await;

        let subscribers = manager.get_subscribers("orders").await;
        assert_eq!(subscribers.len(), 1);

        let subscribers = manager.get_subscribers("users").await;
        assert_eq!(subscribers.len(), 0);
    }

    #[test]
    fn test_session_expiration() {
        let mut session = SyncSession::new("sess-1", "device-1", "api-key-1");
        session.last_activity = 0; // Very old

        assert!(session.is_expired(1000));
        assert!(!session.is_expired(u64::MAX));
    }

    #[test]
    fn test_secure_session_creation() {
        let secret = b"test-cluster-secret-key-12345678";
        let session = SyncSession::new_secure("device-123", "api-key-abc", secret);

        // Session should have the device_id
        assert_eq!(session.device_id, "device-123");
        assert_eq!(session.api_key, "api-key-abc");

        // Session ID should be properly formatted
        assert!(session.session_id.starts_with("device-123-"));
        // Should end with 64 hex chars (SHA256 signature)
        assert!(session.session_id.len() > 64);
    }

    #[test]
    fn test_verify_session_id_valid() {
        let secret = b"test-cluster-secret-key-12345678";
        let session = SyncSession::new_secure("device-123", "api-key-abc", secret);

        // Should verify correctly with same api_key and secret
        assert!(SyncSession::verify_session_id(
            &session.session_id,
            "api-key-abc",
            secret
        ));
    }

    #[test]
    fn test_verify_session_id_wrong_api_key() {
        let secret = b"test-cluster-secret-key-12345678";
        let session = SyncSession::new_secure("device-123", "api-key-abc", secret);

        // Should fail with wrong api_key
        assert!(!SyncSession::verify_session_id(
            &session.session_id,
            "wrong-api-key",
            secret
        ));
    }

    #[test]
    fn test_verify_session_id_wrong_secret() {
        let secret = b"test-cluster-secret-key-12345678";
        let wrong_secret = b"wrong-cluster-secret-key-1234567";
        let session = SyncSession::new_secure("device-123", "api-key-abc", secret);

        // Should fail with wrong secret
        assert!(!SyncSession::verify_session_id(
            &session.session_id,
            "api-key-abc",
            wrong_secret
        ));
    }

    #[test]
    fn test_verify_session_id_tampered() {
        let secret = b"test-cluster-secret-key-12345678";
        let session = SyncSession::new_secure("device-123", "api-key-abc", secret);

        // Tamper with the session ID
        let mut tampered = session.session_id.clone();
        if let Some(last_char) = tampered.pop() {
            // Change the last character
            let new_char = if last_char == 'a' { 'b' } else { 'a' };
            tampered.push(new_char);
        }

        // Should fail verification
        assert!(!SyncSession::verify_session_id(
            &tampered,
            "api-key-abc",
            secret
        ));
    }

    #[test]
    fn test_extract_device_id() {
        let secret = b"test-cluster-secret-key-12345678";
        let session = SyncSession::new_secure("my-device-id", "api-key-abc", secret);

        let extracted = SyncSession::extract_device_id(&session.session_id);
        assert_eq!(extracted, Some("my-device-id".to_string()));
    }

    #[test]
    fn test_extract_device_id_invalid() {
        // Too short session ID
        assert!(SyncSession::extract_device_id("short").is_none());

        // Session ID without proper format
        assert!(SyncSession::extract_device_id("invalid-session-id").is_none());
    }
}
