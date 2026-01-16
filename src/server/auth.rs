use crate::error::DbError;
use crate::server::authorization::{Role, UserRole};
use crate::storage::StorageEngine;
use crate::sync::log::SyncLog;
use crate::sync::{LogEntry, Operation};

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};

use once_cell::sync::Lazy;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Rate limiting configuration
const MAX_LOGIN_ATTEMPTS: usize = 20;
const RATE_LIMIT_WINDOW_SECS: u64 = 60;

/// In-memory rate limiter for login attempts
/// Tracks attempts per IP address with automatic cleanup
static LOGIN_RATE_LIMITER: Lazy<RwLock<HashMap<String, Vec<Instant>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Check if an IP is rate limited, return error if too many attempts
pub fn check_rate_limit(ip: &str) -> Result<(), crate::error::DbError> {
    let now = Instant::now();
    let window = std::time::Duration::from_secs(RATE_LIMIT_WINDOW_SECS);

    let mut limiter = LOGIN_RATE_LIMITER
        .write()
        .unwrap_or_else(|e| e.into_inner());

    // Get or create entry for this IP
    let attempts = limiter.entry(ip.to_string()).or_insert_with(Vec::new);

    // Remove old attempts outside the window
    attempts.retain(|t| now.duration_since(*t) < window);

    // Check if rate limited
    if attempts.len() >= MAX_LOGIN_ATTEMPTS {
        return Err(crate::error::DbError::BadRequest(format!(
            "Too many login attempts. Please wait {} seconds before trying again.",
            RATE_LIMIT_WINDOW_SECS
        )));
    }

    // Record this attempt
    attempts.push(now);

    Ok(())
}

const ADMIN_DB: &str = "_system";
pub const ADMIN_COLL: &str = "_admins";
pub const API_KEYS_COLL: &str = "_api_keys";
pub const ROLES_COLL: &str = "_roles";
pub const USER_ROLES_COLL: &str = "_user_roles";
const DEFAULT_USER: &str = "admin";
const RBAC_CONFIG_KEY: &str = "rbac_migrated";

