use crate::error::DbError;
use crate::storage::StorageEngine;

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
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use once_cell::sync::Lazy;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const ADMIN_DB: &str = "_system";
const ADMIN_COLL: &str = "_admins";
pub const API_KEYS_COLL: &str = "_api_keys";
const DEFAULT_USER: &str = "admin";
const DEFAULT_PASS: &str = "admin";

// Secret for JWT signing - in production this should come from env
static JWT_SECRET: Lazy<String> = Lazy::new(|| {
    std::env::var("JWT_SECRET").unwrap_or_else(|_| "solisoft-secret-key-change-me".to_string())
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
    pub fn init(storage: &StorageEngine) -> Result<(), DbError> {
        let db = storage.get_database(ADMIN_DB)?;
        
        // Ensure _admins collection exists
        if let Err(DbError::CollectionNotFound(_)) = db.get_collection(ADMIN_COLL) {
            tracing::info!("Creating {} collection", ADMIN_COLL);
            db.create_collection(ADMIN_COLL.to_string())?;
        }
        
        // Ensure _api_keys collection exists
        if let Err(DbError::CollectionNotFound(_)) = db.get_collection(API_KEYS_COLL) {
            tracing::info!("Creating {} collection", API_KEYS_COLL);
            db.create_collection(API_KEYS_COLL.to_string())?;
        }
        
        // Check if any admin exists
        let collection = db.get_collection(ADMIN_COLL)?;
        if collection.count() == 0 {
            tracing::warn!("No admin users found. Creating default admin user.");
            
            let salt = SaltString::generate(&mut OsRng);
            let argon2 = Argon2::default();
            let password_hash = argon2
                .hash_password(DEFAULT_PASS.as_bytes(), &salt)
                .map_err(|e| DbError::InternalError(format!("Hashing error: {}", e)))?
                .to_string();

            let user = User {
                username: DEFAULT_USER.to_string(),
                password_hash,
            };

            let doc_value = serde_json::to_value(user)
                .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;

            collection.insert(doc_value)?;
            
            tracing::warn!("Default admin created. Username: '{}', Password: '{}'", DEFAULT_USER, DEFAULT_PASS);
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
    pub fn validate_token(token: &str) -> Result<Claims, jsonwebtoken::errors::Error> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(JWT_SECRET.as_bytes()),
            &Validation::default(),
        )?;
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
    }

    Err(StatusCode::UNAUTHORIZED)
}
