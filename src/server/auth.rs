use crate::error::DbError;
use crate::storage::StorageEngine;
use crate::sync::log::SyncLog;
use crate::sync::{LogEntry, Operation};

use argon2::{
    password_hash::{
        PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
    },
    Argon2,
};
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use jsonwebtoken::{encode, decode, Header, Validation, EncodingKey, DecodingKey, Algorithm};


use once_cell::sync::Lazy;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Rate limiting configuration
const MAX_LOGIN_ATTEMPTS: usize = 5;
const RATE_LIMIT_WINDOW_SECS: u64 = 60;

/// In-memory rate limiter for login attempts
/// Tracks attempts per IP address with automatic cleanup
static LOGIN_RATE_LIMITER: Lazy<RwLock<HashMap<String, Vec<Instant>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Check if an IP is rate limited, return error if too many attempts
pub fn check_rate_limit(ip: &str) -> Result<(), crate::error::DbError> {
    let now = Instant::now();
    let window = std::time::Duration::from_secs(RATE_LIMIT_WINDOW_SECS);

    let mut limiter = LOGIN_RATE_LIMITER.write().unwrap_or_else(|e| e.into_inner());

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
const ADMIN_COLL: &str = "_admins";
pub const API_KEYS_COLL: &str = "_api_keys";
const DEFAULT_USER: &str = "admin";

// Secret for JWT signing - MUST be set via JWT_SECRET env var in production
static JWT_SECRET: Lazy<String> = Lazy::new(|| {
    match std::env::var("JWT_SECRET") {
        Ok(secret) => {
            if secret.len() < 32 {
                tracing::warn!("⚠️  JWT_SECRET is less than 32 characters - consider using a longer secret");
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
            tracing::warn!("║    export JWT_SECRET=\"your-secure-random-secret-here\"            ║");
            tracing::warn!("╚══════════════════════════════════════════════════════════════════╝");
            generated
        }
    }
});

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String, // username
    pub exp: usize,  // expiration
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
}

#[derive(Debug, Serialize)]
pub struct ApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key: String,  // Only returned on creation
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyListItem {
    pub id: String,
    pub name: String,
    pub created_at: String,
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
        let is_joining_cluster = storage.cluster_config()
            .map(|c| !c.peers.is_empty())
            .unwrap_or(false);

        let has_override_password = std::env::var("SOLIDB_ADMIN_PASSWORD")
            .map(|p| !p.is_empty())
            .unwrap_or(false);

        let should_skip_defaults = is_joining_cluster && !has_override_password;

        // Ensure _admins collection exists
        if let Err(DbError::CollectionNotFound(_)) = db.get_collection(ADMIN_COLL) {
            if should_skip_defaults {
                tracing::info!("Cluster join detected: Skipping {} creation (waiting for sync)", ADMIN_COLL);
            } else {
                tracing::info!("Creating {} collection", ADMIN_COLL);
                db.create_collection(ADMIN_COLL.to_string(), None)?;
            }
        }

        // Ensure _api_keys collection exists
        if let Err(DbError::CollectionNotFound(_)) = db.get_collection(API_KEYS_COLL) {
            if should_skip_defaults {
                tracing::info!("Cluster join detected: Skipping {} creation (waiting for sync)", API_KEYS_COLL);
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

                    let doc_value = serde_json::to_value(user)
                        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;

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
                        tracing::warn!("Admin user created with password from SOLIDB_ADMIN_PASSWORD env var");
                    } else {
                        tracing::warn!("╔══════════════════════════════════════════════════════════════════╗");
                        tracing::warn!("║              INITIAL ADMIN ACCOUNT CREATED                       ║");
                        tracing::warn!("╠══════════════════════════════════════════════════════════════════╣");
                        tracing::warn!("║  Username: admin                                                 ║");
                        tracing::warn!("║  Password: {}                             ║", password);
                        tracing::warn!("║                                                                  ║");
                        tracing::warn!("║  ⚠️  SAVE THIS PASSWORD! It will not be shown again.             ║");
                        tracing::warn!("║  Change it after first login via the API.                        ║");
                        tracing::warn!("╚══════════════════════════════════════════════════════════════════╝");
                    }
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
        let expiration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as usize
            + 24 * 3600; // 24 hours

        let claims = Claims {
            sub: username.to_owned(),
            exp: expiration,
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
        ).map_err(|_| DbError::BadRequest("Invalid token".to_string()))?;

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
        use sha2::{Sha256, Digest};
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
                // Return synthetic claims for the API key
                return Ok(Claims {
                    sub: format!("api-key:{}", api_key.name),
                    exp: usize::MAX, // Never expires
                });
            }
        }

        Err(DbError::BadRequest("Invalid API key".to_string()))
    }
}

/// Constant-time comparison to prevent timing attacks
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
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
    // The secret must match SOLIDB_CLUSTER_SECRET env var (generated at startup if not set)
    let is_internal_cluster_request = req.headers().contains_key("X-Shard-Direct")
        || req.headers().contains_key("X-Scatter-Gather");

    if is_internal_cluster_request {
        let cluster_secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
        let provided_secret = req.headers()
            .get("X-Cluster-Secret")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");

        // Only bypass if secrets match and secret is not empty
        if !cluster_secret.is_empty() && constant_time_eq(cluster_secret.as_bytes(), provided_secret.as_bytes()) {
            let claims = Claims {
                sub: "cluster-internal".to_string(),
                exp: usize::MAX,
            };
            req.extensions_mut().insert(claims);
            return Ok(next.run(req).await);
        } else {
            tracing::warn!("CLUSTER AUTH FAILURE: Secret mismatch for internal request. Check SOLIDB_CLUSTER_SECRET env var on all nodes.");
        }
        // If secret doesn't match, fall through to normal auth
        // This prevents external attackers from using X-Shard-Direct to bypass auth
    }


    // First check for X-API-Key header
    if let Some(api_key) = req.headers()
        .get("X-API-Key")
        .and_then(|h| h.to_str().ok())
    {
        match AuthService::validate_api_key(&state.storage, api_key) {
            Ok(claims) => {
                req.extensions_mut().insert(claims);
                return Ok(next.run(req).await);
            }
            Err(_) => return Err(StatusCode::UNAUTHORIZED),
        }
    }

    // Check for Authorization header
    let auth_header = req.headers()
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
            if let Ok(decoded) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded) {
                if let Ok(credentials) = String::from_utf8(decoded) {
                    if let Some((username, password)) = credentials.split_once(':') {
                        // Validate against _admins collection
                        if let Ok(db) = state.storage.get_database("_system") {
                            if let Ok(collection) = db.get_collection("_admins") {
                                if let Ok(doc) = collection.get(username) {
                                    if let Ok(user) = serde_json::from_value::<User>(doc.to_value()) {
                                        if AuthService::verify_password(password, &user.password_hash) {
                                            let claims = Claims {
                                                sub: username.to_string(),
                                                exp: usize::MAX,
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

    Err(StatusCode::UNAUTHORIZED)
}