// Secret for JWT signing - MUST be set via JWT_SECRET env var in production
static JWT_SECRET: Lazy<String> = Lazy::new(|| {
    match std::env::var("JWT_SECRET") {
        Ok(secret) => {
            if secret.len() < 32 {
                tracing::warn!(
                    "⚠️  JWT_SECRET is less than 32 characters - consider using a longer secret"
                );
            }
            secret
        }
        Err(_) => {
            // Generate a random secret for development - tokens will be invalid after restart
            let mut key_bytes = [0u8; 32];
            OsRng.fill_bytes(&mut key_bytes);
            let generated = hex::encode(key_bytes);
            tracing::warn!("╔══════════════════════════════════════════════════════════════════╗");
            tracing::warn!("║  ⚠️  JWT_SECRET environment variable is not set!                 ║");
            tracing::warn!("║  A random secret has been generated for this session.            ║");
            tracing::warn!("║  All tokens will be INVALID after server restart.                ║");
            tracing::warn!("║                                                                  ║");
            tracing::warn!("║  For production, set JWT_SECRET to a secure 32+ character value: ║");
            tracing::warn!(
                "║    export JWT_SECRET=\"your-secure-random-secret-here\"            ║"
            );
            tracing::warn!("╚══════════════════════════════════════════════════════════════════╝");
            generated
        }
    }
});

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String, // username
    pub exp: usize,  // expiration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub livequery: Option<bool>, // If true, this token is only valid for live queries
    /// Role names assigned to this user (for RBAC)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    /// Database restrictions (for scoped API keys)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoped_databases: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "_key")]
    pub username: String,
    pub password_hash: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiKey {
    #[serde(rename = "_key")]
    pub id: String,
    pub name: String,
    pub key_hash: String,
    pub created_at: String,
    /// Role names assigned to this API key (for RBAC)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Database restrictions (None means all databases)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoped_databases: Option<Vec<String>>,
    /// Optional expiration timestamp (RFC3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key: String, // Only returned on creation
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyListItem {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub roles: Vec<String>,
    pub scoped_databases: Option<Vec<String>>,
}

pub struct AuthService;

impl AuthService {
    /// Initialize authentication system
    /// Checks if admin user exists, if not creates default
    pub fn init(storage: &StorageEngine, replication_log: Option<&SyncLog>) -> Result<(), DbError> {
        // Force JWT_SECRET initialization to show warning at startup if not configured
        let _ = JWT_SECRET.len();

        let db = storage.get_database(ADMIN_DB)?;

        // Check for cluster mode with peers (joining node)
        // If we have peers, we expect to sync data, so we SHOULD NOT create default admins/api_keys
        // Unless there is an explicit password override
        let is_joining_cluster = storage
            .cluster_config()
            .map(|c| !c.peers.is_empty())
            .unwrap_or(false);

        let has_override_password = std::env::var("SOLIDB_ADMIN_PASSWORD")
            .map(|p| !p.is_empty())
            .unwrap_or(false);

        let should_skip_defaults = is_joining_cluster && !has_override_password;

        // Ensure _admins collection exists
        if let Err(DbError::CollectionNotFound(_)) = db.get_collection(ADMIN_COLL) {
            if should_skip_defaults {
                tracing::info!(
                    "Cluster join detected: Skipping {} creation (waiting for sync)",
                    ADMIN_COLL
                );
            } else {
                tracing::info!("Creating {} collection", ADMIN_COLL);
                db.create_collection(ADMIN_COLL.to_string(), None)?;
            }
        }

        // Ensure _api_keys collection exists
        if let Err(DbError::CollectionNotFound(_)) = db.get_collection(API_KEYS_COLL) {
            if should_skip_defaults {
                tracing::info!(
                    "Cluster join detected: Skipping {} creation (waiting for sync)",
                    API_KEYS_COLL
                );
            } else {
                tracing::info!("Creating {} collection", API_KEYS_COLL);
                db.create_collection(API_KEYS_COLL.to_string(), None)?;
            }
        }

        // Check if any admin exists
        // Use if let Ok to handle case where we skipped creation above
        if let Ok(collection) = db.get_collection(ADMIN_COLL) {
            if collection.count() == 0 {
                if should_skip_defaults {
                    tracing::info!("Cluster join detected: Skipping default admin user creation (waiting for sync)");
                } else {
                    // Check for override password (for testing/development)
                    // If SOLIDB_ADMIN_PASSWORD is set, use it; otherwise generate random
                    let (password, is_override) = match std::env::var("SOLIDB_ADMIN_PASSWORD") {
                        Ok(pwd) if !pwd.is_empty() => (pwd, true),
                        _ => {
                            // Generate a secure random password for production
                            let mut password_bytes = [0u8; 16];
                            OsRng.fill_bytes(&mut password_bytes);
                            (hex::encode(password_bytes), false)
                        }
                    };

                    let salt = SaltString::generate(&mut OsRng);
                    let argon2 = Argon2::default();
                    let password_hash = argon2
                        .hash_password(password.as_bytes(), &salt)
                        .map_err(|e| DbError::InternalError(format!("Hashing error: {}", e)))?
                        .to_string();

                    let user = User {
                        username: DEFAULT_USER.to_string(),
                        password_hash,
                    };

                    let doc_value = serde_json::to_value(user).map_err(|e| {
                        DbError::InternalError(format!("Serialization error: {}", e))
                    })?;

                    collection.insert(doc_value.clone())?; // Clone for recording

                    // Record write for replication
                    if let Some(log) = replication_log {
                        let entry = LogEntry {
                            sequence: 0,
                            node_id: "".to_string(), // implementation log fills this
                            database: ADMIN_DB.to_string(),
                            collection: ADMIN_COLL.to_string(),
                            operation: Operation::Insert,
                            key: DEFAULT_USER.to_string(),
                            data: serde_json::to_vec(&doc_value).ok(),
                            timestamp: chrono::Utc::now().timestamp_millis() as u64,
                            origin_sequence: None,
                        };
                        let _ = log.append(entry);
                    }

                    if is_override {
                        tracing::warn!(
                            "Admin user created with password from SOLIDB_ADMIN_PASSWORD env var"
                        );
                    } else {
                        tracing::warn!(
                            "╔══════════════════════════════════════════════════════════════════╗"
                        );
                        tracing::warn!(
                            "║              INITIAL ADMIN ACCOUNT CREATED                       ║"
                        );
                        tracing::warn!(
                            "╠══════════════════════════════════════════════════════════════════╣"
                        );
                        tracing::warn!(
                            "║  Username: admin                                                 ║"
                        );
                        tracing::warn!("║  Password: {}                             ║", password);
                        tracing::warn!(
                            "║                                                                  ║"
                        );
                        tracing::warn!(
                            "║  ⚠️  SAVE THIS PASSWORD! It will not be shown again.             ║"
                        );
                        tracing::warn!(
                            "║  Change it after first login via the API.                        ║"
                        );
                        tracing::warn!(
                            "╚══════════════════════════════════════════════════════════════════╝"
                        );
                    }
                }
            }
        }

        // Initialize RBAC system collections
        Self::init_rbac(storage, replication_log, should_skip_defaults)?;

        Ok(())
    }

    /// Initialize RBAC system: create collections, builtin roles, and migrate existing users
    fn init_rbac(
        storage: &StorageEngine,
        replication_log: Option<&SyncLog>,
        should_skip_defaults: bool,
    ) -> Result<(), DbError> {
        let db = storage.get_database(ADMIN_DB)?;

        // Ensure _roles collection exists
        if let Err(DbError::CollectionNotFound(_)) = db.get_collection(ROLES_COLL) {
            if should_skip_defaults {
                tracing::info!(
                    "Cluster join detected: Skipping {} creation (waiting for sync)",
                    ROLES_COLL
                );
            } else {
                tracing::info!("Creating {} collection for RBAC", ROLES_COLL);
                db.create_collection(ROLES_COLL.to_string(), None)?;
            }
        }

        // Ensure _user_roles collection exists
        if let Err(DbError::CollectionNotFound(_)) = db.get_collection(USER_ROLES_COLL) {
            if should_skip_defaults {
                tracing::info!(
                    "Cluster join detected: Skipping {} creation (waiting for sync)",
                    USER_ROLES_COLL
                );
            } else {
                tracing::info!("Creating {} collection for RBAC", USER_ROLES_COLL);
                db.create_collection(USER_ROLES_COLL.to_string(), None)?;
            }
        }

        // Ensure _config collection exists for migration tracking
        let config_coll = "_config";
        if let Err(DbError::CollectionNotFound(_)) = db.get_collection(config_coll) {
            if !should_skip_defaults {
                tracing::info!(
                    "Creating {} collection for system configuration",
                    config_coll
                );
                db.create_collection(config_coll.to_string(), None)?;
            }
        }

        // Skip the rest if joining cluster (will sync from peers)
        if should_skip_defaults {
            return Ok(());
        }

        // Check if RBAC has already been initialized
        let already_migrated = if let Ok(config) = db.get_collection(config_coll) {
            config.get(RBAC_CONFIG_KEY).is_ok()
        } else {
            false
        };

        if already_migrated {
            tracing::debug!("RBAC already initialized, skipping migration");
            return Ok(());
        }

        // Initialize builtin roles
        if let Ok(roles_coll) = db.get_collection(ROLES_COLL) {
            for role in Role::builtin_roles() {
                // Only insert if role doesn't exist
                if roles_coll.get(&role.name).is_err() {
                    let role_value = serde_json::to_value(&role).map_err(|e| {
                        DbError::InternalError(format!("Serialization error: {}", e))
                    })?;
                    roles_coll.insert(role_value.clone())?;
                    tracing::info!("Created builtin role: {}", role.name);

                    // Record for replication
                    if let Some(log) = replication_log {
                        let entry = LogEntry {
                            sequence: 0,
                            node_id: "".to_string(),
                            database: ADMIN_DB.to_string(),
                            collection: ROLES_COLL.to_string(),
                            operation: Operation::Insert,
                            key: role.name.clone(),
                            data: serde_json::to_vec(&role_value).ok(),
                            timestamp: chrono::Utc::now().timestamp_millis() as u64,
                            origin_sequence: None,
                        };
                        let _ = log.append(entry);
                    }
                }
            }
        }

        // Migrate existing users: assign admin role to all existing users
        Self::migrate_existing_users_to_admin(storage, replication_log)?;

        // Migrate existing API keys: assign admin role
        Self::migrate_existing_api_keys_to_admin(storage, replication_log)?;

        // Mark RBAC as initialized
        if let Ok(config) = db.get_collection(config_coll) {
            let migration_record = serde_json::json!({
                "_key": RBAC_CONFIG_KEY,
                "migrated_at": chrono::Utc::now().to_rfc3339(),
                "version": "1.0"
            });
            config.insert(migration_record)?;
            tracing::info!("RBAC migration completed successfully");
        }

        Ok(())
    }

    /// Migrate existing users to have admin role
    fn migrate_existing_users_to_admin(
        storage: &StorageEngine,
        replication_log: Option<&SyncLog>,
    ) -> Result<(), DbError> {
        let db = storage.get_database(ADMIN_DB)?;
        let admins_coll = db.get_collection(ADMIN_COLL)?;
        let user_roles_coll = db.get_collection(USER_ROLES_COLL)?;

        // Get all existing admin users
        for doc in admins_coll.scan(None) {
            let user: User = serde_json::from_value(doc.to_value())
                .map_err(|e| DbError::InternalError(format!("Invalid user data: {}", e)))?;

            // Check if user already has a role assignment
            let mut existing_assignment = false;
            for d in user_roles_coll.scan(None) {
                if let Ok(ur) = serde_json::from_value::<UserRole>(d.to_value()) {
                    if ur.username == user.username {
                        existing_assignment = true;
                        break;
                    }
                }
            }

            if !existing_assignment {
                // Assign admin role to existing user
                let user_role = UserRole::new_global(&user.username, "admin", "migration");
                let user_role_value = serde_json::to_value(&user_role)
                    .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;

                user_roles_coll.insert(user_role_value.clone())?;
                tracing::info!("Migrated user '{}' to admin role", user.username);

                // Record for replication
                if let Some(log) = replication_log {
                    let entry = LogEntry {
                        sequence: 0,
                        node_id: "".to_string(),
                        database: ADMIN_DB.to_string(),
                        collection: USER_ROLES_COLL.to_string(),
                        operation: Operation::Insert,
                        key: user_role.id.clone(),
                        data: serde_json::to_vec(&user_role_value).ok(),
                        timestamp: chrono::Utc::now().timestamp_millis() as u64,
                        origin_sequence: None,
                    };
                    let _ = log.append(entry);
                }
            }
        }

        Ok(())
    }

    /// Migrate existing API keys to have admin role
    fn migrate_existing_api_keys_to_admin(
        storage: &StorageEngine,
        replication_log: Option<&SyncLog>,
    ) -> Result<(), DbError> {
        let db = storage.get_database(ADMIN_DB)?;
        let api_keys_coll = db.get_collection(API_KEYS_COLL)?;

        // Get all existing API keys and add admin role if not already set
        for doc in api_keys_coll.scan(None) {
            let api_key: ApiKey = serde_json::from_value(doc.to_value())
                .map_err(|e| DbError::InternalError(format!("Invalid API key data: {}", e)))?;

            // Only migrate if roles is empty (backward compatibility)
            if api_key.roles.is_empty() {
                let mut updated_key = api_key.clone();
                updated_key.roles = vec!["admin".to_string()];

                let updated_value = serde_json::to_value(&updated_key)
                    .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;

                api_keys_coll.update(&api_key.id, updated_value.clone())?;
                tracing::info!("Migrated API key '{}' to admin role", api_key.name);

                // Record for replication
                if let Some(log) = replication_log {
                    let entry = LogEntry {
                        sequence: 0,
                        node_id: "".to_string(),
                        database: ADMIN_DB.to_string(),
                        collection: API_KEYS_COLL.to_string(),
                        operation: Operation::Update,
                        key: api_key.id.clone(),
                        data: serde_json::to_vec(&updated_value).ok(),
                        timestamp: chrono::Utc::now().timestamp_millis() as u64,
                        origin_sequence: None,
                    };
                    let _ = log.append(entry);
                }
            }
        }

        Ok(())
    }

    /// Verify password against hash
    pub fn verify_password(password: &str, hash: &str) -> bool {
        let parsed_hash = match PasswordHash::new(hash) {
            Ok(h) => h,
            Err(_) => return false,
        };

        Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok()
    }

    /// Hash a password using Argon2
    pub fn hash_password(password: &str) -> Result<String, DbError> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        argon2
            .hash_password(password.as_bytes(), &salt)
            .map(|hash| hash.to_string())
            .map_err(|e| DbError::InternalError(format!("Hashing error: {}", e)))
    }

    /// Create JWT for user
    pub fn create_jwt(username: &str) -> Result<String, DbError> {
        Self::create_jwt_with_roles(username, None, None)
    }

    /// Create JWT for user with roles
    pub fn create_jwt_with_roles(
        username: &str,
        roles: Option<Vec<String>>,
        scoped_databases: Option<Vec<String>>,
    ) -> Result<String, DbError> {
        let expiration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize
            + 24 * 3600; // 24 hours

        let claims = Claims {
            sub: username.to_owned(),
            exp: expiration,
            livequery: None,
            roles,
            scoped_databases,
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(JWT_SECRET.as_bytes()),
        )
        .map_err(|e| DbError::InternalError(format!("Token creation failed: {}", e)))
    }

    /// Create a short-lived JWT token specifically for live query WebSocket connections.
    /// This token expires in 30 seconds - just enough time to establish a WebSocket connection.
    /// The livequery claim can be used to restrict what operations this token allows.
    pub fn create_livequery_jwt() -> Result<String, DbError> {
        let expiration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize
            + 2; // 2 seconds - ultra short lived for file downloads!

        let claims = Claims {
            sub: "livequery".to_owned(),
            exp: expiration,
            livequery: Some(true),
            roles: None,
            scoped_databases: None,
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(JWT_SECRET.as_bytes()),
        )
        .map_err(|e| DbError::InternalError(format!("Token creation failed: {}", e)))
    }

    /// Validate JWT and return claims
    pub fn validate_token(token: &str) -> Result<Claims, DbError> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(JWT_SECRET.as_bytes()),
            &Validation::new(Algorithm::HS256),
        )
        .map_err(|_| DbError::BadRequest("Invalid token".to_string()))?;

        Ok(token_data.claims)
    }

    /// Generate a new API key (returns the raw key and its SHA-256 hash)
    /// Uses SHA-256 instead of Argon2 for fast validation (API keys have high entropy)
    pub fn generate_api_key() -> (String, String) {
        // Generate 32 random bytes for the key
        let mut key_bytes = [0u8; 32];
        use rand_core::RngCore;
        OsRng.fill_bytes(&mut key_bytes);

        // Format as sk_<hex>
        let raw_key = format!("sk_{}", hex::encode(key_bytes));

        // Hash the key with SHA-256 (fast, secure for high-entropy keys)
        let key_hash = Self::hash_api_key(&raw_key);

        (raw_key, key_hash)
    }

    /// Hash an API key using SHA-256 (fast for verification)
    fn hash_api_key(key: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Validate an API key against stored keys
    pub fn validate_api_key(storage: &StorageEngine, raw_key: &str) -> Result<Claims, DbError> {
        let db = storage.get_database(ADMIN_DB)?;
        let collection = db.get_collection(API_KEYS_COLL)?;

        // Hash the incoming key once
        let incoming_hash = Self::hash_api_key(raw_key);

        // Iterate through all keys and compare hashes (O(n) but fast with SHA-256)
        for doc in collection.scan(None) {
            let api_key: ApiKey = serde_json::from_value(doc.to_value())
                .map_err(|_| DbError::InternalError("Corrupted API key data".to_string()))?;

            // Constant-time comparison to prevent timing attacks
            if constant_time_eq(incoming_hash.as_bytes(), api_key.key_hash.as_bytes()) {
                // Check if API key has expired
                if let Some(ref expires_at) = api_key.expires_at {
                    if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(expires_at) {
                        if expiry < chrono::Utc::now() {
                            return Err(DbError::BadRequest("API key has expired".to_string()));
                        }
                    }
                }

                // Return claims with roles and scoped_databases from the API key
                return Ok(Claims {
                    sub: format!("api-key:{}", api_key.name),
                    exp: usize::MAX, // Claims never expire (API key expiry checked above)
                    livequery: None,
                    roles: if api_key.roles.is_empty() {
                        None
                    } else {
                        Some(api_key.roles)
                    },
                    scoped_databases: api_key.scoped_databases,
                });
            }
        }

        Err(DbError::BadRequest("Invalid API key".to_string()))
    }

    /// Get roles for a user from _user_roles collection
    pub fn get_user_roles(storage: &StorageEngine, username: &str) -> Option<Vec<String>> {
        let db = match storage.get_database(ADMIN_DB) {
            Ok(db) => db,
            Err(_) => return None,
        };

        let user_roles_coll = match db.get_collection(USER_ROLES_COLL) {
            Ok(coll) => coll,
            Err(_) => return None,
        };

        let mut roles = Vec::new();
        for doc in user_roles_coll.scan(None) {
            if let Ok(user_role) = serde_json::from_value::<UserRole>(doc.to_value()) {
                if user_role.username == username {
                    roles.push(user_role.role);
                }
            }
        }

        if roles.is_empty() {
            // TEMPORARY: If there's only one admin user and no roles assigned,
            // automatically grant admin role. This will be removed later.
            if let Ok(admins_coll) = db.get_collection(ADMIN_COLL) {
                if admins_coll.count() == 1 {
                    tracing::info!(
                        "Single admin user '{}' detected - auto-granting admin role",
                        username
                    );
                    return Some(vec!["admin".to_string()]);
                }
            }
            None
        } else {
            Some(roles)
        }
    }
}

