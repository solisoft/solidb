use super::system::AppState;
use crate::error::DbError;
use crate::sync::{LogEntry, Operation};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Json,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct AuthParams {
    pub token: String,
    pub htmx: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Debug, Serialize)]
pub struct ChangePasswordResponse {
    pub status: String,
}

// ==================== API Key Types ====================

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    #[serde(default)]
    pub roles: Vec<String>,
    pub scoped_databases: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key: String, // Raw key - only returned on creation!
    pub created_at: String,
    pub roles: Vec<String>,
    pub scoped_databases: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct ListApiKeysResponse {
    pub keys: Vec<crate::server::auth::ApiKeyListItem>,
}

#[derive(Debug, Serialize)]
pub struct DeleteApiKeyResponse {
    pub deleted: bool,
}

/// Handler for changing the current user's password
pub async fn change_password_handler(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<crate::server::auth::Claims>,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<ChangePasswordResponse>, DbError> {
    // 1. Get _system database
    let db = state.storage.get_database("_system")?;

    // 2. Get _admins collection
    let collection = db.get_collection("_admins")?;

    // 3. Get current user document
    let doc = match collection.get(&claims.sub) {
        Ok(d) => d,
        Err(DbError::DocumentNotFound(_)) => {
            return Err(DbError::BadRequest("User not found".to_string()));
        }
        Err(e) => return Err(e),
    };

    // 4. Deserialize user
    let user: crate::server::auth::User = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted user data".to_string()))?;

    // 5. Verify current password
    if !crate::server::auth::AuthService::verify_password(
        &req.current_password,
        &user.password_hash,
    ) {
        return Err(DbError::BadRequest(
            "Current password is incorrect".to_string(),
        ));
    }

    // 6. Hash new password
    let new_hash = crate::server::auth::AuthService::hash_password(&req.new_password)?;

    // 7. Update user document
    let updated_user = crate::server::auth::User {
        username: user.username.clone(),
        password_hash: new_hash,
    };

    let updated_value = serde_json::to_value(&updated_user)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;

    collection.update(&claims.sub, updated_value.clone())?;

    // Record write for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,             // Auto
            node_id: "".to_string(), // Auto
            database: "_system".to_string(),
            collection: "_admins".to_string(),
            operation: Operation::Update,
            key: claims.sub.clone(),
            data: serde_json::to_vec(&updated_value).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    Ok(Json(ChangePasswordResponse {
        status: "password_updated".to_string(),
    }))
}

/// Handler for creating a new API key
pub async fn create_api_key_handler(
    State(state): State<AppState>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<Json<CreateApiKeyResponse>, DbError> {
    // Generate key
    let (raw_key, key_hash) = crate::server::auth::AuthService::generate_api_key();

    // Create unique ID
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    // Store in _system/_api_keys
    let db = state.storage.get_database("_system")?;

    // Ensure collection exists
    if let Err(DbError::CollectionNotFound(_)) =
        db.get_collection(crate::server::auth::API_KEYS_COLL)
    {
        db.create_collection(crate::server::auth::API_KEYS_COLL.to_string(), None)?;
    }

    let collection = db.get_collection(crate::server::auth::API_KEYS_COLL)?;

    // Use provided roles, default to admin if empty
    let roles = if req.roles.is_empty() {
        vec!["admin".to_string()]
    } else {
        req.roles.clone()
    };

    let api_key = crate::server::auth::ApiKey {
        id: id.clone(),
        name: req.name.clone(),
        key_hash,
        created_at: created_at.clone(),
        roles: roles.clone(),
        scoped_databases: req.scoped_databases.clone(),
        expires_at: None,
    };

    let doc_value = serde_json::to_value(&api_key)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;

    collection.insert(doc_value.clone())?;

    // Record write for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: "_system".to_string(),
            collection: crate::server::auth::API_KEYS_COLL.to_string(),
            operation: Operation::Insert,
            key: id.clone(),
            data: serde_json::to_vec(&doc_value).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    tracing::info!("API key '{}' created", req.name);

    // Return response with the raw key (only time it's shown!)
    Ok(Json(CreateApiKeyResponse {
        id,
        name: req.name,
        key: raw_key,
        created_at,
        roles,
        scoped_databases: req.scoped_databases,
    }))
}

/// Handler for listing API keys (without the actual keys)
pub async fn list_api_keys_handler(
    State(state): State<AppState>,
) -> Result<Json<ListApiKeysResponse>, DbError> {
    let db = state.storage.get_database("_system")?;

    // Return empty if collection doesn't exist
    let collection = match db.get_collection(crate::server::auth::API_KEYS_COLL) {
        Ok(c) => c,
        Err(DbError::CollectionNotFound(_)) => {
            return Ok(Json(ListApiKeysResponse { keys: vec![] }));
        }
        Err(e) => return Err(e),
    };

    let mut keys = Vec::new();
    for doc in collection.scan(None) {
        let api_key: crate::server::auth::ApiKey = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted API key data".to_string()))?;

        keys.push(crate::server::auth::ApiKeyListItem {
            id: api_key.id,
            name: api_key.name,
            created_at: api_key.created_at,
            roles: api_key.roles,
            scoped_databases: api_key.scoped_databases,
        });
    }

    Ok(Json(ListApiKeysResponse { keys }))
}

/// Handler for deleting an API key
pub async fn delete_api_key_handler(
    State(state): State<AppState>,
    Path(key_id): Path<String>,
) -> Result<Json<DeleteApiKeyResponse>, DbError> {
    let db = state.storage.get_database("_system")?;
    let collection = db.get_collection(crate::server::auth::API_KEYS_COLL)?;

    collection.delete(&key_id)?;

    // Record write for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: "_system".to_string(),
            collection: crate::server::auth::API_KEYS_COLL.to_string(),
            operation: Operation::Delete,
            key: key_id.clone(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    tracing::info!("API key '{}' deleted", key_id);

    Ok(Json(DeleteApiKeyResponse { deleted: true }))
}

pub async fn login_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, DbError> {
    // Extract client IP for rate limiting (check X-Forwarded-For first for proxied requests)
    let client_ip = headers
        .get("X-Forwarded-For")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            headers
                .get("X-Real-IP")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());

    // Check rate limit before processing
    crate::server::auth::check_rate_limit(&client_ip)?;
    // 1. Get _system database
    let db = state.storage.get_database("_system")?;

    // 2. Get _admins collection (create with default admin if missing)
    let collection = match db.get_collection("_admins") {
        Ok(c) => c,
        Err(DbError::CollectionNotFound(_)) => {
            // Collection doesn't exist - initialize auth (creates collection and default admin)
            tracing::warn!("_admins collection not found, initializing...");
            crate::server::auth::AuthService::init(
                &state.storage,
                state.replication_log.as_deref(),
            )?;
            db.get_collection("_admins")?
        }
        Err(e) => return Err(e),
    };

    // 3. Check if collection is empty (also create default admin)
    if collection.count() == 0 {
        tracing::warn!("_admins collection empty, creating default admin...");
        crate::server::auth::AuthService::init(&state.storage, state.replication_log.as_deref())?;
    }

    // 4. Get user document
    // We expect the username to be the _key
    let doc = match collection.get(&req.username) {
        Ok(d) => d,
        Err(DbError::DocumentNotFound(_)) => {
            // Return generic error for security
            return Err(DbError::BadRequest("Invalid credentials".to_string()));
        }
        Err(e) => return Err(e),
    };

    // 5. Deserialize user
    let user: crate::server::auth::User = serde_json::from_value(doc.to_value()).map_err(|e| {
        tracing::error!("Failed to deserialize user '{}': {}", req.username, e);
        DbError::InternalError("Corrupted user data".to_string())
    })?;

    // 6. Verify password
    if !crate::server::auth::AuthService::verify_password(&req.password, &user.password_hash) {
        tracing::warn!("Password verification failed for user '{}'", req.username);
        return Err(DbError::BadRequest("Invalid credentials".to_string()));
    }

    // 7. Generate Token
    let token = crate::server::auth::AuthService::create_jwt(&user.username)?;

    Ok(Json(LoginResponse { token }))
}

/// Response for livequery token endpoint
#[derive(Debug, Serialize)]
pub struct LiveQueryTokenResponse {
    pub token: String,
    pub expires_in: u32, // seconds until expiration
}

/// Generate a short-lived JWT token for live query WebSocket connections.
/// This endpoint requires authentication (regular JWT or API key).
/// The returned token is valid for 30 seconds - just enough to establish a WebSocket connection.
/// This allows clients to connect to live queries without exposing long-lived admin tokens.
pub async fn livequery_token_handler() -> Result<Json<LiveQueryTokenResponse>, DbError> {
    let token = crate::server::auth::AuthService::create_livequery_jwt()?;
    Ok(Json(LiveQueryTokenResponse {
        token,
        expires_in: 2,
    }))
}
