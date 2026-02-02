//! Mobile FFI Layer - UniFFI bindings for iOS and Android
//!
//! This module wraps the Rust sync functionality and exposes it via FFI
//! for Swift (iOS) and Kotlin (Android) consumption.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::runtime::Runtime;

// Re-export from our sync module
use crate::client::HttpClient;
use crate::sync::store::LocalStore;
use crate::sync::{SyncConfig as RustSyncConfig, SyncManager as RustSyncManager};

uniffi::include_scaffolding!("solidb_client");

/// Error types for mobile clients
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum SyncError {
    #[error("Network error: {0}")]
    NetworkError(String),
    #[error("Database error: {0}")]
    DatabaseError(String),
    #[error("Conflict error: {0}")]
    ConflictError(String),
    #[error("Authentication error: {0}")]
    AuthError(String),
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Invalid data: {0}")]
    InvalidData(String),
}

impl From<Box<dyn std::error::Error + Send + Sync>> for SyncError {
    fn from(e: Box<dyn std::error::Error + Send + Sync>) -> Self {
        SyncError::DatabaseError(e.to_string())
    }
}

impl From<serde_json::Error> for SyncError {
    fn from(e: serde_json::Error) -> Self {
        SyncError::InvalidData(e.to_string())
    }
}

/// Sync configuration for mobile
#[derive(Debug, Clone, uniffi::Record)]
pub struct SyncConfig {
    pub device_id: String,
    pub server_url: Option<String>,
    pub api_key: Option<String>,
    pub collections: Option<Vec<String>>,
    pub sync_interval_secs: u64,
    pub max_retries: u64,
    pub auto_sync: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            device_id: generate_device_id(),
            server_url: None,
            api_key: None,
            collections: None,
            sync_interval_secs: 30,
            max_retries: 5,
            auto_sync: true,
        }
    }
}

/// Document wrapper for FFI
#[derive(Debug, Clone, uniffi::Record)]
pub struct Document {
    pub id: String,
    pub data: String,
}

/// Sync change entry
#[derive(Debug, Clone, uniffi::Record)]
pub struct SyncChange {
    pub collection: String,
    pub document_key: String,
    pub operation: String,
    pub data: Option<String>,
}

/// Sync operation result
#[derive(Debug, Clone, uniffi::Record)]
pub struct SyncResult {
    pub success: bool,
    pub pulled: u64,
    pub pushed: u64,
    pub conflicts: u64,
    pub errors: Vec<String>,
}

/// Conflict information
#[derive(Debug, Clone, uniffi::Record)]
pub struct ConflictInfo {
    pub document_key: String,
    pub collection: String,
    pub local_data: String,
    pub remote_data: String,
}

/// Pending change
#[derive(Debug, Clone, uniffi::Record)]
pub struct PendingChange {
    pub id: u64,
    pub collection: String,
    pub document_key: String,
    pub operation: String,
    pub data: Option<String>,
    pub retry_count: u32,
}

/// Mobile-friendly sync manager wrapper
pub struct SyncManager {
    inner: Arc<RustSyncManager>,
    runtime: Runtime,
    device_id: String,
}

impl SyncManager {
    /// Create a new sync manager with the given configuration
    pub fn new(config: SyncConfig) -> Result<Self, SyncError> {
        let runtime = Runtime::new().map_err(|e| SyncError::DatabaseError(e.to_string()))?;

        // Create local store
        let store = LocalStore::open_default("solidb_mobile", config.device_id.clone())
            .map_err(|e| SyncError::DatabaseError(e.to_string()))?;

        // Create HTTP client if server URL provided
        let client = if let Some(url) = &config.server_url {
            runtime
                .block_on(async {
                    // Simple HTTP client creation
                    // In production, you'd want proper error handling here
                    Ok(HttpClient::new(url))
                })
                .map_err(|e: Box<dyn std::error::Error + Send + Sync>| {
                    SyncError::NetworkError(e.to_string())
                })?
        } else {
            // Create a dummy client - operations will queue locally
            HttpClient::new("http://localhost:6745")
        };

        // Create rust sync config
        let rust_config = RustSyncConfig {
            sync_interval_secs: config.sync_interval_secs,
            batch_size: 100,
            max_retries: config.max_retries as u32,
            auto_sync: config.auto_sync,
            collections: config.collections.unwrap_or_default(),
        };

        let inner = RustSyncManager::new(store, client, rust_config);

        Ok(Self {
            inner: Arc::new(inner),
            runtime,
            device_id: config.device_id,
        })
    }

    /// Start the sync manager
    pub fn start(&self) {
        let inner = self.inner.clone();
        self.runtime.block_on(async {
            let mut manager = inner.as_ref();
            // manager.start().await;
        });
    }

    /// Stop the sync manager
    pub fn stop(&self) {
        let inner = self.inner.clone();
        self.runtime.block_on(async {
            inner.stop().await;
        });
    }

    /// Check if device is online
    pub fn is_online(&self) -> bool {
        true // Placeholder - would check actual network state
    }

    /// Set online/offline status
    pub fn set_online(&self, online: bool) {
        let inner = self.inner.clone();
        self.runtime.block_on(async {
            inner.set_online(online).await;
        });
    }