/// Constant-time comparison to prevent timing attacks
pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b.iter()).fold(0, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Axum Middleware for Authentication
/// Supports both JWT (Authorization: Bearer <token>) and API keys (X-API-Key: <key>)
pub async fn auth_middleware(
    State(state): State<crate::server::handlers::AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Allow internal cluster shard forwarding without auth
    // SECURITY: Requires BOTH X-Shard-Direct/X-Scatter-Gather header AND valid X-Cluster-Secret
    // The secret must match the keyfile content configured at startup
    let is_internal_cluster_request = req.headers().contains_key("X-Shard-Direct")
        || req.headers().contains_key("X-Scatter-Gather");

    if is_internal_cluster_request {
        // Get cluster secret from keyfile via storage config
        let cluster_secret = state
            .storage
            .cluster_config()
            .and_then(|c| c.keyfile.clone())
            .unwrap_or_default();

        let provided_secret = req
            .headers()
            .get("X-Cluster-Secret")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");

        // Only bypass if secrets match and secret is not empty
        if !cluster_secret.is_empty()
            && constant_time_eq(cluster_secret.as_bytes(), provided_secret.as_bytes())
        {
            let claims = Claims {
                sub: "cluster-internal".to_string(),
                exp: usize::MAX,
                livequery: None,
                roles: Some(vec!["admin".to_string()]), // Cluster internal has admin access
                scoped_databases: None,
            };
            req.extensions_mut().insert(claims);
            return Ok(next.run(req).await);
        } else {
            tracing::warn!("CLUSTER AUTH FAILURE: Secret mismatch for internal request. Ensure all nodes use the same keyfile.");
        }
        // If secret doesn't match, fall through to normal auth
        // This prevents external attackers from using X-Shard-Direct to bypass auth
    }

    // First check for X-API-Key header
    if let Some(api_key) = req.headers().get("X-API-Key").and_then(|h| h.to_str().ok()) {
        match AuthService::validate_api_key(&state.storage, api_key) {
            Ok(claims) => {
                req.extensions_mut().insert(claims);
                return Ok(next.run(req).await);
            }
            Err(_) => return Err(StatusCode::UNAUTHORIZED),
        }
    }

    // Check for Authorization header
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok());

    if let Some(header) = auth_header {
        // Support: Authorization: ApiKey <key>
        if header.starts_with("ApiKey ") {
            let api_key = &header[7..];
            match AuthService::validate_api_key(&state.storage, api_key) {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    return Ok(next.run(req).await);
                }
                Err(_) => return Err(StatusCode::UNAUTHORIZED),
            }
        }

        // Support: Authorization: Bearer <jwt>
        if header.starts_with("Bearer ") {
            let token = &header[7..];
            match AuthService::validate_token(token) {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    return Ok(next.run(req).await);
                }
                Err(_) => return Err(StatusCode::UNAUTHORIZED),
            }
        }

        // Support: Authorization: Basic <base64(user:pass)>
        if header.starts_with("Basic ") {
            let encoded = &header[6..];
            if let Ok(decoded) =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            {
                if let Ok(credentials) = String::from_utf8(decoded) {
                    if let Some((username, password)) = credentials.split_once(':') {
                        // Validate against _admins collection
                        if let Ok(db) = state.storage.get_database("_system") {
                            if let Ok(collection) = db.get_collection("_admins") {
                                if let Ok(doc) = collection.get(username) {
                                    if let Ok(user) = serde_json::from_value::<User>(doc.to_value())
                                    {
                                        if AuthService::verify_password(
                                            password,
                                            &user.password_hash,
                                        ) {
                                            // Load user roles from _user_roles
                                            let roles = AuthService::get_user_roles(
                                                &state.storage,
                                                username,
                                            );
                                            let claims = Claims {
                                                sub: username.to_string(),
                                                exp: usize::MAX,
                                                livequery: None,
                                                roles,
                                                scoped_databases: None,
                                            };
                                            req.extensions_mut().insert(claims);
                                            return Ok(next.run(req).await);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    // Check for "token" query parameter
    if let Some(query) = req.uri().query() {
        if let Ok(params) = serde_urlencoded::from_str::<HashMap<String, String>>(query) {
            if let Some(token) = params.get("token") {
                if let Ok(claims) = AuthService::validate_token(token) {
                    req.extensions_mut().insert(claims);
                    return Ok(next.run(req).await);
                }
            }
        }
    }

    Err(StatusCode::UNAUTHORIZED)
}

/// Permissive auth middleware for custom scripts
/// Validates token if present, but allows anonymous access if missing
pub async fn permissive_auth_middleware(
    State(state): State<crate::server::handlers::AppState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // First check for X-API-Key header
    if let Some(api_key) = req.headers().get("X-API-Key").and_then(|h| h.to_str().ok()) {
        // If API key is present, it MUST be valid
        match AuthService::validate_api_key(&state.storage, api_key) {
            Ok(claims) => {
                req.extensions_mut().insert(claims);
                return Ok(next.run(req).await);
            }
            Err(_) => return Err(StatusCode::UNAUTHORIZED),
        }
    }

    // Check for Authorization header
    let auth_header = req
        .headers()
        .get("Authorization")
        .and_then(|h| h.to_str().ok());

    if let Some(header) = auth_header {
        // If Authorization header is present, it MUST be valid

        // Support: Authorization: ApiKey <key>
        if header.starts_with("ApiKey ") {
            let api_key = &header[7..];
            match AuthService::validate_api_key(&state.storage, api_key) {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    return Ok(next.run(req).await);
                }
                Err(_) => return Err(StatusCode::UNAUTHORIZED),
            }
        }

        // Support: Authorization: Bearer <jwt>
        if header.starts_with("Bearer ") {
            let token = &header[7..];
            match AuthService::validate_token(token) {
                Ok(claims) => {
                    req.extensions_mut().insert(claims);
                    return Ok(next.run(req).await);
                }
                Err(_) => return Err(StatusCode::UNAUTHORIZED),
            }
        }

        // Support: Authorization: Basic <base64(user:pass)>
        if header.starts_with("Basic ") {
            let encoded = &header[6..];
            if let Ok(decoded) =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            {
                if let Ok(credentials) = String::from_utf8(decoded) {
                    if let Some((username, password)) = credentials.split_once(':') {
                        // Validate against _admins collection
                        if let Ok(db) = state.storage.get_database("_system") {
                            if let Ok(collection) = db.get_collection("_admins") {
                                if let Ok(doc) = collection.get(username) {
                                    if let Ok(user) = serde_json::from_value::<User>(doc.to_value())
                                    {
                                        if AuthService::verify_password(
                                            password,
                                            &user.password_hash,
                                        ) {
                                            // Load user roles from _user_roles
                                            let roles = AuthService::get_user_roles(
                                                &state.storage,
                                                username,
                                            );
                                            let claims = Claims {
                                                sub: username.to_string(),
                                                exp: usize::MAX,
                                                livequery: None,
                                                roles,
                                                scoped_databases: None,
                                            };
                                            req.extensions_mut().insert(claims);
                                            return Ok(next.run(req).await);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            return Err(StatusCode::UNAUTHORIZED);
        }
    }

    // No auth header present - proceed as anonymous (no claims injected)
    Ok(next.run(req).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify_password() {
        let password = "test_password_123";
        let hash = AuthService::hash_password(password).unwrap();

        assert!(!hash.is_empty());
        assert!(AuthService::verify_password(password, &hash));
        assert!(!AuthService::verify_password("wrong_password", &hash));
    }

    #[test]
    fn test_verify_password_invalid_hash() {
        assert!(!AuthService::verify_password("password", "invalid_hash"));
    }

    #[test]
    fn test_create_and_validate_jwt() {
        let token = AuthService::create_jwt("testuser").unwrap();

        assert!(!token.is_empty());

        let claims = AuthService::validate_token(&token).unwrap();
        assert_eq!(claims.sub, "testuser");
        assert!(claims.exp > 0);
        assert!(claims.livequery.is_none());
    }

    #[test]
    fn test_validate_invalid_token() {
        let result = AuthService::validate_token("invalid.token.here");
        assert!(result.is_err());
    }

    #[test]
    fn test_create_livequery_jwt() {
        let token = AuthService::create_livequery_jwt().unwrap();

        let claims = AuthService::validate_token(&token).unwrap();
        assert_eq!(claims.sub, "livequery");
        assert_eq!(claims.livequery, Some(true));
    }

    #[test]
    fn test_generate_api_key() {
        let (raw_key, hash) = AuthService::generate_api_key();

        // Key should start with sk_
        assert!(raw_key.starts_with("sk_"));

        // Key should be 67 characters (sk_ + 64 hex chars)
        assert_eq!(raw_key.len(), 67);

        // Hash should be 64 characters (SHA-256 hex)
        assert_eq!(hash.len(), 64);

        // Hashing same key should produce same hash
        let hash2 = AuthService::hash_api_key(&raw_key);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_api_key_uniqueness() {
        let (key1, _) = AuthService::generate_api_key();
        let (key2, _) = AuthService::generate_api_key();

        assert_ne!(key1, key2);
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"test", b"test"));
        assert!(!constant_time_eq(b"test", b"Test"));
        assert!(!constant_time_eq(b"test", b"testing"));
        assert!(!constant_time_eq(b"short", b"longer_string"));
    }

    #[test]
    fn test_claims_struct() {
        let claims = Claims {
            sub: "user1".to_string(),
            exp: 12345,
            livequery: Some(true),
            roles: Some(vec!["admin".to_string()]),
            scoped_databases: None,
        };

        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.exp, 12345);
        assert_eq!(claims.livequery, Some(true));
        assert_eq!(claims.roles, Some(vec!["admin".to_string()]));
        assert_eq!(claims.scoped_databases, None);
    }

    #[test]
    fn test_user_struct() {
        let user = User {
            username: "admin".to_string(),
            password_hash: "hash123".to_string(),
        };

        assert_eq!(user.username, "admin");
        assert_eq!(user.password_hash, "hash123");
    }

    #[test]
    fn test_api_key_struct() {
        let api_key = ApiKey {
            id: "key1".to_string(),
            name: "My Key".to_string(),
            key_hash: "hash123".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            roles: vec!["admin".to_string()],
            scoped_databases: Some(vec!["db1".to_string()]),
            expires_at: None,
        };

        assert_eq!(api_key.id, "key1");
        assert_eq!(api_key.name, "My Key");
        assert_eq!(api_key.roles, vec!["admin".to_string()]);
        assert_eq!(api_key.scoped_databases, Some(vec!["db1".to_string()]));
    }

    #[test]
    fn test_claims_serialization() {
        let claims = Claims {
            sub: "user".to_string(),
            exp: 1000,
            livequery: None,
            roles: None,
            scoped_databases: None,
        };

        let json = serde_json::to_string(&claims).unwrap();
        assert!(json.contains("user"));
        assert!(json.contains("1000"));
        // Optional fields should be skipped when None
        assert!(!json.contains("livequery"));
        assert!(!json.contains("roles"));
        assert!(!json.contains("scoped_databases"));

        let deserialized: Claims = serde_json::from_str(&json).unwrap();
        assert_eq!(claims.sub, deserialized.sub);

        // Test with roles
        let claims_with_roles = Claims {
            sub: "user".to_string(),
            exp: 1000,
            livequery: None,
            roles: Some(vec!["admin".to_string(), "editor".to_string()]),
            scoped_databases: Some(vec!["db1".to_string()]),
        };

        let json = serde_json::to_string(&claims_with_roles).unwrap();
        assert!(json.contains("roles"));
        assert!(json.contains("admin"));
        assert!(json.contains("scoped_databases"));
    }

    #[test]
    fn test_check_rate_limit_initial() {
        // First call should succeed (using unique IP)
        let result = check_rate_limit("192.168.1.1_test");
        assert!(result.is_ok());
    }

    #[test]
    fn test_password_hash_different_each_time() {
        let password = "same_password";
        let hash1 = AuthService::hash_password(password).unwrap();
        let hash2 = AuthService::hash_password(password).unwrap();

        // Hashes should be different due to random salt
        assert_ne!(hash1, hash2);

        // But both should verify correctly
        assert!(AuthService::verify_password(password, &hash1));
        assert!(AuthService::verify_password(password, &hash2));
    }
}