    /// Save a document locally
    pub fn save_document(&self, collection: &str, key: &str, data: &str) -> Result<(), SyncError> {
        let json_data: serde_json::Value =
            serde_json::from_str(data).map_err(|e| SyncError::InvalidData(e.to_string()))?;

        let inner = self.inner.clone();
        self.runtime.block_on(async {
            inner
                .save_document(collection, key, &json_data)
                .await
                .map_err(|e: Box<dyn std::error::Error + Send + Sync>| {
                    SyncError::DatabaseError(e.to_string())
                })
        })
    }

    /// Get a document by key
    pub fn get_document(&self, collection: &str, key: &str) -> Result<Option<String>, SyncError> {
        let inner = self.inner.clone();
        self.runtime.block_on(async {
            let result = inner.get_document(collection, key).await.map_err(
                |e: Box<dyn std::error::Error + Send + Sync>| {
                    SyncError::DatabaseError(e.to_string())
                },
            )?;

            match result {
                Some(doc) => {
                    let json_str = serde_json::to_string(&doc)
                        .map_err(|e| SyncError::InvalidData(e.to_string()))?;
                    Ok(Some(json_str))
                }
                None => Ok(None),
            }
        })
    }

    /// Delete a document
    pub fn delete_document(&self, collection: &str, key: &str) -> Result<(), SyncError> {
        let inner = self.inner.clone();
        self.runtime.block_on(async {
            inner.delete_document(collection, key).await.map_err(
                |e: Box<dyn std::error::Error + Send + Sync>| {
                    SyncError::DatabaseError(e.to_string())
                },
            )
        })
    }

    /// Query all documents in a collection
    pub fn query_documents(&self, collection: &str) -> Result<Vec<Document>, SyncError> {
        let inner = self.inner.clone();
        self.runtime.block_on(async {
            let docs = inner.query_documents(collection).await.map_err(
                |e: Box<dyn std::error::Error + Send + Sync>| {
                    SyncError::DatabaseError(e.to_string())
                },
            )?;

            let result: Vec<Document> = docs
                .into_iter()
                .map(|(key, data)| {
                    let data_str =
                        serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_string());
                    Document {
                        id: key,
                        data: data_str,
                    }
                })
                .collect();

            Ok(result)
        })
    }

    /// Trigger manual sync
    pub fn sync_now(&self) -> Result<SyncResult, SyncError> {
        let inner = self.inner.clone();
        self.runtime.block_on(async {
            inner.sync_now().await;

            // Get sync stats
            let (last_sync, pending) = inner.get_state().await;

            Ok(SyncResult {
                success: true,
                pulled: 0, // Would get from actual sync
                pushed: 0,
                conflicts: 0,
                errors: vec![],
            })
        })
    }

    /// Get pending changes count
    pub fn get_pending_count(&self) -> u64 {
        let inner = self.inner.clone();
        self.runtime.block_on(async {
            let (_, count) = inner.get_state().await;
            count as u64
        })
    }

    /// Get list of pending changes
    pub fn get_pending_changes(&self) -> Vec<PendingChange> {
        // This would need to be implemented in the LocalStore
        // For now, return empty
        vec![]
    }

    /// Subscribe to a collection
    pub fn subscribe_collection(&self, collection: &str) {
        let inner = self.inner.clone();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            inner.subscribe(&collection, None).await;
        });
    }

    /// Unsubscribe from a collection
    pub fn unsubscribe_collection(&self, collection: &str) {
        let inner = self.inner.clone();
        let collection = collection.to_string();
        self.runtime.block_on(async {
            inner.unsubscribe(&collection).await;
        });
    }

    /// Get subscribed collections
    pub fn get_subscriptions(&self) -> Vec<String> {
        // Would need to track this in the manager
        vec![]
    }

    /// Get conflicts (would need conflict tracking)
    pub fn get_conflicts(&self) -> Vec<ConflictInfo> {
        vec![]
    }

    /// Resolve a conflict
    pub fn resolve_conflict(
        &self,
        _document_key: &str,
        _resolution: &str,
        _merged_data: Option<String>,
    ) -> Result<(), SyncError> {
        // Would implement conflict resolution here
        Ok(())
    }

    /// Get last sync time
    pub fn get_last_sync_time(&self) -> Option<String> {
        let inner = self.inner.clone();
        self.runtime.block_on(async {
            let (timestamp, _) = inner.get_state().await;
            timestamp.map(|ts| {
                let datetime = chrono::DateTime::from_timestamp_millis(ts as i64);
                datetime.map(|dt| dt.to_rfc3339()).unwrap_or_default()
            })
        })
    }

    /// Get stats as JSON string
    pub fn get_stats(&self) -> String {
        let pending = self.get_pending_count();
        let last_sync = self.get_last_sync_time();

        serde_json::json!({
            "device_id": self.device_id,
            "pending_changes": pending,
            "last_sync": last_sync,
            "is_online": self.is_online(),
        })
        .to_string()
    }
}

/// Generate a unique device ID
fn generate_device_id() -> String {
    use uuid::Uuid;
    Uuid::new_v4().to_string()
}

/// Utility functions module
pub mod utils {
    use super::*;

    /// Generate a unique device ID
    pub fn generate_device_id() -> String {
        use uuid::Uuid;
        Uuid::new_v4().to_string()
    }

    /// Check if string is valid JSON
    pub fn is_valid_json(data: &str) -> bool {
        serde_json::from_str::<serde_json::Value>(data).is_ok()
    }

    /// Get SDK version
    pub fn version() -> String {
        env!("CARGO_PKG_VERSION").to_string()
    }
}
