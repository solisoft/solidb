use axum::{
    extract::{Multipart, Path, Query as AxumQuery, State, ws::{Message, WebSocket, WebSocketUpgrade}},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    body::Body,
};

#[derive(Debug, Deserialize)]
pub struct AuthParams {
    pub token: String,
}
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::cluster::stats::NodeBasicStats;
use std::sync::Arc;
use base64::{Engine as _, engine::general_purpose};

use crate::sdbql::{parse, BodyClause, Query, QueryExecutor};
use crate::sync::{Operation, LogEntry};
use crate::sync::blob_replication::replicate_blob_to_node;
use crate::error::DbError;
use crate::scripting::ScriptStats;
use crate::server::response::ApiResponse;


/// Default query execution timeout (30 seconds)
const QUERY_TIMEOUT_SECS: u64 = 30;

/// Sanitize a filename for use in Content-Disposition header to prevent header injection
/// Removes/replaces: quotes, backslashes, newlines, carriage returns, and non-ASCII characters
fn sanitize_filename(filename: &str) -> String {
    filename
        .chars()
        .filter(|c| c.is_ascii() && *c != '"' && *c != '\\' && *c != '\n' && *c != '\r')
        .collect::<String>()
        .trim()
        .to_string()
}
use crate::server::cursor_store::CursorStore;
use crate::storage::{GeoIndexStats, IndexStats, IndexType, StorageEngine, TtlIndexStats};
use crate::transaction::TransactionId;
use std::collections::HashMap;

/// Check if a query is potentially long-running (contains mutations or range iterations)
#[inline]
fn is_long_running_query(query: &Query) -> bool {
    query.body_clauses.iter().any(|clause| match clause {
        BodyClause::Insert(_) | BodyClause::Update(_) | BodyClause::Remove(_) => true,
        // All FOR loops should use spawn_blocking because:
        // 1. Range expressions (source_expression.is_some()) can be large
        // 2. Collection scans might trigger scatter-gather with blocking HTTP calls
        BodyClause::For(_) => true,
        _ => false,
    })
}

/// Protected system collections that cannot be deleted or modified via standard API
const PROTECTED_COLLECTIONS: [&str; 2] = ["_admins", "_api_keys"];

/// Check if a collection is a protected system collection
#[inline]
fn is_protected_collection(db_name: &str, coll_name: &str) -> bool {
    db_name == "_system" && PROTECTED_COLLECTIONS.contains(&coll_name)
}

/// Check if a collection is a physical shard (ends with _sN where N is a number)
/// Physical shards are implementation details and should be hidden from users
#[inline]
fn is_physical_shard_collection(name: &str) -> bool {
    if let Some(pos) = name.rfind("_s") {
        let suffix = &name[pos + 2..];
        !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<StorageEngine>,
    pub cursor_store: CursorStore,
    // New Architecture Components
    pub cluster_manager: Option<Arc<crate::cluster::manager::ClusterManager>>,
    pub replication_log: Option<Arc<crate::sync::log::SyncLog>>,
    pub shard_coordinator: Option<Arc<crate::sharding::ShardCoordinator>>,
    pub startup_time: std::time::Instant,
    pub request_counter: Arc<std::sync::atomic::AtomicU64>,
    pub system_monitor: Arc<std::sync::Mutex<sysinfo::System>>,
    pub queue_worker: Option<Arc<crate::queue::QueueWorker>>,
    pub script_stats: Arc<ScriptStats>,
}

// ==================== Auth Types ====================

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
    if !crate::server::auth::AuthService::verify_password(&req.current_password, &user.password_hash) {
        return Err(DbError::BadRequest("Current password is incorrect".to_string()));
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
            sequence: 0, // Auto
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

// ==================== API Key Types ====================

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key: String,  // Raw key - only returned on creation!
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ListApiKeysResponse {
    pub keys: Vec<crate::server::auth::ApiKeyListItem>,
}

#[derive(Debug, Serialize)]
pub struct DeleteApiKeyResponse {
    pub deleted: bool,
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
    if let Err(DbError::CollectionNotFound(_)) = db.get_collection(crate::server::auth::API_KEYS_COLL) {
        db.create_collection(crate::server::auth::API_KEYS_COLL.to_string(), None)?;
    }

    let collection = db.get_collection(crate::server::auth::API_KEYS_COLL)?;

    let api_key = crate::server::auth::ApiKey {
        id: id.clone(),
        name: req.name.clone(),
        key_hash,
        created_at: created_at.clone(),
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

// ==================== Blob Handlers ====================

pub async fn upload_blob(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    multipart_result: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Json<Value>, DbError> {
    let mut multipart = multipart_result.map_err(|e| DbError::BadRequest(e.to_string()))?;
    let database = state.storage.get_database(&db_name)?;

    // Try to get the collection, auto-create as blob collection if it doesn't exist
    let collection = match database.get_collection(&coll_name) {
        Ok(coll) => {
            // Collection exists - check if it's a blob collection
            if coll.get_type() != "blob" {
                return Err(DbError::BadRequest(format!("Collection '{}' is not a blob collection. Please create it as a blob collection first.", coll_name)));
            }
            coll
        },
        Err(DbError::CollectionNotFound(_)) => {
            // Auto-create blob collection
            tracing::info!("Auto-creating blob collection {}/{}", db_name, coll_name);
            database.create_collection(coll_name.clone(), Some("blob".to_string()))?;
            database.get_collection(&coll_name)?
        },
        Err(e) => return Err(e),
    };

    let mut file_name = None;
    let mut mime_type = None;
    let mut total_size = 0usize;
    let mut chunk_count = 0u32;
    // Generate a temporary key or use one if we support PUT (for now auto-generate)
    let blob_key = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
    tracing::info!("Starting upload_blob for {}/{} with key {}", db_name, coll_name, blob_key);

    let mut chunks_buffer: Vec<(u32, Vec<u8>)> = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|e| DbError::BadRequest(e.to_string()))? {
        if let Some(name) = field.name() {
            tracing::info!("Processing field: {}", name);
            if name == "file" {
                if let Some(fname) = field.file_name() {
                    file_name = Some(fname.to_string());
                }
                if let Some(mtype) = field.content_type() {
                    mime_type = Some(mtype.to_string());
                }

                let mut stream = field;
                while let Some(chunk_res) = stream.next().await {
                    let chunk = chunk_res.map_err(|e| {
                        tracing::error!("Chunk error: {}", e);
                        DbError::BadRequest(e.to_string())
                    })?;
                    let data = chunk.to_vec();
                    let len = data.len();
                    tracing::debug!("Received chunk size: {}", len);

                    if len > 0 {
                        chunks_buffer.push((chunk_count, data));
                        total_size += len;
                        chunk_count += 1;
                    }
                }
                tracing::info!("Buffered file. Total size: {}, chunks: {}", total_size, chunks_buffer.len());
            }
        }
    }

    // Create metadata document
    let mut metadata = serde_json::Map::new();
    metadata.insert("_key".to_string(), Value::String(blob_key.clone()));
    if let Some(fn_str) = file_name {
        metadata.insert("name".to_string(), Value::String(fn_str));
    }
    if let Some(mt_str) = mime_type {
        metadata.insert("type".to_string(), Value::String(mt_str));
    }
    metadata.insert("size".to_string(), Value::Number(total_size.into()));
    metadata.insert("chunks".to_string(), Value::Number(chunk_count.into()));
    metadata.insert("created".to_string(), Value::String(chrono::Utc::now().to_rfc3339()));
    let doc_value = Value::Object(metadata);

    // Check for sharding
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                tracing::info!("[BLOB_UPLOAD] Using ShardCoordinator for {}/{}", db_name, coll_name);
                let doc = coordinator.upload_blob(
                    &db_name,
                    &coll_name,
                    &shard_config,
                    doc_value,
                    chunks_buffer,
                ).await?;
                return Ok(Json(doc));
            } else {
                return Err(DbError::InternalError("Sharded blob collection requires ShardCoordinator".to_string()));
            }
        }
    }

    // Only reach here for non-sharded collections
    // For blob collections, distribute chunks across the cluster for fault tolerance
    if collection.get_type() == "blob" {
        if let Some(ref _coordinator) = state.shard_coordinator {
            // Distribute blob chunks across available nodes
            tracing::info!("Distributing {} blob chunks for {}/{} across cluster", chunks_buffer.len(), db_name, coll_name);
            distribute_blob_chunks_across_cluster(
                state.shard_coordinator.as_ref().unwrap(),
                &db_name,
                &coll_name,
                &blob_key,
                &chunks_buffer,
                &doc_value,
                &state.storage,
            ).await?;
        } else {
            // No coordinator available, store locally as fallback
            tracing::warn!("No cluster coordinator available, storing blob chunks locally");
            for (idx, data) in &chunks_buffer {
                collection.put_blob_chunk(&blob_key, *idx, data)?;
            }
            collection.insert(doc_value.clone())?;
        }
    } else {
        // Regular document collection - store locally
        for (idx, data) in &chunks_buffer {
            collection.put_blob_chunk(&blob_key, *idx, data)?;
        }
        collection.insert(doc_value.clone())?;
    }

    // Log operation for replication (if enabled for other collections, keep logging for consistency)
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: db_name.clone(),
            collection: coll_name.clone(),
            operation: Operation::Insert,
            key: blob_key.clone(),
            data: serde_json::to_vec(&doc_value).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    Ok(Json(doc_value))
}

pub async fn download_blob(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
) -> Result<Response, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    if collection.get_type() != "blob" {
        return Err(DbError::BadRequest(format!("Collection '{}' is not a blob collection.", coll_name)));
    }

    // Check for sharding
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                tracing::info!("[BLOB_DOWNLOAD] Using ShardCoordinator for {}/{}", db_name, coll_name);
                return coordinator.download_blob(&db_name, &coll_name, &shard_config, &key).await;
            } else {
                return Err(DbError::InternalError("Sharded blob collection requires ShardCoordinator".to_string()));
            }
        }
    }

    // Only reach here for non-sharded collections
    // For blob collections, chunks may be distributed across the cluster
    // First check if metadata exists locally
    if collection.get(&key).is_err() {
        return Err(DbError::DocumentNotFound(format!("Blob not found: {}", key)));
    }

    let content_type = if let Ok(doc) = collection.get(&key) {
        if let Some(v) = doc.get("type") {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                "application/octet-stream".to_string()
            }
        } else {
            "application/octet-stream".to_string()
        }
    } else {
        "application/octet-stream".to_string()
    };

    let file_name = if let Ok(doc) = collection.get(&key) {
        if let Some(v) = doc.get("name") {
            if let Some(s) = v.as_str() {
                Some(s.to_string())
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let stream = async_stream::stream! {
        let mut chunk_idx = 0;
        loop {
            match collection.get_blob_chunk(&key, chunk_idx) {
                Ok(Some(data)) => {
                    yield Ok::<_, std::io::Error>(axum::body::Bytes::from(data));
                    chunk_idx += 1;
                }
                Ok(None) => break, // End of chunks
                Err(_) => {
                    // For blob collections, try to fetch chunk from other nodes
                    if collection.get_type() == "blob" {
                        if let Some(ref coordinator) = state.shard_coordinator {
                            match fetch_blob_chunk_from_cluster(coordinator, &db_name, &coll_name, &key, chunk_idx).await {
                                Ok(Some(data)) => {
                                    yield Ok::<_, std::io::Error>(axum::body::Bytes::from(data));
                                    chunk_idx += 1;
                                    continue;
                                }
                                Ok(None) => break, // No more chunks
                                Err(e) => {
                                    yield Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                                    break;
                                }
                            }
                        } else {
                            yield Err(std::io::Error::new(std::io::ErrorKind::Other, "Blob chunk not found locally and no cluster available".to_string()));
                            break;
                        }
                    } else {
                        yield Err(std::io::Error::new(std::io::ErrorKind::Other, "Blob chunk not found".to_string()));
                        break;
                    }
                }
            }
        }
    };

    let body = axum::body::Body::from_stream(stream);

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_str(&content_type).unwrap()
    );

    if let Some(name) = file_name {
        let disposition = format!("attachment; filename=\"{}\"", name);
         if let Ok(val) = axum::http::HeaderValue::from_str(&disposition) {
             headers.insert(axum::http::header::CONTENT_DISPOSITION, val);
         }
    }

    Ok((headers, body).into_response())
}

// ==================== Request/Response Types ====================

#[derive(Debug, Deserialize)]
pub struct CreateDatabaseRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateDatabaseResponse {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ListDatabasesResponse {
    pub databases: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCollectionRequest {
    pub name: String,
    /// Collection type: "document" (default), "edge", or "blob"
    #[serde(rename = "type")]
    pub collection_type: Option<String>,
    /// Number of shards (optional - if not set, collection is not sharded)
    #[serde(rename = "numShards")]
    pub num_shards: Option<u16>,
    /// Field to use for sharding key (default: "_key")
    #[serde(rename = "shardKey")]
    pub shard_key: Option<String>,
    /// Replication factor (optional, default: 1 = no replicas)
    #[serde(rename = "replicationFactor")]
    pub replication_factor: Option<u16>,
}

#[derive(Debug, Serialize)]
pub struct CreateCollectionResponse {
    pub name: String,
    pub status: String,
    /// Number of shards (if sharded)
    #[serde(rename = "numShards", skip_serializing_if = "Option::is_none")]
    pub num_shards: Option<u16>,
    /// Shard key field (if sharded)
    #[serde(rename = "shardKey", skip_serializing_if = "Option::is_none")]
    pub shard_key: Option<String>,
    /// Replication factor (if sharded)
    #[serde(rename = "replicationFactor", skip_serializing_if = "Option::is_none")]
    pub replication_factor: Option<u16>,
}

#[derive(Debug, Serialize)]
pub struct CollectionSummary {
    pub name: String,
    pub count: usize,
    #[serde(rename = "localCount", skip_serializing_if = "Option::is_none")]
    pub local_count: Option<usize>,
    #[serde(rename = "type")]
    pub collection_type: String,
    #[serde(rename = "shardConfig", skip_serializing_if = "Option::is_none")]
    pub shard_config: Option<crate::sharding::coordinator::CollectionShardConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<crate::storage::CollectionStats>,
}

#[derive(Debug, Serialize)]
pub struct ListCollectionsResponse {
    pub collections: Vec<CollectionSummary>,
}

#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct UpdateCollectionPropertiesRequest {
    /// Collection type: "document", "edge", or "blob"
    #[serde(rename = "type")]
    pub type_: Option<String>,
    /// Number of shards (updating this triggers rebalance)
    #[serde(rename = "numShards", alias = "num_shards")]
    pub num_shards: Option<u16>,
    /// Replication factor (optional, default: 1 = no replicas)
    #[serde(rename = "replicationFactor", alias = "replication_factor")]
    pub replication_factor: Option<u16>,
    /// Whether to propagate this update to other nodes (default: true)
    #[serde(default)]
    pub propagate: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct CollectionPropertiesResponse {
    pub name: String,
    pub status: String,
    #[serde(rename = "shardConfig")]
    pub shard_config: crate::sharding::coordinator::CollectionShardConfig,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteQueryRequest {
    pub query: String,
    #[serde(rename = "bindVars", default)]
    pub bind_vars: HashMap<String, Value>,
    #[serde(rename = "batchSize", default = "default_batch_size")]
    pub batch_size: usize,
}

fn default_batch_size() -> usize {
    100
}

#[derive(Debug, Serialize)]
pub struct ExecuteQueryResponse {
    pub result: Vec<Value>,
    pub count: usize,
    #[serde(rename = "hasMore")]
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub cached: bool,
    #[serde(rename = "executionTimeMs")]
    pub execution_time_ms: f64,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

// Convert DbError to HTTP response
impl IntoResponse for DbError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            DbError::CollectionNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            DbError::DocumentNotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            DbError::CollectionAlreadyExists(_) => (StatusCode::CONFLICT, self.to_string()),
            DbError::ParseError(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            DbError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            DbError::InvalidDocument(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

// ==================== Health Check Handler ====================

/// Simple health check endpoint for cluster node monitoring
/// Returns 200 OK if the node is alive and accepting requests
pub async fn health_check_handler() -> Json<Value> {
    Json(serde_json::json!({
        "status": "ok",
        "timestamp": chrono::Utc::now().to_rfc3339()
    }))
}

// ==================== Auth Handlers ====================

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
            crate::server::auth::AuthService::init(&state.storage, state.replication_log.as_deref())?;
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
    let user: crate::server::auth::User = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted user data".to_string()))?;

    // 6. Verify password
    if !crate::server::auth::AuthService::verify_password(&req.password, &user.password_hash) {
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
    pub expires_in: u32,  // seconds until expiration
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

// ==================== Database Handlers ====================

pub async fn create_database(
    State(state): State<AppState>,
    Json(req): Json<CreateDatabaseRequest>,
) -> Result<Json<CreateDatabaseResponse>, DbError> {
    state.storage.create_database(req.name.clone())?;

    // Record to replication log
    // Record to replication log
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: req.name.clone(),
            collection: "".to_string(),
            operation: Operation::CreateDatabase,
            key: "".to_string(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    // Auto-create _scripts collection for the new database
    if let Ok(db) = state.storage.get_database(&req.name) {
        if let Ok(_) = db.create_collection("_scripts".to_string(), None) {
             // Record _scripts creation to replication log
            if let Some(ref log) = state.replication_log {
                let metadata = serde_json::json!({
                    "type": "document",
                    "shardConfig": None::<serde_json::Value>
                });

                let entry = LogEntry {
                    sequence: 0,
                    node_id: "".to_string(),
                    database: req.name.clone(),
                    collection: "_scripts".to_string(),
                    operation: Operation::CreateCollection,
                    key: "".to_string(),
                    data: serde_json::to_vec(&metadata).ok(),
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                    origin_sequence: None,
                };
                let _ = log.append(entry);
            }
        }
    }


    Ok(Json(CreateDatabaseResponse {
        name: req.name,
        status: "created".to_string(),
    }))
}

pub async fn list_databases(State(state): State<AppState>) -> Json<ListDatabasesResponse> {
    let databases = state.storage.list_databases();
    Json(ListDatabasesResponse { databases })
}

pub async fn delete_database(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, DbError> {
    state.storage.delete_database(&name)?;

    // Record to replication log
    // Record to replication log
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: name.clone(),
            collection: "".to_string(),
            operation: Operation::DeleteDatabase,
            key: "".to_string(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }


    Ok(StatusCode::NO_CONTENT)
}

// ==================== Collection Handlers ====================

pub async fn create_collection(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<CreateCollectionRequest>,
) -> Result<Json<CreateCollectionResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    database.create_collection(req.name.clone(), req.collection_type.clone())?;

    let collection = database.get_collection(&req.name)?;

    // Store sharding configuration if specified
    // Auto-configure sharding for blob collections OR use explicitly provided config
    let shard_config = if let Some(num_shards) = req.num_shards {
        // Explicit sharding configuration provided
        Some(crate::sharding::coordinator::CollectionShardConfig {
            num_shards,
            shard_key: req.shard_key.clone().unwrap_or_else(|| "_key".to_string()),
            replication_factor: req.replication_factor.unwrap_or(1),
        })
    } else if req.collection_type.as_deref() == Some("blob") {
        // Blob collections are NOT auto-sharded by default - users can explicitly shard them if needed
        // Chunks will be distributed across the cluster for fault tolerance
        tracing::info!("Blob collection {} will use cluster-wide chunk distribution", req.name);
        None
    } else {
        None
    };

    // Apply sharding configuration if present
    if let Some(config) = shard_config {
        // Store shard config in collection metadata
        collection.set_shard_config(&config)?;

        // Initialize sharding via coordinator if available
        if let Some(ref coordinator) = state.shard_coordinator {
            tracing::info!("Initializing sharding for {}.{}: {:?}", db_name, req.name, config);

            // 1. Compute assignments (in-memory)
            coordinator.init_collection(&db_name, &req.name, &config)
                .map_err(|e| DbError::InternalError(format!("Failed to init sharding: {}", e)))?;

            // 2. Create physical shards (distributed)
            coordinator.create_shards(&db_name, &req.name).await
                .map_err(|e| DbError::InternalError(format!("Failed to create shards: {}", e)))?;
        }
    }

    // Set persistence type if blob
    if let Some(ctype) = &req.collection_type {
        if ctype == "blob" {
            collection.set_type("blob")?;
        }
    }

    // Record to replication log
    // Record to replication log
    if let Some(ref log) = state.replication_log {
        // Create metadata for replication
        // We reuse CreateCollectionMetadata struct but we might need to move it out of cluster::service if we delete it
        // Or redefine it. Let's use a simple JSON object for now or assume we migrated the struct.
        // Or even better, let's create a local struct or use serde_json::json!
        let metadata = serde_json::json!({
            "type": req.collection_type.clone().unwrap_or_else(|| "document".to_string()),
            "shardConfig": if let Some(num_shards) = req.num_shards {
                Some(serde_json::json!({
                    "num_shards": num_shards,
                    "shard_key": req.shard_key.clone().unwrap_or_else(|| "_key".to_string()),
                    "replication_factor": req.replication_factor.unwrap_or(1)
                }))
            } else {
                None::<serde_json::Value>
            }
        });

        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: db_name.clone(),
            collection: req.name.clone(),
            operation: Operation::CreateCollection,
            key: "".to_string(),
            data: serde_json::to_vec(&metadata).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }


    Ok(Json(CreateCollectionResponse {
        name: req.name,
        status: "created".to_string(),
        num_shards: req.num_shards,
        shard_key: req.shard_key,
        replication_factor: req.replication_factor,
    }))
}

pub async fn list_collections(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ListCollectionsResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let names = database.list_collections();

    // Get auth token from request headers to forward to remote nodes
    let auth_header = headers.get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let mut collections = Vec::with_capacity(names.len());
    for name in names {
        // Hide physical shard collections (they end with _sN where N is a number)
        // These are internal implementation details - users work with the logical collection
        if is_physical_shard_collection(&name) {
            continue;
        }

        if let Ok(coll) = database.get_collection(&name) {
            let shard_config = coll.get_shard_config();

            // For sharded collections, compute cluster-wide count and local count
            let (count, local_count) = if let Some(ref config) = shard_config {
                if config.num_shards > 0 {
                    // Local count: sum all local physical shards (both primary and replica on this node)
                    let mut local_total = 0usize;
                    for shard_id in 0..config.num_shards {
                        let physical_name = format!("{}_s{}", name, shard_id);
                        if let Ok(shard_coll) = database.get_collection(&physical_name) {
                            local_total += shard_coll.count();
                        }
                    }

                    // Cluster count: sum PRIMARY shard counts only (no replicas)
                    // For shards we're primary for, use local count
                    // For shards we're not primary for, query the primary node
                    let mut cluster_total = 0usize;

                    if let Some(ref coordinator) = state.shard_coordinator {
                        if let Some(table) = coordinator.get_shard_table(&db_name, &name) {
                            let local_id = if let Some(ref mgr) = state.cluster_manager {
                                mgr.local_node_id()
                            } else {
                                "local".to_string()
                            };

                            for shard_id in 0..config.num_shards {
                                let physical_name = format!("{}_s{}", name, shard_id);

                                if let Some(assignment) = table.assignments.get(&shard_id) {
                                    // Check if we're primary OR a replica for this shard
                                    let is_primary = assignment.primary_node == local_id || assignment.primary_node == "local";
                                    let is_replica = assignment.replica_nodes.contains(&local_id);

                                    if is_primary || is_replica {
                                        // We have this shard locally - use local count
                                        if let Ok(shard_coll) = database.get_collection(&physical_name) {
                                            cluster_total += shard_coll.count();
                                        }
                                    } else {
                                        // Query remote node for count - try primary first, then replicas
                                        let mut shard_count = 0usize;
                                        let mut found = false;

                                        if let Some(ref mgr) = state.cluster_manager {
                                            // Build list of nodes to try: primary first, then replicas
                                            let mut nodes_to_try = vec![assignment.primary_node.clone()];
                                            nodes_to_try.extend(assignment.replica_nodes.clone());

                                            let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
                                            let client = reqwest::Client::new();

                                            for node_id in &nodes_to_try {
                                                if let Some(addr) = mgr.get_node_api_address(node_id) {
                                                    let url = format!("http://{}/_api/database/{}/collection/{}/count", addr, db_name, physical_name);

                                                    let mut req = client.get(&url)
                                                        .header("X-Cluster-Secret", &secret)
                                                        .timeout(std::time::Duration::from_secs(2));

                                                    // Forward user's auth token
                                                    if !auth_header.is_empty() {
                                                        req = req.header("Authorization", &auth_header);
                                                    }

                                                    match req.send().await
                                                    {

                                                        Ok(res) if res.status().is_success() => {
                                                            if let Ok(json) = res.json::<serde_json::Value>().await {
                                                                if let Some(c) = json.get("count").and_then(|v| v.as_u64()) {
                                                                    shard_count = c as usize;
                                                                    found = true;
                                                                    break; // Got count, no need to try other nodes
                                                                }
                                                            }
                                                        }
                                                        _ => {
                                                            // This node failed, try next
                                                            tracing::debug!("Failed to query shard {} count from {}, trying next", shard_id, node_id);
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        if found {
                                            cluster_total += shard_count;
                                        } else {
                                            tracing::warn!("Could not get count for shard {} from any node", shard_id);
                                        }
                                    }
                                }
                            }
                        } else {
                            // No shard table - just use local count
                            cluster_total = local_total;
                        }
                    } else {
                        // No coordinator - just use local count
                        cluster_total = local_total;
                    }

                    (cluster_total, Some(local_total))
                } else {
                    (coll.count(), None)
                }
            } else {
                (coll.count(), None)
            };

            let collection_type = coll.get_type().to_string();

            // For sharded collections, aggregate stats from physical shards (local + remote)
            let stats = if let Some(ref config) = shard_config {
                if config.num_shards > 0 {
                    let mut total_sst_files_size = 0u64;
                    let mut total_live_data_size = 0u64;
                    let mut total_num_sst_files = 0u64;
                    let mut total_memtable_size = 0u64;
                    let mut total_chunk_count = 0usize;

                    // Get shard table to know where each shard lives
                    let shard_table = if let Some(ref coordinator) = state.shard_coordinator {
                        coordinator.get_shard_table(&db_name, &name)
                    } else {
                        None
                    };

                    let local_id = if let Some(ref mgr) = state.cluster_manager {
                        mgr.local_node_id()
                    } else {
                        "local".to_string()
                    };

                    let client = reqwest::Client::new();
                    let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();

                    for shard_id in 0..config.num_shards {
                        let physical_name = format!("{}_s{}", name, shard_id);

                        // Check if we are the PRIMARY for this shard (not just replica)
                        // Only count from primaries to avoid double-counting disk usage
                        let is_primary_local = if let Some(ref table) = shard_table {
                            if let Some(assignment) = table.assignments.get(&shard_id) {
                                assignment.primary_node == local_id ||
                                assignment.primary_node == "local"
                            } else {
                                false
                            }
                        } else {
                            // No shard table - check if collection exists locally
                            database.get_collection(&physical_name).is_ok()
                        };

                        if is_primary_local {

                            // Use local stats
                            if let Ok(shard_coll) = database.get_collection(&physical_name) {
                                let s = shard_coll.stats();
                                total_sst_files_size += s.disk_usage.sst_files_size;
                                total_live_data_size += s.disk_usage.live_data_size;
                                total_num_sst_files += s.disk_usage.num_sst_files;
                                total_memtable_size += s.disk_usage.memtable_size;
                                total_chunk_count += s.chunk_count;
                            }
                        } else {
                            // Query remote node for stats
                            if let Some(ref table) = shard_table {
                                if let Some(assignment) = table.assignments.get(&shard_id) {
                                    if let Some(ref mgr) = state.cluster_manager {
                                        // Try primary first, then replicas
                                        let mut nodes_to_try = vec![assignment.primary_node.clone()];
                                        nodes_to_try.extend(assignment.replica_nodes.clone());

                                        for node_id in &nodes_to_try {
                                            if let Some(addr) = mgr.get_node_api_address(node_id) {
                                                let url = format!("http://{}/_api/database/{}/collection/{}/stats?local=true", addr, db_name, physical_name);

                                                let mut req = client.get(&url)
                                                    .header("X-Cluster-Secret", &secret)
                                                    .timeout(std::time::Duration::from_secs(2));

                                                // Forward user's auth token
                                                if !auth_header.is_empty() {
                                                    req = req.header("Authorization", &auth_header);
                                                }

                                                match req.send().await
                                                {

                                                    Ok(res) if res.status().is_success() => {
                                                        if let Ok(json) = res.json::<serde_json::Value>().await {
                                                            total_chunk_count += json.get("chunk_count").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                                                            if let Some(disk) = json.get("disk_usage") {
                                                                total_sst_files_size += disk.get("sst_files_size").and_then(|v| v.as_u64()).unwrap_or(0);
                                                                total_live_data_size += disk.get("live_data_size").and_then(|v| v.as_u64()).unwrap_or(0);
                                                                total_num_sst_files += disk.get("num_sst_files").and_then(|v| v.as_u64()).unwrap_or(0);
                                                                total_memtable_size += disk.get("memtable_size").and_then(|v| v.as_u64()).unwrap_or(0);
                                                            }
                                                        }
                                                        break; // Got stats, no need to try other nodes
                                                    }
                                                    _ => {
                                                        // This node failed, try next
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    crate::storage::CollectionStats {
                        name: name.clone(),
                        document_count: count,
                        chunk_count: total_chunk_count,
                        disk_usage: crate::storage::DiskUsage {
                            sst_files_size: total_sst_files_size,
                            live_data_size: total_live_data_size,
                            num_sst_files: total_num_sst_files,
                            memtable_size: total_memtable_size,
                        }
                    }
                } else {
                    coll.stats()
                }
            } else {
                coll.stats()
            };


            collections.push(CollectionSummary {
                name,
                count,
                local_count,
                collection_type,
                shard_config,
                stats: Some(stats),
            });
        }
    }

    // Sort by name for consistent UI
    collections.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(ListCollectionsResponse { collections }))
}

pub async fn delete_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, DbError> {
    // Protect system collections
    if is_protected_collection(&db_name, &coll_name) {
        return Err(DbError::BadRequest(format!("Cannot delete protected system collection: {}", coll_name)));
    }

    let database = state.storage.get_database(&db_name)?;

    // Check if this is a direct shard delete request (internal)
    let is_shard_direct = headers.contains_key("X-Shard-Direct");

    // For sharded collections, delete all physical shards (local and remote)
    if let Ok(collection) = database.get_collection(&coll_name) {
        if let Some(shard_config) = collection.get_shard_config() {
            if shard_config.num_shards > 0 && !is_shard_direct {
                // Get nodes for remote deletion
                let remote_nodes: Vec<(String, String)> = if let Some(ref mgr) = state.cluster_manager {
                    let my_id = mgr.local_node_id();
                    mgr.state().get_all_members()
                        .into_iter()
                        .filter(|m| m.node.id != my_id)
                        .map(|m| (m.node.id.clone(), m.node.api_address.clone()))
                        .collect()
                } else {
                    vec![]
                };

                // Delete physical shards locally
                for shard_id in 0..shard_config.num_shards {
                    let physical_name = format!("{}_s{}", coll_name, shard_id);
                    let _ = database.delete_collection(&physical_name);
                }

                // Delete physical shards on remote nodes
                if !remote_nodes.is_empty() {
                    let client = reqwest::Client::new();
                    let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();

                    for shard_id in 0..shard_config.num_shards {
                        let physical_name = format!("{}_s{}", coll_name, shard_id);

                        for (_node_id, addr) in &remote_nodes {
                            let url = format!("http://{}/_api/database/{}/collection/{}", addr, db_name, physical_name);
                            let _ = client.delete(&url)
                                .header("X-Shard-Direct", "true")
                                .header("X-Cluster-Secret", &secret)
                                .timeout(std::time::Duration::from_secs(10))
                                .send()
                                .await;
                        }
                    }
                }
            }
        }
    }

    // Delete the logical collection
    database.delete_collection(&coll_name)?;

    // Record to replication log
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(), // Log assigns it
            database: db_name.clone(),
            collection: coll_name.clone(),
            operation: Operation::DeleteCollection,
            key: "".to_string(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    Ok(StatusCode::NO_CONTENT)
}


pub async fn truncate_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, DbError> {
    // Protect system collections
    if is_protected_collection(&db_name, &coll_name) {
        return Err(DbError::BadRequest(format!("Cannot truncate protected system collection: {}", coll_name)));
    }

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check if this is a direct shard truncate request (internal)
    let is_shard_direct = headers.contains_key("X-Shard-Direct");

    // Save shard config before truncating (truncate may clear it)
    let saved_shard_config = collection.get_shard_config();

    // For sharded collections, also truncate all physical shards on this node
    let mut total_count = 0usize;
    if let Some(ref shard_config) = saved_shard_config {
        if shard_config.num_shards > 0 {
            // Get nodes for remote truncation (only for non-direct requests)
            let remote_nodes: Vec<(String, String)> = if !is_shard_direct {
                if let Some(ref mgr) = state.cluster_manager {
                    let my_id = mgr.local_node_id();
                    mgr.state().get_all_members()
                        .into_iter()
                        .filter(|m| m.node.id != my_id)
                        .map(|m| (m.node.id.clone(), m.node.api_address.clone()))
                        .collect()
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            // Truncate physical shards locally
            for shard_id in 0..shard_config.num_shards {
                let physical_name = format!("{}_s{}", coll_name, shard_id);
                if let Ok(shard_coll) = database.get_collection(&physical_name) {
                    let c = shard_coll.clone();
                    if let Ok(count) = tokio::task::spawn_blocking(move || c.truncate()).await.map_err(|e| DbError::InternalError(format!("Task error: {}", e)))? {
                        total_count += count;
                    }
                }
            }

            // Truncate physical shards on remote nodes
            if !remote_nodes.is_empty() {
                let client = reqwest::Client::new();
                let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
                let auth_header = headers.get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                for shard_id in 0..shard_config.num_shards {
                    let physical_name = format!("{}_s{}", coll_name, shard_id);

                    for (_node_id, addr) in &remote_nodes {
                        let url = format!("http://{}/_api/database/{}/collection/{}/truncate", addr, db_name, physical_name);
                        let mut req = client.put(&url)
                            .header("X-Shard-Direct", "true")
                            .header("X-Cluster-Secret", &secret)
                            .timeout(std::time::Duration::from_secs(10));

                        if !auth_header.is_empty() {
                            req = req.header("Authorization", &auth_header);
                        }

                        let _ = req.send().await;
                    }
                }
            }
        }
    }

    // Also truncate the logical collection (may have some data or metadata)
    let coll = collection.clone();
    let count = tokio::task::spawn_blocking(move || coll.truncate())
        .await
        .map_err(|e| DbError::InternalError(format!("Task error: {}", e)))??;
    total_count += count;

    // Restore shard config after truncating
    if let Some(config) = saved_shard_config.clone() {
        let _ = collection.set_shard_config(&config);
    }

    // Record to replication log (only for non-direct requests to avoid duplicate logging)
    if !is_shard_direct {
        if let Some(ref log) = state.replication_log {
            let entry = LogEntry {
                 sequence: 0,
                node_id: "".to_string(),
                database: db_name.clone(),
                collection: coll_name.clone(),
                operation: Operation::TruncateCollection,
                key: "".to_string(),
                data: None,
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            };
            let _ = log.append(entry);
        }
    }

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "deleted": total_count,
        "status": "truncated"
    })))
}

pub async fn compact_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    collection.compact();

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "status": "compacted"
    })))
}

#[derive(Debug, Deserialize)]
pub struct PruneCollectionRequest {
    pub older_than: String, // ISO8601
}

#[derive(Debug, Serialize)]
pub struct PruneCollectionResponse {
    pub status: String,
    pub timestamp_ms: u64,
}

pub async fn prune_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<PruneCollectionRequest>,
) -> Result<Json<PruneCollectionResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let collection = db.get_collection(&coll_name)?;

    // Parse timestamp
    let dt = chrono::DateTime::parse_from_rfc3339(&req.older_than)
        .map_err(|e| DbError::BadRequest(format!("Invalid timestamp format: {}", e)))?;
    
    let ts_i64 = dt.timestamp_millis();
    if ts_i64 < 0 {
        return Err(DbError::BadRequest("Pruning timestamp must be after 1970-01-01".to_string()));
    }
    
    // Ensure accurate conversion to u64 ms
    let timestamp_ms = ts_i64 as u64;

    collection.prune_older_than(timestamp_ms)?;

    tracing::info!("Pruned collection {}/{} older than {}", db_name, coll_name, req.older_than);

    Ok(Json(PruneCollectionResponse {
        status: "pruned".to_string(),
        timestamp_ms,
    }))
}

/// Get document count for a collection (used for cluster-wide aggregation)
pub async fn get_collection_count(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, DbError> {
    // Get auth token from request headers to forward to remote nodes
    let auth_header = headers.get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let count = if let Some(ref coordinator) = state.shard_coordinator {
        match coordinator.get_total_count(&db_name, &coll_name, auth_header).await {
            Ok(c) => c,
            Err(_) => {
                // Fallback to local count if cluster aggregation fails
                let database = state.storage.get_database(&db_name)?;
                let collection = database.get_collection(&coll_name)?;
                collection.count()
            }
        }
    } else {
        let database = state.storage.get_database(&db_name)?;
        let collection = database.get_collection(&coll_name)?;
        collection.count()
    };

    Ok(Json(serde_json::json!({
        "count": count
    })))
}

/// Recount documents from actual RocksDB data (bypasses cache)
/// Useful for debugging replication consistency
pub async fn recount_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let cached_count = collection.count();
    let actual_count = collection.recount_documents();

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "cached_count": cached_count,
        "actual_count": actual_count,
        "match": cached_count == actual_count,
        "status": "recounted"
    })))
}

/// Repair sharded collection by removing misplaced documents
pub async fn repair_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    if let Some(coordinator) = state.shard_coordinator {
         let report = coordinator.repair_collection(&db_name, &coll_name).await
             .map_err(|e| DbError::InternalError(e))?;

         Ok(Json(serde_json::json!({
             "status": "repaired",
             "report": report
         })))
    } else {
        Err(DbError::InternalError("Shard coordinator not available".to_string()))
    }
}

pub async fn get_collection_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((db_name, coll_name)): Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let mut stats = collection.stats();
    let collection_type = collection.get_type();

    // For sharded collections, try to get aggregated count
    if let Some(ref coordinator) = state.shard_coordinator {
         let auth_header = headers.get("authorization")
             .and_then(|v| v.to_str().ok())
             .map(|s| s.to_string());
         
         if let Ok(total) = coordinator.get_total_count(&db_name, &coll_name, auth_header).await {
              stats.document_count = total;
         }
    }

    // Check if this is a local-only request (to prevent infinite recursion when aggregating)
    let _local_only = params.get("local").map(|v| v == "true").unwrap_or(false);

    // Get shard configuration
    let shard_config = collection.get_shard_config();
    let is_sharded = shard_config.as_ref().map(|c| c.num_shards > 0).unwrap_or(false);

    // Build sharding stats
    let sharding_stats = if let Some(config) = &shard_config {
        serde_json::json!({
            "enabled": is_sharded,
            "num_shards": config.num_shards,
            "shard_key": config.shard_key,
            "replication_factor": config.replication_factor
        })
    } else {
        serde_json::json!({
            "enabled": false,
            "num_shards": 0,
            "shard_key": null,
            "replication_factor": 1
        })
    };

    // Build cluster distribution info
    let cluster_stats = if let Some(ref coordinator) = state.shard_coordinator {
        let all_nodes = coordinator.get_node_addresses();
        let total_nodes = all_nodes.len();
        let _my_address = coordinator.my_address();

        // For sharded collections, calculate shard distribution with doc counts
        let shard_distribution = if is_sharded {
            let config = shard_config.as_ref().unwrap();

            // Use total document count / num_shards as approximation
            // Scanning all docs is too expensive and blocks the server
            let total_docs = stats.document_count;
            let docs_per_shard = if config.num_shards > 0 {
                total_docs / config.num_shards as usize
            } else {
                total_docs
            };

            let mut shards_info: Vec<serde_json::Value> = Vec::new();

            for shard_id in 0..config.num_shards {
                let mut nodes_for_shard: Vec<String> = Vec::new();

                if total_nodes > 0 {
                    let primary_idx = (shard_id as usize) % total_nodes;
                    let primary_node = all_nodes.get(primary_idx).cloned().unwrap_or_default();
                    nodes_for_shard.push(primary_node);

                    // Replica nodes
                    for r in 1..config.replication_factor {
                        let replica_idx = (primary_idx + r as usize) % total_nodes;
                        if replica_idx != primary_idx {
                            let replica_node = all_nodes.get(replica_idx).cloned().unwrap_or_default();
                            nodes_for_shard.push(replica_node);
                        }
                    }
                }

                shards_info.push(serde_json::json!({
                    "shard_id": shard_id,
                    "nodes": nodes_for_shard,
                    "document_count": docs_per_shard  // Approximate
                }));
            }

            serde_json::to_value(shards_info).unwrap_or(serde_json::json!([]))
        } else {
            // Non-sharded: single "shard" with all docs
            serde_json::json!([{
                "shard_id": 0,
                "nodes": all_nodes.clone(),
                "document_count": stats.document_count
            }])
        };

        serde_json::json!({
            "cluster_mode": true,
            "total_nodes": total_nodes,
            "nodes": all_nodes,
            "shards": shard_distribution
        })
    } else {
        serde_json::json!({
            "cluster_mode": false,
            "total_nodes": 1,
            "nodes": [],
            "distribution": {}
        })
    };

    // Calculate local document count (documents stored on this node's shards)
    // For non-sharded collections, local = total (all replicated)
    // For sharded collections, use total count as approximation
    // (Scanning all docs is too expensive and blocks the server)
    let local_document_count = stats.document_count;

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "type": collection_type,
        "document_count": stats.document_count,
        "local_document_count": local_document_count,
        "disk_usage": {
            "sst_files_size": stats.disk_usage.sst_files_size,
            "live_data_size": stats.disk_usage.live_data_size,
            "num_sst_files": stats.disk_usage.num_sst_files,
            "memtable_size": stats.disk_usage.memtable_size,
            "total_size": stats.disk_usage.sst_files_size + stats.disk_usage.memtable_size
        },
        "sharding": sharding_stats,
        "cluster": cluster_stats
    })))
}

/// Get detailed sharding information including per-shard document counts, disk sizes, and node assignments
pub async fn get_sharding_details(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let shard_config = collection.get_shard_config();

    let is_sharded = shard_config.as_ref().map(|c| c.num_shards > 0).unwrap_or(false);

    if !is_sharded {
        // Not a sharded collection
        let stats = collection.stats();
        return Ok(Json(serde_json::json!({
            "database": db_name,
            "collection": coll_name,
            "type": collection.get_type(),
            "sharded": false,
            "total_documents": stats.document_count,
            "total_size": stats.disk_usage.sst_files_size + stats.disk_usage.memtable_size,
            "shards": []
        })));
    }

    let config = shard_config.unwrap();

    // Get cluster nodes info
    let (nodes, healthy_nodes, node_id_to_address) = if let Some(ref coordinator) = state.shard_coordinator {
        let all_node_ids = coordinator.get_node_ids();
        let my_node_id = coordinator.my_node_id();

        // Build node ID to address mapping
        let mut id_to_addr: HashMap<String, String> = HashMap::new();
        if let Some(ref mgr) = state.cluster_manager {
            for member in mgr.state().get_all_members() {
                id_to_addr.insert(member.node.id.clone(), member.node.api_address.clone());
            }
        }

        // Get healthy nodes from cluster manager
        let healthy = if let Some(ref mgr) = state.cluster_manager {
            mgr.get_healthy_nodes()
        } else {
            vec![my_node_id.clone()]
        };

        (all_node_ids, healthy, id_to_addr)
    } else {
        (vec!["local".to_string()], vec!["local".to_string()], HashMap::new())
    };

    // Get shard table for assignments
    let shard_table = if let Some(ref coordinator) = state.shard_coordinator {
        coordinator.get_shard_table(&db_name, &coll_name)
    } else {
        None
    };

    let mut shards_info: Vec<serde_json::Value> = Vec::new();
    let mut total_documents = 0u64;
    let mut total_size = 0u64;

    // Get my node ID to check if shard is local
    let my_node_id = if let Some(ref coordinator) = state.shard_coordinator {
        coordinator.my_node_id()
    } else {
        "local".to_string()
    };

    // Query each physical shard for actual stats
    // Use scatter-gather to query remote nodes when shard isn't local
    let client = reqwest::Client::new();
    let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();

    // Get auth token from request headers to forward to remote nodes
    let auth_header = headers.get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    for shard_id in 0..config.num_shards {
        let physical_coll_name = format!("{}_s{}", coll_name, shard_id);

        // Get assignment info first
        let (primary_node, replica_nodes) = if let Some(ref table) = shard_table {
            if let Some(assignment) = table.assignments.get(&shard_id) {
                (assignment.primary_node.clone(), assignment.replica_nodes.clone())
            } else {
                ("unknown".to_string(), vec![])
            }
        } else {
            // Fall back to computing assignment based on modulo
            let num_nodes = nodes.len();
            if num_nodes > 0 {
                let primary_idx = (shard_id as usize) % num_nodes;
                let primary = nodes.get(primary_idx).cloned().unwrap_or_default();
                let mut replicas = Vec::new();
                for r in 1..config.replication_factor {
                    let replica_idx = (primary_idx + r as usize) % num_nodes;
                    if replica_idx != primary_idx {
                        if let Some(n) = nodes.get(replica_idx) {
                            replicas.push(n.clone());
                        }
                    }
                }
                (primary, replicas)
            } else {
                ("local".to_string(), vec![])
            }
        };

        // Get stats - either local or remote
        let mut stats_result: Option<(u64, u64, u64)> = None;

        // 1. Try Primary Node (Local or Remote)
        if primary_node == my_node_id || primary_node == "local" {
            // Local primary
            if let Ok(physical_coll) = database.get_collection(&physical_coll_name) {
                let stats = physical_coll.stats();
                stats_result = Some((stats.document_count as u64, stats.chunk_count as u64, stats.disk_usage.sst_files_size + stats.disk_usage.memtable_size));
            }
        } else if let Some(primary_addr) = node_id_to_address.get(&primary_node) {
            // Remote primary
            let scheme = std::env::var("SOLIDB_CLUSTER_SCHEME").unwrap_or_else(|_| "http".to_string());
            let url = format!("{}://{}/_api/database/{}/collection/{}/stats", scheme, primary_addr, db_name, physical_coll_name);
            let mut req = client.get(&url)
                .header("X-Cluster-Secret", &secret)
                .timeout(std::time::Duration::from_secs(3));

            if !auth_header.is_empty() {
                req = req.header("Authorization", &auth_header);
            }

            if let Ok(res) = req.send().await {
                if res.status().is_success() {
                    if let Ok(body) = res.json::<serde_json::Value>().await {
                        let count = body.get("document_count").and_then(|c| c.as_u64()).unwrap_or(0);
                        let chunk_count = body.get("chunk_count").and_then(|c| c.as_u64()).unwrap_or(0);
                        let disk = body.get("disk_usage");
                        let size = disk.and_then(|d| {
                            let sst = d.get("sst_files_size").and_then(|v| v.as_u64()).unwrap_or(0);
                            let mem = d.get("memtable_size").and_then(|v| v.as_u64()).unwrap_or(0);
                            Some(sst + mem)
                        }).unwrap_or(0);
                        stats_result = Some((count, chunk_count, size));
                    }
                }
            }
        }

        // 2. Fallback to Replicas if Primary failed
        if stats_result.is_none() {
            for replica_node in &replica_nodes {
                if replica_node == &my_node_id {
                    // Local replica
                    if let Ok(physical_coll) = database.get_collection(&physical_coll_name) {
                        let stats = physical_coll.stats();
                        stats_result = Some((stats.document_count as u64, stats.chunk_count as u64, stats.disk_usage.sst_files_size + stats.disk_usage.memtable_size));
                        break;
                    }
                } else if let Some(replica_addr) = node_id_to_address.get(replica_node) {
                    // Remote replica
                    let scheme = std::env::var("SOLIDB_CLUSTER_SCHEME").unwrap_or_else(|_| "http".to_string());
                    let url = format!("{}://{}/_api/database/{}/collection/{}/stats", scheme, replica_addr, db_name, physical_coll_name);
                    let mut req = client.get(&url)
                        .header("X-Cluster-Secret", &secret)
                        .timeout(std::time::Duration::from_secs(2));

                    if !auth_header.is_empty() {
                        req = req.header("Authorization", &auth_header);
                    }

                    if let Ok(res) = req.send().await {
                        if res.status().is_success() {
                            if let Ok(body) = res.json::<serde_json::Value>().await {
                                let count = body.get("document_count").and_then(|c| c.as_u64()).unwrap_or(0);
                                let chunk_count = body.get("chunk_count").and_then(|c| c.as_u64()).unwrap_or(0);
                                let disk = body.get("disk_usage");
                                let size = disk.and_then(|d| {
                                    let sst = d.get("sst_files_size").and_then(|v| v.as_u64()).unwrap_or(0);
                                    let mem = d.get("memtable_size").and_then(|v| v.as_u64()).unwrap_or(0);
                                    Some(sst + mem)
                                }).unwrap_or(0);
                                stats_result = Some((count, chunk_count, size));
                                break;
                            }
                        }
                    }
                }
            }
        }

        let (doc_count, chunk_count, disk_size) = stats_result.unwrap_or((0, 0, 0));
        let fetch_failed = stats_result.is_none();

        let primary_healthy = healthy_nodes.contains(&primary_node);
        let primary_address = node_id_to_address.get(&primary_node).cloned().unwrap_or_else(|| primary_node.clone());

        // Build replica info with health status
        let replicas_info: Vec<serde_json::Value> = replica_nodes.iter().map(|node_id| {
            let is_healthy = healthy_nodes.contains(node_id);
            let address = node_id_to_address.get(node_id).cloned().unwrap_or_else(|| node_id.clone());
            serde_json::json!({
                "node_id": node_id,
                "address": address,
                "healthy": is_healthy
            })
        }).collect();

        total_documents += doc_count;
        total_size += disk_size;

        // Status checks - distinguish between dead node and syncing shard
        let shard_status = if !primary_healthy {
            "dead" // Node is actually unhealthy
        } else if fetch_failed && primary_node != my_node_id {
            "syncing" // Node is healthy but shard data not available yet
        } else {
            "healthy"
        };

        shards_info.push(serde_json::json!({
            "shard_id": shard_id,
            "physical_collection": physical_coll_name,
            "document_count": doc_count,
            "chunk_count": chunk_count,
            "disk_size": disk_size,
            "disk_size_formatted": format_size(disk_size),
            "primary": {
                "node_id": primary_node,
                "address": primary_address,
                "healthy": primary_healthy
            },
            "replicas": replicas_info,
            "status": shard_status,
            "fetch_failed": fetch_failed
        }));
    }


    // Build node summary with actual status from cluster state
    // NodeId -> (PrimaryDocs, ReplicaDocs, PrimarySize, ReplicaSize, HasFailedFetch)
    struct NodeStat {
        primary_docs: u64,
        replica_docs: u64,
        primary_chunks: u64,
        replica_chunks: u64,
        primary_size: u64,
        replica_size: u64,
        has_failed: bool,
    }

    let mut node_stats: HashMap<String, NodeStat> = HashMap::new();

    // First, initialize with all primary nodes from assignment to ensure we track them even if fetch failed
    for shard in &shards_info {
        // Track Primary Stats
        if let Some(primary) = shard.get("primary").and_then(|p| p.get("node_id")).and_then(|n| n.as_str()) {
            let doc_count = shard.get("document_count").and_then(|d| d.as_u64()).unwrap_or(0);
            let chunk_count = shard.get("chunk_count").and_then(|c| c.as_u64()).unwrap_or(0);
            let disk_size = shard.get("disk_size").and_then(|d| d.as_u64()).unwrap_or(0);
            let fetch_failed = shard.get("fetch_failed").and_then(|f| f.as_bool()).unwrap_or(false);

            let entry = node_stats.entry(primary.to_string()).or_insert(NodeStat {
                primary_docs: 0, replica_docs: 0, 
                primary_chunks: 0, replica_chunks: 0,
                primary_size: 0, replica_size: 0, has_failed: false
            });

            entry.primary_docs += doc_count;
            entry.primary_chunks += chunk_count;
            entry.primary_size += disk_size;
            if fetch_failed {
                entry.has_failed = true;
            }
        }

        // Track Replica Stats
        if let Some(replicas) = shard.get("replicas").and_then(|r| r.as_array()) {
            for replica in replicas {
                if let Some(replica_node) = replica.get("node_id").and_then(|n| n.as_str()) {
                    // Replicas have the same doc count/disk size as the primary (they're copies)
                    let doc_count = shard.get("document_count").and_then(|d| d.as_u64()).unwrap_or(0);
                    let chunk_count = shard.get("chunk_count").and_then(|c| c.as_u64()).unwrap_or(0);
                    let disk_size = shard.get("disk_size").and_then(|d| d.as_u64()).unwrap_or(0);

                    let entry = node_stats.entry(replica_node.to_string()).or_insert(NodeStat {
                        primary_docs: 0, replica_docs: 0, 
                        primary_chunks: 0, replica_chunks: 0,
                        primary_size: 0, replica_size: 0, has_failed: false
                    });

                    entry.replica_docs += doc_count;
                    entry.replica_chunks += chunk_count;
                    entry.replica_size += disk_size;
                }
            }
        }
    }

    // ALSO include all cluster members - not just those in shard assignments
    // This ensures returning nodes that haven't been assigned shards yet still appear
    if let Some(ref mgr) = state.cluster_manager {
        for member in mgr.state().get_all_members() {
            // Add to node_stats if not already present
            node_stats.entry(member.node.id.clone()).or_insert(NodeStat {
                primary_docs: 0, replica_docs: 0, 
                primary_chunks: 0, replica_chunks: 0,
                primary_size: 0, replica_size: 0, has_failed: false
            });
        }
    }

    let nodes_summary: Vec<serde_json::Value> = node_stats.iter().map(|(node_id, stats)| {
        let is_healthy = healthy_nodes.contains(node_id);
        let address = node_id_to_address.get(node_id).cloned().unwrap_or_else(|| node_id.clone());
        let shard_count = shards_info.iter().filter(|s| {
            s.get("primary").and_then(|p| p.get("node_id")).and_then(|n| n.as_str()) == Some(node_id)
        }).count();

        // Count replica shards for this node
        let replica_count = shards_info.iter().filter(|s| {
            if let Some(replicas) = s.get("replicas").and_then(|r| r.as_array()) {
                replicas.iter().any(|r| {
                    r.get("node_id").and_then(|n| n.as_str()) == Some(node_id)
                })
            } else {
                false
            }
        }).count();

        // Get actual node status from cluster manager
        let mut status = if let Some(ref mgr) = state.cluster_manager {
            if let Some(member) = mgr.state().get_member(node_id) {
                match member.status {
                    crate::cluster::state::NodeStatus::Syncing => "syncing",
                    crate::cluster::state::NodeStatus::Active => "healthy",
                    crate::cluster::state::NodeStatus::Joining => "joining",
                    crate::cluster::state::NodeStatus::Suspected => "suspected",
                    crate::cluster::state::NodeStatus::Dead => "dead",
                    crate::cluster::state::NodeStatus::Leaving => "leaving",
                }
            } else if is_healthy {
                // Node id not in cluster state but marked healthy (shouldn't happen normally)
                "healthy"
            } else {
                // Node was removed from cluster - mark as dead
                "dead"
            }
        } else if is_healthy {
            "healthy"
        } else {
            // No cluster manager and not healthy - assume dead
            "dead"
        };

        // Override status if fetch failed - node is healthy but missing data
        if status == "healthy" && stats.has_failed {
             status = "syncing";  // Node is healthy but missing data, needs sync
        }

        let total_docs = stats.primary_docs + stats.replica_docs;
        let total_chunks = stats.primary_chunks + stats.replica_chunks;
        let total_size = stats.primary_size + stats.replica_size;

        serde_json::json!({
            "node_id": node_id,
            "address": address,
            "healthy": is_healthy,
            "status": status,
            "primary_shards": shard_count,
            "replica_shards": replica_count,
            "document_count": total_docs,
            "chunk_count": total_chunks,
            "primary_docs": stats.primary_docs,
            "replica_docs": stats.replica_docs,
            "primary_chunks": stats.primary_chunks,
            "replica_chunks": stats.replica_chunks,
            "disk_size": total_size,
            "primary_size": stats.primary_size,
            "replica_size": stats.replica_size,
            "disk_size_formatted": format_size(total_size)
        })
    }).collect();

    let mut nodes_sorted = nodes_summary;
    nodes_sorted.sort_by(|a, b| {
        let addr_a = a.get("address").and_then(|s| s.as_str()).unwrap_or("");
        let addr_b = b.get("address").and_then(|s| s.as_str()).unwrap_or("");
        addr_a.cmp(addr_b)
    });

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "type": collection.get_type(),
        "sharded": true,
        "config": {
            "num_shards": config.num_shards,
            "shard_key": config.shard_key,
            "replication_factor": config.replication_factor
        },
        "total_documents": total_documents,
        // Calculate total chunks for the collection (sum of primary chunks)
        "total_chunks": node_stats.values().map(|s| s.primary_chunks).sum::<u64>(),
        "total_size": total_size,
        "total_size_formatted": format_size(total_size),
        "cluster": {
            "total_nodes": nodes.len(),
            "healthy_nodes": healthy_nodes.len()
        },
        "nodes": nodes_sorted,
        "shards": shards_info
    })))
}

/// Format size in human-readable format
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

pub async fn update_collection_properties(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(payload): Json<UpdateCollectionPropertiesRequest>,
) -> Result<Json<CollectionPropertiesResponse>, DbError> {
    tracing::info!("update_collection_properties called: db={}, coll={}, payload={:?}", db_name, coll_name, payload);

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Update collection type if specified
    if let Some(new_type) = &payload.type_ {
        collection.set_type(new_type)?;
        tracing::info!("Updated collection type for {}/{} to {}", db_name, coll_name, new_type);
    }

    // Get existing config or create new one if not sharded yet
    let mut config = collection.get_shard_config()
        .unwrap_or_else(|| crate::sharding::coordinator::CollectionShardConfig::default());

    tracing::info!("Current config before update: {:?}", config);

    let old_num_shards = config.num_shards;
    let mut shard_count_changed = false;

    // Get healthy node count for capping shard/replica values
    let healthy_node_count = if let Some(ref coordinator) = state.shard_coordinator {
        let count = coordinator.get_node_addresses().len();
        tracing::info!("Coordinator reports {} nodes", count);
        count
    } else {
        tracing::info!("No coordinator, using 1 node");
        1
    };

    // Update num_shards if specified
    if let Some(mut num_shards) = payload.num_shards {
        if num_shards < 1 {
            return Err(DbError::BadRequest("Number of shards must be >= 1".to_string()));
        }

        // Cap num_shards to the number of healthy nodes
        tracing::info!(
            "Shard update check: requested={}, available_nodes={}",
            num_shards, healthy_node_count
        );

        if num_shards as usize > healthy_node_count {
            tracing::warn!(
                "Requested {} shards but only {} nodes available, capping to {}",
                num_shards, healthy_node_count, healthy_node_count
            );
            num_shards = healthy_node_count as u16;
        }

        if num_shards != config.num_shards {
            tracing::info!(
                "Updating num_shards for {}.{} from {} to {}",
                db_name, coll_name, config.num_shards, num_shards
            );
            config.num_shards = num_shards;
            shard_count_changed = true;
        } else {
             tracing::info!("num_shards unchanged ({})", num_shards);
        }
    } else {
        tracing::warn!("Update payload missing num_shards. Valid keys: numShards, num_shards");
    }

    // Update replication_factor if specified
    if let Some(mut rf) = payload.replication_factor {
        if rf < 1 {
            return Err(DbError::BadRequest("Replication factor must be >= 1".to_string()));
        }

        // Cap replication_factor to the number of healthy nodes
        if rf as usize > healthy_node_count {
            tracing::warn!(
                "Requested replication factor {} but only {} nodes available, capping to {}",
                rf, healthy_node_count, healthy_node_count
            );
            rf = healthy_node_count as u16;
        }

        config.replication_factor = rf;
    }

    tracing::info!("Saving config: {:?}", config);

    // Save updated config
    collection.set_shard_config(&config)?;

    tracing::info!("Config saved successfully");

    // Trigger rebalance if shard count changed
    if shard_count_changed {
        if let Some(ref coordinator) = state.shard_coordinator {
            tracing::info!(
                "Shard count changed from {} to {} for {}/{}, triggering rebalance",
                old_num_shards, config.num_shards, db_name, coll_name
            );
            // Spawn rebalance as background task to avoid blocking the response
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                if let Err(e) = coordinator.rebalance().await {
                   tracing::error!("Failed to trigger rebalance: {}", e);
                }
            });
        }
    }

    // Broadcast metadata update to other cluster nodes to ensure consistency
    // This prevents "split brain" where only the coordinator node knows the new config
    let propagate = payload.propagate.unwrap_or(true);

    if propagate {
        if let Some(ref manager) = state.cluster_manager {
            let my_node_id = manager.local_node_id();
            let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
            let client = reqwest::Client::new();

            // Clone payload and set propagate = false
            let mut forward_payload = payload.clone();
            forward_payload.propagate = Some(false);

            for member in manager.state().get_all_members() {
                if member.node.id == my_node_id {
                    continue;
                }

                let address = &member.node.api_address;
                let url = format!("http://{}/_api/database/{}/collection/{}/properties", address, db_name, coll_name);

                tracing::info!("Propagating config update to node {} ({})", member.node.id, address);

                // Spawn background task for propagation to avoid latency
                let client = client.clone();
                let payload = forward_payload.clone();
                let secret = secret.clone();
                let url = url.clone();

                tokio::spawn(async move {
                    match client.put(&url)
                        .header("X-Cluster-Secret", &secret)
                        .header("X-Shard-Direct", "true") // Bylass auth check
                        .json(&payload)
                        .send()
                        .await
                    {
                        Ok(res) => {
                            if !res.status().is_success() {
                                tracing::warn!("Failed to propagate config to {}: {}", url, res.status());
                            } else {
                                tracing::debug!("Successfully propagated config to {}", url);
                            }
                        }
                        Err(e) => {
                             tracing::warn!("Failed to send propagation request to {}: {}", url, e);
                        }
                    }
                });
            }
        }
    }

    Ok(Json(CollectionPropertiesResponse {
        name: coll_name,
        status: if shard_count_changed { "updated_rebalancing".to_string() } else { "updated".to_string() },
        shard_config: config,
    }))
}

pub async fn export_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    _headers: HeaderMap,
) -> Result<Response, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let shard_config = collection.get_shard_config();
    let is_blob = collection.get_type() == "blob";

    // Prepare coordinator reference and secret for remote calls
    let coordinator_opt = state.shard_coordinator.clone();
    let cluster_manager_opt = state.cluster_manager.clone();
    let secret_env = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
    
    // Capture necessary variables for the async stream
    let db_name_clone = db_name.clone();
    let coll_name_clone = coll_name.clone();
    let collection_clone = collection.clone();
    let state_storage = state.storage.clone();

    let stream = async_stream::stream! {
        let num_shards = shard_config.as_ref().map(|c| c.num_shards).unwrap_or(0);

        if num_shards > 0 {
            // SHARDED EXPORT: Iterate over all physical shards
            // We need to determine where each shard is located
            let shard_table = if let Some(coord) = &coordinator_opt {
                coord.get_shard_table(&db_name_clone, &coll_name_clone)
            } else {
                None
            };

            let client = reqwest::Client::new();
            let my_node_id = if let Some(mgr) = &cluster_manager_opt {
                mgr.local_node_id()
            } else {
                "local".to_string()
            };

            for shard_id in 0..num_shards {
                let physical_name = format!("{}_s{}", coll_name_clone, shard_id);
                
                // Determine primary node for this shard
                let primary_node = if let Some(ref table) = shard_table {
                    table.assignments.get(&shard_id).map(|a| a.primary_node.clone()).unwrap_or_else(|| "unknown".to_string())
                } else {
                     // Fallback: assume local if no table (standalone mode?) or simple modulo
                     "local".to_string()
                };

                let is_local = primary_node == "local" || primary_node == my_node_id;

                if is_local {
                    // Export from LOCAL physical shard
                    if let Ok(db) = state_storage.get_database(&db_name_clone) {
                        if let Ok(phys_coll) = db.get_collection(&physical_name) {
                             // Scan documents (load all into memory - current limitation)
                             let docs = phys_coll.scan(None);
                             
                             for doc in docs {
                                 // Yield document line
                                 let mut val = doc.to_value();
                                 if let Some(obj) = val.as_object_mut() {
                                     if let Some(ref config) = shard_config {
                                          obj.insert("_shardConfig".to_string(), serde_json::to_value(config).unwrap_or_default());
                                     }
                                 }
                                 if let Ok(json) = serde_json::to_string(&val) {
                                     yield Ok::<_, std::io::Error>(axum::body::Bytes::from(format!("{}\n", json)));
                                 }

                                 // Yield blob chunks if blob collection
                                 if is_blob {
                                     let key = &doc.key;
                                     let mut chunk_index: u32 = 0;
                                     loop {
                                         match phys_coll.get_blob_chunk(key, chunk_index) {
                                             Ok(Some(data)) => {
                                                 let chunk_header = serde_json::json!({
                                                     "_type": "blob_chunk",
                                                     "_doc_key": key,
                                                     "_chunk_index": chunk_index,
                                                     "_data_length": data.len()
                                                 });

                                                 if let Ok(header_json) = serde_json::to_string(&chunk_header) {
                                                     // Header line
                                                     yield Ok(axum::body::Bytes::from(format!("{}\n", header_json)));
                                                     // Binary data
                                                     yield Ok(axum::body::Bytes::from(data));
                                                     // Trailing newline delimiter
                                                     yield Ok(axum::body::Bytes::from("\n"));
                                                 }
                                                 chunk_index += 1;
                                             },
                                             Ok(None) => break,
                                             Err(e) => {
                                                 tracing::error!("Failed to read blob chunk {} for {}: {}", chunk_index, key, e);
                                                 break;
                                             }
                                         }
                                     }
                                 }
                             }
                        }
                    }
                } else {
                    // Export from REMOTE physical shard
                    if let Some(mgr) = &cluster_manager_opt {
                        if let Some(addr) = mgr.get_node_api_address(&primary_node) {
                            let url = format!("http://{}/_api/database/{}/collection/{}/export", addr, db_name_clone, physical_name);
                            tracing::info!("Exporting remote shard {} from {}", physical_name, addr);

                            let req = client.get(&url)
                                .header("X-Shard-Direct", "true")
                                .header("X-Cluster-Secret", &secret_env);
                            
                            // Stream the response
                            match req.send().await {
                                Ok(mut res) => {
                                    if res.status().is_success() {
                                        loop {
                                            match res.chunk().await {
                                                Ok(Some(bytes)) => yield Ok(bytes),
                                                Ok(None) => break,
                                                Err(e) => {
                                                    tracing::error!("Error reading remote stream: {}", e);
                                                    break;
                                                }
                                            }
                                        }
                                    } else {
                                        tracing::error!("Remote export failed: {}", res.status());
                                    }
                                },
                                Err(e) => tracing::error!("Remote request failed: {}", e),
                            }
                        }
                    }
                }
            }

        } else {
            // NON-SHARDED: Existing logic (scan logical collection)
            // Note: Logical collection matches physical for non-sharded
            let docs = collection_clone.scan(None);
            
            for doc in docs {
                let mut val = doc.to_value();
            if let Some(obj) = val.as_object_mut() {
                if let Some(ref config) = shard_config {
                     obj.insert("_shardConfig".to_string(), serde_json::to_value(config).unwrap_or_default());
                }
                // Export collection type so restore knows how to create it
                obj.insert("_collectionType".to_string(), Value::String(if is_blob { "blob".to_string() } else { "document".to_string() }));
            }
            if let Ok(json) = serde_json::to_string(&val) {
                yield Ok::<_, std::io::Error>(axum::body::Bytes::from(format!("{}\n", json)));
            }

            // For blob collections, also export the blob chunks
            if is_blob {
                {
                    let coll = &collection_clone;
                     let key = &doc.key;
                     // Iterate chunks until none found
                     let mut chunk_index: u32 = 0;
                     loop {
                         match coll.get_blob_chunk(key, chunk_index) {
                             Ok(Some(data)) => {
                                 // Create a specific chunk document
                                 let chunk_doc = serde_json::json!( {
                                     "_type": "blob_chunk",
                                     "_collectionType": "blob", // redundant but helpful context
                                     "_doc_key": key,
                                     "_chunk_index": chunk_index,
                                     "_data_length": data.len() // Required for binary reading
                                 });

                                 if let Ok(chunk_json) = serde_json::to_string(&chunk_doc) {
                                     yield Ok(axum::body::Bytes::from(format!("{}\n", chunk_json)));
                                 }
                                 
                                 yield Ok(axum::body::Bytes::from(data));
                                 yield Ok(axum::body::Bytes::from("\n")); // Newline delimiter for binary
                                 
                                 chunk_index += 1;
                             },
                             Ok(None) => break, // No more chunks
                             Err(e) => {
                                 tracing::error!("Failed to read blob chunk {} for {}: {}", chunk_index, key, e);
                                 break;
                             }
                         }
                     }
                }
            }
            }
        }
    };

    let body = Body::from_stream(stream);

    Response::builder()
        .header("Content-Type", "application/x-ndjson")
        .header("Content-Disposition", format!("attachment; filename=\"{}-{}.jsonl\"", sanitize_filename(&db_name), sanitize_filename(&coll_name)))
        .body(body)
        .map_err(|e| DbError::InternalError(format!("Failed to build response: {}", e)))
}

pub async fn import_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    mut multipart: Multipart,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name).or_else(|_| {
         // Auto-create
         tracing::info!("Auto-creating collection '{}' during import", coll_name);
         database.create_collection(coll_name.clone(), None)?;
         database.get_collection(&coll_name)
    })?;

    // Check sharding config once
    let shard_config = collection.get_shard_config();
    let is_sharded = shard_config.as_ref().map(|c| c.num_shards > 0).unwrap_or(false);
    
    let mut imported_count = 0;
    let mut failed_count = 0;

    while let Some(field) = multipart.next_field().await.map_err(|e| DbError::BadRequest(e.to_string()))? {
        if field.name() == Some("file") {
            let mut stream = field;
            let mut buffer = Vec::new();

            // Read first chunk to detect format
            if let Some(Ok(first_chunk)) = stream.next().await {
                buffer.extend_from_slice(&first_chunk);
            } else {
                continue; // Empty file
            }

            let first_char = buffer.iter().find(|&&b| !b.is_ascii_whitespace()).copied().unwrap_or(b' ');

            if first_char == b'{' {
                // Streaming Mode (JSONL / Mixed Binary)
                let mut batch_docs: Vec<Value> = Vec::with_capacity(1000);

                loop {
                    // Try to extract lines from buffer
                    while let Some(newline_pos) = buffer.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = buffer.drain(0..=newline_pos).collect();
                        let line_slice = &line_bytes[..line_bytes.len()-1]; // Trim newline

                        if line_slice.iter().all(|b| b.is_ascii_whitespace()) {
                            continue;
                        }

                        // Try parsing JSON
                        match serde_json::from_slice::<Value>(line_slice) {
                            Ok(doc) => {
                                // Check for Blob Chunk Header
                                let is_blob_chunk = doc.get("_type")
                                    .and_then(|t| t.as_str())
                                    .map(|t| t == "blob_chunk")
                                    .unwrap_or(false);

                                if is_blob_chunk {
                                    // Handle Binary Chunk
                                    if let Some(data_len) = doc.get("_data_length").and_then(|v| v.as_u64()) {
                                        let required_len = data_len as usize;
                                        let total_required = required_len + 1; // +1 for trailing newline

                                        // Ensure we have enough bytes
                                        while buffer.len() < total_required {
                                            match stream.next().await {
                                                Some(Ok(chunk)) => buffer.extend_from_slice(&chunk),
                                                Some(Err(e)) => return Err(DbError::BadRequest(e.to_string())),
                                                None => return Err(DbError::BadRequest("Unexpected EOF reading binary chunk".to_string())),
                                            }
                                        }

                                        // Extract binary data
                                        let chunk_data: Vec<u8> = buffer.drain(0..required_len).collect();
                                        // Consume trailing newline
                                        if !buffer.is_empty() && buffer[0] == b'\n' {
                                            buffer.drain(0..1);
                                        }

                                        // Put chunk (Directly, chunks are not batched usually)
                                        if let (Some(key), Some(index)) = (
                                            doc.get("_doc_key").and_then(|s| s.as_str()),
                                            doc.get("_chunk_index").and_then(|n| n.as_u64())
                                        ) {
                                            match collection.put_blob_chunk(key, index as u32, &chunk_data) {
                                                Ok(_) => imported_count += 1,
                                                Err(e) => {
                                                    tracing::error!("Failed to import blob chunk: {}", e);
                                                    failed_count += 1;
                                                }
                                            }
                                        }
                                    } else {
                                        // Legacy Base64 chunk or other format
                                         if let (Some(key), Some(index), Some(data_b64)) = (
                                            doc.get("_doc_key").and_then(|s| s.as_str()),
                                            doc.get("_chunk_index").and_then(|n| n.as_u64()),
                                            doc.get("_blob_data").and_then(|s| s.as_str())
                                        ) {
                                            if let Ok(data) = general_purpose::STANDARD.decode(data_b64) {
                                                match collection.put_blob_chunk(key, index as u32, &data) {
                                                    Ok(_) => imported_count += 1,
                                                    Err(e) => {
                                                        tracing::error!("Failed import blob chunk: {}", e);
                                                        failed_count += 1;
                                                    }
                                                }
                                            } else {
                                                failed_count += 1;
                                            }
                                        }
                                    }
                                } else {
                                    // Regular Document
                                    // Remove metadata if present
                                    let mut doc_to_insert = doc;
                                    if let Some(obj) = doc_to_insert.as_object_mut() {
                                        obj.remove("_database");
                                        obj.remove("_collection");
                                        obj.remove("_shardConfig");
                                    }
                                    batch_docs.push(doc_to_insert);

                                    // Check batch size
                                    if batch_docs.len() >= 1000 {
                                        if is_sharded && state.shard_coordinator.is_some() {
                                            let coordinator = state.shard_coordinator.as_ref().unwrap();
                                            let config = shard_config.as_ref().unwrap();
                                            let docs_to_insert: Vec<Value> = batch_docs.drain(..).collect();
                                            match coordinator.insert_batch(&db_name, &coll_name, config, docs_to_insert).await {
                                                Ok((successes, failures)) => {
                                                    imported_count += successes;
                                                    failed_count += failures;
                                                }
                                                Err(e) => {
                                                    tracing::error!("[IMPORT] Batch insert failed: {}", e);
                                                    failed_count += 1;
                                                }
                                            }
                                        } else {
                                            // Local batch insert
                                            match collection.insert_batch(batch_docs.clone()) {
                                                Ok(inserted) => {
                                                    if let Err(e) = collection.index_documents(&inserted) {
                                                        tracing::error!("Failed to index batch: {}", e);
                                                    }
                                                    // Replication log
                                                    if let Some(ref log) = state.replication_log {
                                                        for doc in &inserted {
                                                            let entry = LogEntry {
                                                                sequence: 0,
                                                                node_id: "".to_string(),
                                                                database: db_name.clone(),
                                                                collection: coll_name.clone(),
                                                                operation: Operation::Insert,
                                                                key: doc.key.clone(),
                                                                data: serde_json::to_vec(&doc.to_value()).ok(),
                                                                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                                                origin_sequence: None,
                                                            };
                                                            let _ = log.append(entry);
                                                        }
                                                    }
                                                    imported_count += inserted.len();
                                                },
                                                Err(e) => {
                                                    tracing::error!("Failed to insert batch: {}", e);
                                                    failed_count += batch_docs.len();
                                                }
                                            }
                                            batch_docs.clear();
                                        }
                                    }
                                }
                            },
                            Err(e) => {
                                tracing::warn!("Failed to parse line as JSON: {}", e);
                                failed_count += 1;
                            }
                        }
                    }

                    // Replenish buffer
                    match stream.next().await {
                        Some(Ok(chunk)) => buffer.extend_from_slice(&chunk),
                        Some(Err(e)) => return Err(DbError::BadRequest(e.to_string())),
                        None => break, // EOF
                    }
                }

                // Flush remaining batch
                if !batch_docs.is_empty() {
                    if is_sharded && state.shard_coordinator.is_some() {
                        let coordinator = state.shard_coordinator.as_ref().unwrap();
                        let config = shard_config.as_ref().unwrap();
                        let docs_to_insert: Vec<Value> = batch_docs.drain(..).collect();
                         match coordinator.insert_batch(&db_name, &coll_name, config, docs_to_insert).await {
                            Ok((successes, failures)) => {
                                imported_count += successes;
                                failed_count += failures;
                            }
                            Err(e) => {
                                tracing::error!("[IMPORT] Remaining batch insert failed: {}", e);
                                failed_count += 1;
                            }
                        }
                    } else {
                         match collection.insert_batch(batch_docs.clone()) {
                            Ok(inserted) => {
                                if let Err(e) = collection.index_documents(&inserted) {
                                    tracing::error!("Failed to index batch: {}", e);
                                }
                                if let Some(ref log) = state.replication_log {
                                    for doc in &inserted {
                                        let entry = LogEntry {
                                            sequence: 0,
                                            node_id: "".to_string(),
                                            database: db_name.clone(),
                                            collection: coll_name.clone(),
                                            operation: Operation::Insert,
                                            key: doc.key.clone(),
                                            data: serde_json::to_vec(&doc.to_value()).ok(),
                                            timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                            origin_sequence: None,
                                        };
                                        let _ = log.append(entry);
                                    }
                                }
                                imported_count += inserted.len();
                            },
                            Err(e) => {
                                tracing::error!("Failed to insert batch: {}", e);
                                failed_count += batch_docs.len();
                            }
                        }
                        batch_docs.clear();
                    }
                }

            } else {
                // Legacy Mode (Read Full File for Array/CSV)
                // Consume rest of stream into buffer
                while let Some(chunk_res) = stream.next().await {
                    let chunk = chunk_res.map_err(|e| DbError::BadRequest(e.to_string()))?;
                    buffer.extend_from_slice(&chunk);
                }
                
                let text = String::from_utf8(buffer).map_err(|e| DbError::BadRequest(format!("Invalid UTF-8: {}", e)))?;

                let docs: Vec<Value> = if first_char == b'[' {
                    serde_json::from_str(&text).map_err(|e| DbError::BadRequest(format!("Invalid JSON Array: {}", e)))?
                } else {
                     // CSV
                     let mut reader = csv::Reader::from_reader(text.as_bytes());
                     let headers = reader.headers().map_err(|e| DbError::BadRequest(e.to_string()))?.clone();
                     let mut csv_docs = Vec::new();

                     for result in reader.records() {
                         let record = result.map_err(|e| DbError::BadRequest(e.to_string()))?;
                         let mut map = serde_json::Map::new();
                         for (i, field) in record.iter().enumerate() {
                             if i < headers.len() {
                                 let val = if let Ok(n) = field.parse::<i64>() {
                                     Value::Number(n.into())
                                 } else if let Ok(f) = field.parse::<f64>() {
                                     if let Some(n) = serde_json::Number::from_f64(f) {
                                         Value::Number(n)
                                     } else {
                                         Value::String(field.to_string())
                                     }
                                 } else if let Ok(b) = field.parse::<bool>() {
                                     Value::Bool(b)
                                 } else {
                                     Value::String(field.to_string())
                                 };
                                 map.insert(headers[i].to_string(), val);
                             }
                         }
                         csv_docs.push(Value::Object(map));
                     }
                     csv_docs
                };
                
                // Legacy batch insert fallback logic
                let mut legacy_batch = Vec::with_capacity(1000);
                for doc in docs {
                    legacy_batch.push(doc);
                    if legacy_batch.len() >= 1000 {
                          if is_sharded && state.shard_coordinator.is_some() {
                                let coordinator = state.shard_coordinator.as_ref().unwrap();
                                let config = shard_config.as_ref().unwrap();
                                let docs_to_insert: Vec<Value> = legacy_batch.drain(..).collect();
                                match coordinator.insert_batch(&db_name, &coll_name, config, docs_to_insert).await {
                                    Ok((s, f)) => { imported_count += s; failed_count += f; },
                                    Err(_) => failed_count += 1,
                                }
                          } else {
                                match collection.insert_batch(legacy_batch.clone()) {
                                    Ok(inserted) => {
                                        if let Err(e) = collection.index_documents(&inserted) { tracing::error!("Idx error: {}",e); }
                                         if let Some(ref log) = state.replication_log {
                                            for doc in &inserted {
                                                let entry = LogEntry {
                                                    sequence: 0, node_id: "".to_string(), database: db_name.clone(), collection: coll_name.clone(),
                                                    operation: Operation::Insert, key: doc.key.clone(), 
                                                    data: serde_json::to_vec(&doc.to_value()).ok(), 
                                                    timestamp: chrono::Utc::now().timestamp_millis() as u64, origin_sequence: None,
                                                };
                                                let _ = log.append(entry);
                                            }
                                        }
                                        imported_count += inserted.len();
                                    },
                                    Err(_) => failed_count += legacy_batch.len(),
                                }
                                legacy_batch.clear();
                          }
                    }
                }
                 if !legacy_batch.is_empty() {
                      if is_sharded && state.shard_coordinator.is_some() {
                            let coordinator = state.shard_coordinator.as_ref().unwrap();
                            let config = shard_config.as_ref().unwrap();
                            let docs_to_insert: Vec<Value> = legacy_batch.drain(..).collect();
                            match coordinator.insert_batch(&db_name, &coll_name, config, docs_to_insert).await {
                                Ok((s, f)) => { imported_count += s; failed_count += f; },
                                Err(_) => failed_count += 1,
                            }
                      } else {
                           match collection.insert_batch(legacy_batch.clone()) {
                                Ok(inserted) => {
                                     if let Err(e) = collection.index_documents(&inserted) { tracing::error!("Idx error: {}",e); }
                                     if let Some(ref log) = state.replication_log {
                                         for doc in &inserted {
                                            let entry = LogEntry {
                                                sequence: 0, node_id: "".to_string(), database: db_name.clone(), collection: coll_name.clone(),
                                                operation: Operation::Insert, key: doc.key.clone(), 
                                                data: serde_json::to_vec(&doc.to_value()).ok(), 
                                                timestamp: chrono::Utc::now().timestamp_millis() as u64, origin_sequence: None,
                                            };
                                            let _ = log.append(entry);
                                        }
                                     }
                                     imported_count += inserted.len();
                                },
                                Err(_) => failed_count += legacy_batch.len(),
                           }
                           legacy_batch.clear();
                      }
                 }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "imported": imported_count,
        "failed": failed_count,
        "status": "completed"
    })))
}

// ==================== Document Handlers ====================


fn get_transaction_id(headers: &HeaderMap) -> Option<TransactionId> {
    headers
        .get("X-Transaction-ID")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| {
            // Support "tx:123" or just "123"
            let id_str = s.strip_prefix("tx:").unwrap_or(s);
            id_str.parse::<u64>().ok()
        })
        .map(TransactionId::from_u64)
}

pub async fn insert_document(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check for transaction context
    if let Some(tx_id) = get_transaction_id(&headers) {
        let tx_manager = state.storage.transaction_manager()?;
        let tx_arc = tx_manager.get(tx_id)?;
        let mut tx = tx_arc.write().map_err(|_| DbError::InternalError("Transaction lock poisoned".into()))?;
        let wal = tx_manager.wal();

        let doc = collection.insert_tx(&mut tx, wal, data)?;

        // No replication log for transactional write yet (will happen on commit)

        return Ok(Json(doc.to_value()));
    }

    // Check for sharding
    // If sharded and we have a coordinator, use it
    if let Some(shard_config) = collection.get_shard_config() {
        tracing::info!("[INSERT] shard_config found: num_shards={}", shard_config.num_shards);
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                // Check for direct shard access (prevention of infinite loops)
                if !headers.contains_key("X-Shard-Direct") {
                    tracing::info!("[INSERT] Using ShardCoordinator for {}/{}", db_name, coll_name);
                    let doc = coordinator.insert(
                        &db_name,
                        &coll_name,
                        &shard_config,
                        data
                    ).await?;

                    // NOTE: Don't add to replication log here!
                    // If we forwarded to another node, that node adds to its log.
                    // If we stored locally (we're the primary), ShardCoordinator already
                    // returned from collection.insert() which doesn't add to log -
                    // but the X-Shard-Direct path on the primary handles replication.
                    // So replication log entry is only added by the PRIMARY node via X-Shard-Direct path.

                    return Ok(Json(doc));
                }
                // If X-Shard-Direct header present, fall through to direct insert (replica receiving forwarded data)
            } else {
                // Sharded collection but no coordinator - this is an error state
                tracing::error!("[INSERT] Sharded collection {}/{} but no shard_coordinator available!", db_name, coll_name);
                return Err(DbError::InternalError("Sharded collection requires ShardCoordinator".to_string()));
            }
        }
    }

    // Only reach here for:
    // 1. Non-sharded collections
    // 2. Sharded with X-Shard-Direct header (PRIMARY receiving forwarded insert)
    let doc = collection.insert(data)?;

    // Add to replication log ONLY for non-sharded collections
    // Physical shard collections are partitioned across the cluster - do NOT replicate them
    // to all nodes (that would defeat the purpose of sharding for horizontal scaling)
    let is_shard = is_physical_shard_collection(&coll_name);
    if !is_shard {
        if let Some(ref log) = state.replication_log {
            let entry = LogEntry {
                sequence: 0,
                node_id: "".to_string(),
                database: db_name.clone(),
                collection: coll_name.clone(),
                operation: Operation::Insert,
                key: doc.key.clone(),
                data: serde_json::to_vec(&doc.to_value()).ok(),
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            };
            let _ = log.append(entry);
        }
    }

    Ok(Json(doc.to_value()))
}

/// Batch insert endpoint for internal shard forwarding
/// Accepts an array of documents and inserts them all in one request
pub async fn insert_documents_batch(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(documents): Json<Vec<Value>>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // This is always a direct shard operation (internal API)
    // X-Shard-Direct should be required
    if !headers.contains_key("X-Shard-Direct") {
        return Err(DbError::BadRequest("Batch endpoint requires X-Shard-Direct header".to_string()));
    }

    // Use upsert for physical shard collections (prevents duplicates during resharding)
    // Physical shards have names like "users_s0", "users_s1", etc.
    let is_physical_shard = coll_name.contains("_s") && coll_name.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false);

    let insert_count = if is_physical_shard {
        // Convert documents to (key, doc) pairs for upsert
        let keyed_docs: Vec<(String, Value)> = documents.iter().map(|doc| {
            let key = doc.get("_key")
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .to_string();
            (key, doc.clone())
        }).filter(|(key, _)| !key.is_empty())
        .collect();

        collection.upsert_batch(keyed_docs)?
    } else {
        collection.insert_batch(documents.clone())?.len()
    };

    // NOTE: Do NOT log to replication log for sharded data!
    // This endpoint is for internal shard operations (X-Shard-Direct).
    // Each node only stores its assigned shards - data is partitioned across the cluster.

    // Forward to replica nodes if this is a primary shard
    // Parse shard ID from collection name (e.g., "users_s0" -> shard 0)
    // IMPORTANT: Skip replica forwarding during migrations to prevent duplication
    let is_migration = headers.contains_key("X-Migration");
    if is_migration {
        tracing::debug!("BATCH: Skipping replica forwarding - migration operation for {}/{}", db_name, coll_name);
    } else if let Some(ref coordinator) = state.shard_coordinator {
        // Check if coordinator is currently rebalancing - skip replica forwarding during resharding
        // to prevent timeouts and deadlocks
        let is_rebalancing = coordinator.is_rebalancing();
        if is_rebalancing {
            tracing::debug!("BATCH: Skipping replica forwarding during rebalancing for {}/{}", db_name, coll_name);
        } else {
            // Extract base collection name and shard ID
            if let Some(idx) = coll_name.rfind("_s") {
                let base_coll = &coll_name[..idx];
                if let Ok(shard_id) = coll_name[idx+2..].parse::<u16>() {
                    // Get shard table to find replica nodes
                    if let Some(table) = coordinator.get_shard_table(&db_name, base_coll) {
                        if let Some(assignment) = table.assignments.get(&shard_id) {
                            if !assignment.replica_nodes.is_empty() {
                                // Forward to replicas in parallel
                                let client = reqwest::Client::new();
                                let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();

                                if let Some(ref cluster_manager) = state.cluster_manager {
                                    let mut futures = Vec::new();

                                    for replica_node in &assignment.replica_nodes {
                                        if let Some(addr) = cluster_manager.get_node_api_address(replica_node) {
                                            let url = format!("http://{}/_api/database/{}/document/{}/_replica", addr, db_name, coll_name);
                                            tracing::debug!("REPLICA FWD: Forwarding {} docs to replica {} at {}", documents.len(), replica_node, addr);

                                            let client = client.clone();
                                            let secret = secret.clone();
                                        let docs = documents.clone();

                                        let future = async move {
                                            let _ = tokio::time::timeout(
                                                std::time::Duration::from_secs(10), // 10 second timeout for replicas
                                                client.post(&url)
                                                    .header("X-Shard-Direct", "true")
                                                    .header("X-Cluster-Secret", &secret)
                                                    .json(&docs)
                                                    .send()
                                            ).await;
                                        };
                                        futures.push(future);
                                    }
                                }

                                // Fire and forget - don't wait for replicas
                                tokio::spawn(async move {
                                    futures::future::join_all(futures).await;
                                });
                            }
                        }
                    }
                }
            }
        }
        }
    }

    Ok(Json(serde_json::json!({
        "inserted": insert_count,
        "success": true
    })))
}

/// Replica insert endpoint - stores documents without further forwarding
/// This is called by primary nodes to replicate data to their replicas
pub async fn insert_documents_replica(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(documents): Json<Vec<Value>>,
) -> Result<Json<Value>, DbError> {
    // Require X-Shard-Direct header
    if !headers.contains_key("X-Shard-Direct") {
        return Err(DbError::BadRequest("Replica endpoint requires X-Shard-Direct header".to_string()));
    }

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Use upsert to prevent duplicates (replicas may already have some data)
    // Convert documents to (key, doc) pairs for upsert
    let keyed_docs: Vec<(String, Value)> = documents.iter().map(|doc| {
        let key = doc.get("_key")
            .and_then(|k| k.as_str())
            .unwrap_or("")
            .to_string();
        (key, doc.clone())
    }).filter(|(key, _)| !key.is_empty())
    .collect();

    let insert_count = collection.upsert_batch(keyed_docs)?;

    tracing::debug!("REPLICA: Stored {} docs for {}/{}", insert_count, db_name, coll_name);

    Ok(Json(serde_json::json!({
        "inserted": insert_count,
        "success": true
    })))
}

/// Verify that documents exist in a collection
/// Used by migration to confirm documents arrived before deleting from source
/// POST /_api/database/{db}/document/{coll}/_verify
/// Body: { "keys": ["key1", "key2", ...] }
/// Returns: { "found": ["key1"], "missing": ["key2"], "total_checked": 2 }
pub async fn verify_documents_exist(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(request): Json<serde_json::Value>,
) -> Result<Json<Value>, DbError> {
    let keys = request.get("keys")
        .and_then(|k| k.as_array())
        .ok_or_else(|| DbError::BadRequest("Missing 'keys' array in request body".to_string()))?;

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let mut found: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();

    for key_value in keys {
        if let Some(key) = key_value.as_str() {
            match collection.get(key) {
                Ok(_) => found.push(key.to_string()),
                Err(_) => missing.push(key.to_string()),
            }
        }
    }

    let total_checked = found.len() + missing.len();
    tracing::debug!("VERIFY: Checked {} docs in {}/{}: {} found, {} missing", 
        total_checked, db_name, coll_name, found.len(), missing.len());

    Ok(Json(serde_json::json!({
        "found": found,
        "missing": missing,
        "total_checked": total_checked
    })))
}

/// Copy shard data from a source node (used for healing)
#[derive(Debug, Deserialize)]
pub struct CopyShardRequest {
    source_address: String,
}

pub async fn copy_shard_data(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(request): Json<CopyShardRequest>,
) -> Result<Json<Value>, DbError> {
    tracing::info!("COPY_SHARD: Copying {}/{} from {}", db_name, coll_name, request.source_address);

    // Step 1: Check Source Count using Metadata API
    let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
    let client = reqwest::Client::new();

    // Get doc count first to avoid massive transfer if already in sync
    let meta_url = format!("http://{}/_api/database/{}/collection/{}", request.source_address, db_name, coll_name);
    let meta_res = client.get(&meta_url)
        .header("X-Cluster-Secret", &secret)
        .header("X-Shard-Direct", "true")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let mut source_count = 0;
    let mut check_count = false;

    if let Ok(res) = meta_res {
         if res.status().is_success() {
             if let Ok(json) = res.json::<serde_json::Value>().await {
                 if let Some(c) = json.get("count").and_then(|v| v.as_u64()) {
                     source_count = c as usize;
                     check_count = true;
                 }
             }
         }
    }

    // Ensure collection exists locally
    let database = state.storage.get_database(&db_name)?;
    let collection = match database.get_collection(&coll_name) {
        Ok(c) => c,
        Err(_) => {
            database.create_collection(coll_name.clone(), None)?;
            database.get_collection(&coll_name)?
        }
    };

    // Skip if in sync (count matches)
    if check_count {
        let local_count = collection.count();
        if local_count == source_count {
             // Already in sync
             tracing::info!("COPY_SHARD: Skipping sync for {}/{} (Count match: {})", db_name, coll_name, local_count);
             return Ok(Json(serde_json::json!({
                "copied": 0,
                "success": true,
                "skipped": true
            })));
        }
        tracing::info!("COPY_SHARD: Count mismatch for {}/{} (Local: {}, Source: {}). Truncating before sync.", db_name, coll_name, local_count, source_count);
        let _ = collection.truncate();
    }

    // Query all documents from source shard
    let url = format!("http://{}/_api/database/{}/cursor", request.source_address, db_name);
    let query = format!("FOR doc IN {} RETURN doc", coll_name);
    // Reuse secret from above or fetch again
    // let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
    // We already have 'secret' and 'client' in scope from earlier meta check block

    let res = client.post(&url)
        .header("X-Cluster-Secret", &secret)
        .json(&serde_json::json!({ "query": query }))
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| DbError::InternalError(format!("Request failed: {}", e)))?;

    if !res.status().is_success() {
        let status = res.status();
        let body_text = res.text().await.unwrap_or_else(|_| "Could not read error body".to_string());
        tracing::error!("COPY_SHARD: Source query failed. Status: {}, Body: {}", status, body_text);
        return Err(DbError::InternalError(format!("Source query failed: {}. Body: {}", status, body_text)));
    }

    let body: serde_json::Value = res.json().await
        .map_err(|e| DbError::InternalError(format!("Parse failed: {}", e)))?;

    let docs = body.get("result")
        .and_then(|r| r.as_array())
        .ok_or_else(|| DbError::InternalError("No result array".to_string()))?;


    // Use upsert to prevent duplicates (shard may already have some data)
    let keyed_docs: Vec<(String, serde_json::Value)> = docs.iter().map(|doc| {
        let key = doc.get("_key")
            .and_then(|k| k.as_str())
            .unwrap_or("")
            .to_string();
        (key, doc.clone())
    }).filter(|(key, _)| !key.is_empty())
    .collect();
    let count = keyed_docs.len();
    collection.upsert_batch(keyed_docs)?;

    tracing::info!("COPY_SHARD: Copied {} docs to {}/{}", count, db_name, coll_name);

    Ok(Json(serde_json::json!({
        "copied": count,
        "success": true
    })))
}

pub async fn get_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<ApiResponse<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check for sharding
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                let doc = coordinator.get(
                    &db_name,
                    &coll_name,
                    &key
                ).await?;

                let mut doc_value = doc;
                let replicas = coordinator.get_replicas(&key, &shard_config);
                if let Value::Object(ref mut map) = doc_value {
                    map.insert("_replicas".to_string(), serde_json::json!(replicas));
                }

                return Ok(ApiResponse::new(doc_value, &headers));
            }
        }
    }

    let doc = collection.get(&key)?;
    Ok(ApiResponse::new(doc.to_value(), &headers))
}

pub async fn update_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
    headers: HeaderMap,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
    Json(mut data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check for upsert query param
    let upsert = params.get("upsert").map(|v| v == "true").unwrap_or(false);

    // Check for transaction context
    if let Some(tx_id) = get_transaction_id(&headers) {
        let tx_manager = state.storage.transaction_manager()?;
        let tx_arc = tx_manager.get(tx_id)?;
        let mut tx = tx_arc.write().map_err(|_| DbError::InternalError("Transaction lock poisoned".into()))?;
        let wal = tx_manager.wal();

        let doc = collection.update_tx(&mut tx, wal, &key, data)?;
        return Ok(Json(doc.to_value()));
    }

    // Check for sharding
    // If sharded and we have a coordinator, use it
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                // Check for direct shard access
                if !headers.contains_key("X-Shard-Direct") {
                    let doc = coordinator.update(
                        &db_name,
                        &coll_name,
                        &shard_config,
                        &key,
                        data
                    ).await?;
                    return Ok(Json(doc));
                }
            }
        }
    }

    // Try update, or insert if upsert=true and document not found
    let doc = match collection.update(&key, data.clone()) {
        Ok(doc) => doc,
        Err(DbError::DocumentNotFound(_)) if upsert => {
            // Ensure _key is set for insert
            if let Value::Object(ref mut obj) = data {
                obj.insert("_key".to_string(), Value::String(key.clone()));
            }
            collection.insert(data)?
        }
        Err(e) => return Err(e),
    };

    // Record to replication log ONLY for non-sharded collections
    // Physical shard collections are partitioned across the cluster - do NOT replicate them
    // to all nodes (that would defeat the purpose of sharding for horizontal scaling)
    let is_shard = is_physical_shard_collection(&coll_name);
    let is_sharded_logical = collection.get_shard_config().is_some();
    if !is_shard && !is_sharded_logical {
        if let Some(ref log) = state.replication_log {
            let entry = LogEntry {
                sequence: 0,
                node_id: "".to_string(),
                database: db_name.clone(),
                collection: coll_name.clone(),
                operation: Operation::Update,
                key: doc.key.clone(),
                data: serde_json::to_vec(&doc.to_value()).ok(),
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            };
            let _ = log.append(entry);
        }
    }

    Ok(Json(doc.to_value()))
}

pub async fn delete_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, DbError> {
    // Protect system collections from direct document deletion
    if is_protected_collection(&db_name, &coll_name) {
        return Err(DbError::BadRequest(format!("Cannot delete documents from protected collection: {}", coll_name)));
    }

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check for transaction context
    if let Some(tx_id) = get_transaction_id(&headers) {
        let tx_manager = state.storage.transaction_manager()?;
        let tx_arc = tx_manager.get(tx_id)?;
        let mut tx = tx_arc.write().map_err(|_| DbError::InternalError("Transaction lock poisoned".into()))?;
        let wal = tx_manager.wal();

        collection.delete_tx(&mut tx, wal, &key)?;
        return Ok(StatusCode::NO_CONTENT);
    }

    // Check for sharding
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                if !headers.contains_key("X-Shard-Direct") {
                    coordinator.delete(
                        &db_name,
                        &coll_name,
                        &shard_config,
                        &key
                    ).await?;
                    return Ok(StatusCode::NO_CONTENT);
                }
            }
        }
    }

    collection.delete(&key)?;

    // If this is a blob collection, trigger compaction to reclaim space from deleted chunks immediately
    if collection.get_type() == "blob" {
        tracing::info!("Compacting blob collection {}/{} after deletion of {}", db_name, coll_name, key);
        collection.compact();
    }

    // Record to replication log ONLY for non-sharded collections
    // Physical shard collections are partitioned across the cluster - do NOT replicate them
    // to all nodes (that would defeat the purpose of sharding for horizontal scaling)
    let is_shard = is_physical_shard_collection(&coll_name);
    let is_sharded_logical = collection.get_shard_config().is_some();
    if !is_shard && !is_sharded_logical {
        if let Some(ref log) = state.replication_log {
            let entry = LogEntry {
                sequence: 0,
                node_id: state.cluster_manager.as_ref().map(|m| m.local_node_id()).unwrap_or_else(|| "".to_string()),
                database: db_name.clone(),
                collection: coll_name.clone(),
                operation: Operation::Delete,
                key: key.clone(),
                data: None,
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            };
            let _ = log.append(entry);
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

// ==================== Query Handlers ====================

pub async fn execute_query(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ExecuteQueryRequest>,
) -> Result<ApiResponse<ExecuteQueryResponse>, DbError> {
    // Check for transaction context
    if let Some(tx_id) = get_transaction_id(&headers) {
        // Execute transactional SDBQL query
        use crate::sdbql::ast::BodyClause;

        let query = parse(&req.query)?;

        // Get transaction manager
        let tx_manager = state.storage.transaction_manager()?;
        let tx_arc = tx_manager.get(tx_id)?;
        let mut tx = tx_arc.write().map_err(|_| DbError::InternalError("Transaction lock poisoned".into()))?;
        let wal = tx_manager.wal();

        // Check if query contains mutation operations
        let has_mutations = query.body_clauses.iter().any(|clause| {
            matches!(clause, BodyClause::Insert(_) | BodyClause::Update(_) | BodyClause::Remove(_))
        });

        if !has_mutations {
            // No mutations - just execute normally (read operations)
            // No mutations - just execute normally (read operations)
            let executor = if req.bind_vars.is_empty() {
                QueryExecutor::with_database(&state.storage, db_name)
            } else {
                QueryExecutor::with_database_and_bind_vars(&state.storage, db_name, req.bind_vars)
            };

            let results = executor.execute(&query)?;
            return Ok(ApiResponse::new(ExecuteQueryResponse {
                result: results.clone(),
                count: results.len(),
                has_more: false,
                id: None,
                cached: false,
                execution_time_ms: 0.0,
            }, &headers));
        }

        // For mutation operations, execute transactionally
        let executor = if req.bind_vars.is_empty() {
            QueryExecutor::with_database(&state.storage, db_name.clone())
        } else {
            QueryExecutor::with_database_and_bind_vars(&state.storage, db_name.clone(), req.bind_vars.clone())
        };

        // Execute body clauses manually to intercept mutations
        let mut initial_bindings = std::collections::HashMap::new();

        // Merge bind variables
        for (key, value) in &req.bind_vars {
            initial_bindings.insert(format!("@{}", key), value.clone());
        }

        // Process LET clauses
        for let_clause in &query.let_clauses {
            let value = executor.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
            initial_bindings.insert(let_clause.variable.clone(), value);
        }

        let mut rows: Vec<std::collections::HashMap<String, Value>> = vec![initial_bindings.clone()];
        let mut mutation_count = 0;

        // Process body clauses in order
        for clause in &query.body_clauses {
            match clause {
                BodyClause::For(for_clause) => {
                    let mut new_rows = Vec::new();
                    for ctx in &rows {
                        let docs = if let Some(ref expr) = for_clause.source_expression {
                            let value = executor.evaluate_expr_with_context(expr, ctx)?;
                            match value {
                                Value::Array(arr) => arr,
                                other => vec![other],
                            }
                        } else {
                            let source_name = for_clause.source_variable.as_ref().unwrap_or(&for_clause.collection);
                            if let Some(value) = ctx.get(source_name) {
                                match value {
                                    Value::Array(arr) => arr.clone(),
                                    other => vec![other.clone()],
                                }
                            } else {
                                // Scan collection - check if sharded
                                let full_coll_name = format!("{}:{}", db_name, for_clause.collection);
                                let collection = state.storage.get_collection(&full_coll_name)?;
                                let shard_config = collection.get_shard_config();

                                if let (Some(config), Some(coordinator)) = (shard_config, &state.shard_coordinator) {
                                    // Sharded collection - use scatter-gather
                                    // Execute async operation in blocking context
                                    let coordinator_clone = coordinator.clone();
                                    let db_name_owned = db_name.to_string();
                                    let coll_name_owned = for_clause.collection.clone();
                                    let config_clone = config.clone();

                                    match tokio::task::block_in_place(|| {
                                        tokio::runtime::Handle::current().block_on(async {
                                            coordinator_clone.scan_all_shards(&db_name_owned, &coll_name_owned, &config_clone).await
                                        })
                                    }) {
                                        Ok(docs) => docs.into_iter().map(|d| d.to_value()).collect(),
                                        Err(e) => {
                                            eprintln!("Scatter-gather failed: {:?}, using local shards only", e);
                                            collection.scan(None).into_iter().map(|d| d.to_value()).collect()
                                        }
                                    }
                                } else {
                                    // Non-sharded or no coordinator - local scan
                                    collection.scan(None).into_iter().map(|d| d.to_value()).collect()
                                }
                            }
                        };

                        for doc in docs {
                            let mut new_ctx = ctx.clone();
                            new_ctx.insert(for_clause.variable.clone(), doc);
                            new_rows.push(new_ctx);
                        }
                    }
                    rows = new_rows;
                }
                BodyClause::Let(let_clause) => {
                    for ctx in &mut rows {
                        let value = executor.evaluate_expr_with_context(&let_clause.expression, ctx)?;
                        ctx.insert(let_clause.variable.clone(), value);
                    }
                }
                BodyClause::Filter(filter_clause) => {
                    rows.retain(|ctx| {
                        executor.evaluate_filter_with_context(&filter_clause.expression, ctx).unwrap_or(false)
                    });
                }
                BodyClause::Insert(insert_clause) => {
                    let full_coll_name = format!("{}:{}", db_name, insert_clause.collection);
                    let collection = state.storage.get_collection(&full_coll_name)?;

                    for ctx in &rows {
                        let doc_value = executor.evaluate_expr_with_context(&insert_clause.document, ctx)?;
                        collection.insert_tx(&mut tx, wal, doc_value)?;
                        mutation_count += 1;
                    }
                }
                BodyClause::Update(update_clause) => {
                    let full_coll_name = format!("{}:{}", db_name, update_clause.collection);
                    let collection = state.storage.get_collection(&full_coll_name)?;

                    for ctx in &rows {
                        let selector_value = executor.evaluate_expr_with_context(&update_clause.selector, ctx)?;
                        let key = match &selector_value {
                            Value::String(s) => s.clone(),
                            Value::Object(obj) => obj.get("_key")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .ok_or_else(|| DbError::ExecutionError(
                                    "UPDATE: selector object must have a _key field".to_string()
                                ))?,
                            _ => return Err(DbError::ExecutionError(
                                "UPDATE: selector must be a string key or an object with _key field".to_string()
                            )),
                        };

                        let changes_value = executor.evaluate_expr_with_context(&update_clause.changes, ctx)?;
                        collection.update_tx(&mut tx, wal, &key, changes_value)?;
                        mutation_count += 1;
                    }
                }
                BodyClause::Remove(remove_clause) => {
                    let full_coll_name = format!("{}:{}", db_name, remove_clause.collection);
                    let collection = state.storage.get_collection(&full_coll_name)?;

                    for ctx in &rows {
                        let selector_value = executor.evaluate_expr_with_context(&remove_clause.selector, ctx)?;
                        let key = match &selector_value {
                            Value::String(s) => s.clone(),
                            Value::Object(obj) => obj.get("_key")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .ok_or_else(|| DbError::ExecutionError(
                                    "REMOVE: selector object must have a _key field".to_string()
                                ))?,
                            _ => return Err(DbError::ExecutionError(
                                "REMOVE: selector must be a string key or an object with _key field".to_string()
                            )),
                        };

                        collection.delete_tx(&mut tx, wal, &key)?;
                        mutation_count += 1;
                    }
                }
                _ => {}
            }
        }

        // Return mutation result
        return Ok(ApiResponse::new(ExecuteQueryResponse {
            result: vec![serde_json::json!({
                "mutationCount": mutation_count,
                "message": format!("{} operation(s) staged in transaction. Commit to apply changes.", mutation_count)
            })],
            count: 1,
            has_more: false,
            id: None,
            cached: false,
            execution_time_ms: 0.0,
        }, &headers));
    }

    // Non-transactional execution (existing logic)
    let query = parse(&req.query)?;
    let batch_size = req.batch_size;

    // Only use spawn_blocking for potentially long-running queries
    // (mutations or range iterations). Simple reads run directly.
    let (result, execution_time_ms) = if is_long_running_query(&query) {
        let storage = state.storage.clone();
        let bind_vars = req.bind_vars.clone();
        let replication_log = state.replication_log.clone();
        let shard_coordinator = state.shard_coordinator.clone();
        let is_scatter_gather = headers.contains_key("X-Scatter-Gather");

        // Apply timeout to prevent DoS from long-running queries
        match tokio::time::timeout(
            std::time::Duration::from_secs(QUERY_TIMEOUT_SECS),
            tokio::task::spawn_blocking(move || {
                let mut executor = if bind_vars.is_empty() {
                    QueryExecutor::with_database(&storage, db_name)
                } else {
                    QueryExecutor::with_database_and_bind_vars(&storage, db_name, bind_vars)
                };

                // Add replication service for mutation logging
                if let Some(ref log) = replication_log {
                    executor = executor.with_replication(log);
                }

                // Inject shard coordinator for scatter-gather (if not already a sub-query)
                if !is_scatter_gather {
                    if let Some(coord) = shard_coordinator {
                        executor = executor.with_shard_coordinator(coord);
                    }
                }

                let start = std::time::Instant::now();
                let result = executor.execute(&query)?;
                let execution_time_ms = start.elapsed().as_secs_f64() * 1000.0;
                Ok::<_, DbError>((result, execution_time_ms))
            })
        ).await {
            Ok(join_result) => join_result.map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??,
            Err(_) => return Err(DbError::BadRequest(format!("Query execution timeout: exceeded {} seconds", QUERY_TIMEOUT_SECS))),
        }
    } else {
        let mut executor = if req.bind_vars.is_empty() {
            QueryExecutor::with_database(&state.storage, db_name)
        } else {
            QueryExecutor::with_database_and_bind_vars(&state.storage, db_name, req.bind_vars)
        };

        // Add replication service for mutation logging
        if let Some(ref log) = state.replication_log {
            executor = executor.with_replication(log);
        }

        // Inject shard coordinator for scatter-gather (if not already a sub-query)
        if !headers.contains_key("X-Scatter-Gather") {
            if let Some(coordinator) = state.shard_coordinator.clone() {
                executor = executor.with_shard_coordinator(coordinator);
            }
        }

        let start = std::time::Instant::now();
        let result = executor.execute(&query)?;
        let execution_time_ms = start.elapsed().as_secs_f64() * 1000.0;
        (result, execution_time_ms)
    };

    let total_count = result.len();

    if total_count > batch_size {
        let cursor_id = state.cursor_store.store(result, batch_size);
        let (first_batch, has_more) = state
            .cursor_store
            .get_next_batch(&cursor_id)
            .unwrap_or((vec![], false));

        Ok(ApiResponse::new(ExecuteQueryResponse {
            result: first_batch,
            count: total_count,
            has_more,
            id: if has_more { Some(cursor_id) } else { None },
            cached: false,
            execution_time_ms,
        }, &headers))
    } else {
        Ok(ApiResponse::new(ExecuteQueryResponse {
            result,
            count: total_count,
            has_more: false,
            id: None,
            cached: false,
            execution_time_ms,
        }, &headers))
    }
}

pub async fn explain_query(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ExecuteQueryRequest>,
) -> Result<Json<crate::sdbql::QueryExplain>, DbError> {
    let query = parse(&req.query)?;

    // explain() is fast - no need for spawn_blocking
    // explain() is fast - no need for spawn_blocking
    let mut executor = if req.bind_vars.is_empty() {
        QueryExecutor::with_database(&state.storage, db_name)
    } else {
        QueryExecutor::with_database_and_bind_vars(&state.storage, db_name, req.bind_vars)
    };

    // Inject shard coordinator for explain (if not already a sub-query)
    if !headers.contains_key("X-Scatter-Gather") {
        if let Some(coordinator) = state.shard_coordinator.clone() {
            executor = executor.with_shard_coordinator(coordinator);
        }
    }

    let explain = executor.explain(&query)?;

    Ok(Json(explain))
}

// ==================== Cursor Handlers ====================

pub async fn get_next_batch(
    State(state): State<AppState>,
    Path(cursor_id): Path<String>,
) -> Result<Json<ExecuteQueryResponse>, DbError> {
    if let Some((batch, has_more)) = state.cursor_store.get_next_batch(&cursor_id) {
        let count = batch.len();
        Ok(Json(ExecuteQueryResponse {
            result: batch,
            count,
            has_more,
            id: if has_more { Some(cursor_id) } else { None },
            cached: true,
            execution_time_ms: 0.0, // Cached results, no execution time
        }))
    } else {
        Err(DbError::DocumentNotFound(format!(
            "Cursor not found or expired: {}",
            cursor_id
        )))
    }
}

pub async fn delete_cursor(
    State(state): State<AppState>,
    Path(cursor_id): Path<String>,
) -> Result<StatusCode, DbError> {
    if state.cursor_store.delete(&cursor_id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(DbError::DocumentNotFound(format!(
            "Cursor not found: {}",
            cursor_id
        )))
    }
}

// ==================== Index Handlers ====================

#[derive(Debug, Deserialize)]
pub struct CreateIndexRequest {
    pub name: String,
    pub field: Option<String>,
    pub fields: Option<Vec<String>>,
    #[serde(rename = "type", default = "default_index_type")]
    pub index_type: String,
    #[serde(default)]
    pub unique: bool,
}

fn default_index_type() -> String {
    "persistent".to_string()
}

#[derive(Debug, Serialize)]
pub struct CreateIndexResponse {
    pub name: String,
    pub field: String,
    pub fields: Vec<String>,
    #[serde(rename = "type")]
    pub index_type: IndexType,
    pub unique: bool,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ListIndexesResponse {
    pub indexes: Vec<IndexStats>,
}

pub async fn create_index(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<CreateIndexRequest>,
) -> Result<Json<CreateIndexResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let fields = if let Some(fields) = req.fields {
        fields
    } else if let Some(field) = req.field {
        vec![field]
    } else {
        return Err(DbError::BadRequest(
            "One of 'field' or 'fields' must be provided".to_string(),
        ));
    };

    let index_type = match req.index_type.to_lowercase().as_str() {
        "hash" => IndexType::Hash,
        "persistent" | "skiplist" | "btree" => IndexType::Persistent,
        "fulltext" => IndexType::Fulltext,
        _ => {
            return Err(DbError::InvalidDocument(format!(
                "Unknown index type: {}",
                req.index_type
            )))
        }
    };

    match index_type {
        IndexType::Fulltext => {
            collection.create_fulltext_index(
                req.name.clone(),
                fields.clone(),
                None, // Use default min_length
            )?;
        }
        _ => {
            collection.create_index(
                req.name.clone(),
                fields.clone(),
                index_type.clone(),
                req.unique,
            )?;
        }
    }

    Ok(Json(CreateIndexResponse {
        name: req.name,
        field: fields.first().cloned().unwrap_or_default(),
        fields,
        index_type,
        unique: req.unique,
        status: "created".to_string(),
    }))
}

pub async fn list_indexes(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<ListIndexesResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let indexes = collection.list_indexes();
    Ok(Json(ListIndexesResponse { indexes }))
}

pub async fn rebuild_indexes(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Run in blocking task since this can be slow for large collections
    let coll = collection.clone();
    let count = tokio::task::spawn_blocking(move || coll.rebuild_all_indexes())
        .await
        .map_err(|e| DbError::InternalError(format!("Task error: {}", e)))??;

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "documents_indexed": count,
        "status": "rebuilt"
    })))
}

pub async fn delete_index(
    State(state): State<AppState>,
    Path((db_name, coll_name, index_name)): Path<(String, String, String)>,
) -> Result<StatusCode, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    
    // Try dropping as standard index
    if collection.drop_index(&index_name).is_ok() {
        return Ok(StatusCode::NO_CONTENT);
    }
    
    // Try dropping as fulltext index
    if collection.drop_fulltext_index(&index_name).is_ok() {
        return Ok(StatusCode::NO_CONTENT);
    }

    // Try dropping as geo index
    if collection.drop_geo_index(&index_name).is_ok() {
        return Ok(StatusCode::NO_CONTENT);
    }

    // Try dropping as TTL index
    if collection.drop_ttl_index(&index_name).is_ok() {
        return Ok(StatusCode::NO_CONTENT);
    }

    // If all attempts failed, it genuinely doesn't exist
    Err(DbError::InvalidDocument(format!(
        "Index '{}' not found",
        index_name
    )))
}

// ==================== Geo Index Handlers ====================

#[derive(Debug, Deserialize)]
pub struct CreateGeoIndexRequest {
    pub name: String,
    pub field: String,
}

#[derive(Debug, Serialize)]
pub struct CreateGeoIndexResponse {
    pub name: String,
    pub field: String,
    #[serde(rename = "type")]
    pub index_type: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ListGeoIndexesResponse {
    pub indexes: Vec<GeoIndexStats>,
}

#[derive(Debug, Deserialize)]
pub struct GeoNearRequest {
    pub lat: f64,
    pub lon: f64,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    10
}

#[derive(Debug, Deserialize)]
pub struct GeoWithinRequest {
    pub lat: f64,
    pub lon: f64,
    pub radius: f64,
}

#[derive(Debug, Serialize)]
pub struct GeoResult {
    pub document: Value,
    pub distance: f64,
}

#[derive(Debug, Serialize)]
pub struct GeoQueryResponse {
    pub results: Vec<GeoResult>,
    pub count: usize,
}

pub async fn create_geo_index(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<CreateGeoIndexRequest>,
) -> Result<Json<CreateGeoIndexResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    collection.create_geo_index(req.name.clone(), req.field.clone())?;

    Ok(Json(CreateGeoIndexResponse {
        name: req.name,
        field: req.field,
        index_type: "geo".to_string(),
        status: "created".to_string(),
    }))
}

pub async fn list_geo_indexes(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<ListGeoIndexesResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let indexes = collection.list_geo_indexes();
    Ok(Json(ListGeoIndexesResponse { indexes }))
}

pub async fn delete_geo_index(
    State(state): State<AppState>,
    Path((db_name, coll_name, index_name)): Path<(String, String, String)>,
) -> Result<StatusCode, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    collection.drop_geo_index(&index_name)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn geo_near(
    State(state): State<AppState>,
    Path((db_name, coll_name, field)): Path<(String, String, String)>,
    Json(req): Json<GeoNearRequest>,
) -> Result<Json<GeoQueryResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let results = collection
        .geo_near(&field, req.lat, req.lon, req.limit)
        .ok_or_else(|| {
            DbError::InvalidDocument(format!("No geo index found on field '{}'", field))
        })?;

    let geo_results: Vec<GeoResult> = results
        .into_iter()
        .map(|(doc, dist)| GeoResult {
            document: doc.to_value(),
            distance: dist,
        })
        .collect();

    let count = geo_results.len();

    Ok(Json(GeoQueryResponse {
        results: geo_results,
        count,
    }))
}

pub async fn geo_within(
    State(state): State<AppState>,
    Path((db_name, coll_name, field)): Path<(String, String, String)>,
    Json(req): Json<GeoWithinRequest>,
) -> Result<Json<GeoQueryResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let results = collection
        .geo_within(&field, req.lat, req.lon, req.radius)
        .ok_or_else(|| {
            DbError::InvalidDocument(format!("No geo index found on field '{}'", field))
        })?;

    let geo_results: Vec<GeoResult> = results
        .into_iter()
        .map(|(doc, dist)| GeoResult {
            document: doc.to_value(),
            distance: dist,
        })
        .collect();

    let count = geo_results.len();

    Ok(Json(GeoQueryResponse {
        results: geo_results,
        count,
    }))
}

// ==================== TTL Index Handlers ====================

#[derive(Debug, Deserialize)]
pub struct CreateTtlIndexRequest {
    pub name: String,
    pub field: String,
    pub expire_after_seconds: u64,
}

#[derive(Debug, Serialize)]
pub struct CreateTtlIndexResponse {
    pub name: String,
    pub field: String,
    pub expire_after_seconds: u64,
    #[serde(rename = "type")]
    pub index_type: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ListTtlIndexesResponse {
    pub indexes: Vec<TtlIndexStats>,
}

pub async fn create_ttl_index(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<CreateTtlIndexRequest>,
) -> Result<Json<CreateTtlIndexResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    collection.create_ttl_index(req.name.clone(), req.field.clone(), req.expire_after_seconds)?;

    Ok(Json(CreateTtlIndexResponse {
        name: req.name,
        field: req.field,
        expire_after_seconds: req.expire_after_seconds,
        index_type: "ttl".to_string(),
        status: "created".to_string(),
    }))
}

pub async fn list_ttl_indexes(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<ListTtlIndexesResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let indexes = collection.list_ttl_indexes();
    Ok(Json(ListTtlIndexesResponse { indexes }))
}

pub async fn delete_ttl_index(
    State(state): State<AppState>,
    Path((db_name, coll_name, index_name)): Path<(String, String, String)>,
) -> Result<StatusCode, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    collection.drop_ttl_index(&index_name)?;

    Ok(StatusCode::NO_CONTENT)
}

// ==================== Cluster Status ====================

#[derive(Debug, Serialize)]
pub struct PeerStatusResponse {
    pub address: String,
    pub is_connected: bool,
    pub last_seen_secs_ago: u64,
    pub replication_lag: u64,
    pub stats: Option<NodeBasicStats>,
}

#[derive(Debug, Serialize)]
pub struct NodeStats {
    pub database_count: usize,
    pub collection_count: usize,
    pub document_count: u64,
    pub storage_bytes: u64,
    pub uptime_secs: u64,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub cpu_usage_percent: f32,
    pub request_count: u64,
    // New stats
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub system_load_avg: f64,
    pub total_file_count: u64,
    pub total_chunk_count: u64,
    pub total_sst_size: u64,
    pub total_memtable_size: u64,
    pub total_live_size: u64,
}

#[derive(Debug, Serialize)]
pub struct ClusterStatusResponse {
    pub node_id: String,
    pub status: String,
    pub replication_port: u16,
    pub current_sequence: u64,
    pub log_entries: usize,
    pub peers: Vec<PeerStatusResponse>,
    pub data_dir: String,
    pub stats: NodeStats,
}

/// Generate cluster status data (shared between HTTP and WebSocket handlers)
fn generate_cluster_status(state: &AppState, sys: &mut sysinfo::System) -> ClusterStatusResponse {
    use std::sync::atomic::Ordering;

    let node_id = state.storage.node_id().to_string();
    let data_dir = state.storage.data_dir().to_string();

    let replication_port = if let Some(ref manager) = state.cluster_manager {
        let addr = manager.get_local_address();
        addr.split(':')
            .last()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(6746)
    } else {
        state
            .storage
            .cluster_config()
            .map(|c| c.replication_port)
            .unwrap_or(6746)
    };

    // Calculate stats
    let databases = state.storage.list_databases();
    let database_count = databases.len();

    let mut collection_count = 0;
    let mut document_count: u64 = 0;
    let mut total_file_count: u64 = 0;
    let mut total_chunk_count: u64 = 0;
    let mut total_sst_size: u64 = 0;
    let mut total_memtable_size: u64 = 0;
    let mut total_live_size: u64 = 0;

    for db_name in &databases {
        if let Ok(db) = state.storage.get_database(db_name) {
            let coll_names = db.list_collections();
            collection_count += coll_names.len();
            for coll_name in coll_names {
                if let Ok(coll) = db.get_collection(&coll_name) {
                    let stats = coll.stats();
                    document_count += stats.document_count as u64;
                    total_file_count += stats.disk_usage.num_sst_files;
                    total_chunk_count += stats.chunk_count as u64;
                    total_sst_size += stats.disk_usage.sst_files_size;
                    total_memtable_size += stats.disk_usage.memtable_size;
                    total_live_size += stats.disk_usage.live_data_size;
                }
            }
        }
    }

    // Storage size (approximate from data directory)
    let storage_bytes = get_dir_size(&data_dir).unwrap_or(0);

    // Uptime
    let uptime_secs = state.startup_time.elapsed().as_secs();

    // Memory and CPU usage
    sys.refresh_memory();
    let pid = sysinfo::get_current_pid().ok();

    let (memory_used_mb, cpu_usage_percent) = if let Some(p) = pid {
        sys.refresh_process(p);
        sys.process(p)
            .map(|proc| (proc.memory() / (1024 * 1024), proc.cpu_usage()))
            .unwrap_or((0, 0.0))
    } else {
        (0, 0.0)
    };

    let memory_total_mb = sys.total_memory() / (1024 * 1024);

    // Request count
    let request_count = state.request_counter.load(Ordering::Relaxed);

    // Network I/O - use separate Networks struct (sysinfo 0.30 API)
    let networks = sysinfo::Networks::new_with_refreshed_list();
    let mut network_rx_bytes = 0u64;
    let mut network_tx_bytes = 0u64;
    for (_, network) in &networks {
        network_rx_bytes += network.total_received();
        network_tx_bytes += network.total_transmitted();
    }

    // System Load
    let system_load_avg = sysinfo::System::load_average().one;

    let stats = NodeStats {
        database_count,
        collection_count,
        document_count,
        storage_bytes,
        uptime_secs,
        memory_used_mb,
        memory_total_mb,
        cpu_usage_percent,
        request_count,
        network_rx_bytes,
        network_tx_bytes,
        system_load_avg,
        total_file_count,
        total_chunk_count,
        total_sst_size,
        total_memtable_size,
        total_live_size,
    };

    // Get live status from cluster manager and replication log
    if let Some(ref manager) = state.cluster_manager {
        let member_list = manager.state().get_all_members();

        let status = if member_list.iter().any(|m| m.status == crate::cluster::state::NodeStatus::Active && m.node.id != manager.local_node_id()) {
            "cluster".to_string()
        } else if member_list.len() > 1 {
             "cluster-connecting".to_string()
        } else {
             "cluster-ready".to_string()
        };

        let peers: Vec<PeerStatusResponse> = member_list
            .into_iter()
            .filter(|m| m.node.id != manager.local_node_id())
            .map(|m| PeerStatusResponse {
                address: m.node.address,
                is_connected: m.status == crate::cluster::state::NodeStatus::Active,
                last_seen_secs_ago: (chrono::Utc::now().timestamp_millis() as u64 - m.last_heartbeat) / 1000,
                replication_lag: 0, // TODO: track actual lag
                stats: m.stats.clone(),
            })
            .collect();

        let (current_seq, count) = if let Some(log) = &state.replication_log {
            (log.current_sequence(), log.current_sequence())
        } else {
            (0, 0)
        };

        ClusterStatusResponse {
            node_id: manager.local_node_id(),
            status,
            replication_port,
             // TODO: We need to put actual logic based on sequence
            current_sequence: current_seq,
            log_entries: count as usize,
            peers,
            data_dir,
            stats,
        }
    } else {
        ClusterStatusResponse {
            node_id,
            status: "standalone".to_string(),
            replication_port,
            current_sequence: 0,
            log_entries: 0,
            peers: vec![],
            data_dir,
            stats,
        }
    }
}

pub async fn cluster_status(State(state): State<AppState>) -> Json<ClusterStatusResponse> {
    use sysinfo::System;
    // For single HTTP request, we create a new system.
    // Note: CPU usage might be inaccurate (0.0) for single requests without a previous refresh.
    // If accurate CPU is needed on HTTP, we'd need to sleep/refresh, but avoiding blocking is better.
    let mut sys = System::new();
    Json(generate_cluster_status(&state, &mut sys))
}

/// WebSocket handler for real-time cluster status updates
pub async fn cluster_status_ws(
    ws: axum::extract::ws::WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_cluster_ws(socket, state))
}

/// Handle the WebSocket connection for cluster status
async fn handle_cluster_ws(mut socket: axum::extract::ws::WebSocket, state: AppState) {
    use axum::extract::ws::Message;
    use tokio::time::{interval, Duration};

    let mut ticker = interval(Duration::from_secs(1));

    // We use the shared system monitor from AppState to avoid expensive initialization
    // and to ensure CPU usage is calculated correctly (delta since last refresh).

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Generate status using shared logic and persistent sys
                let status = {
                    let mut sys = state.system_monitor.lock().unwrap();
                    generate_cluster_status(&state, &mut *sys)
                };

                let json = match serde_json::to_string(&status) {
                    Ok(j) => j,
                    Err(_) => continue,
                };

                if socket.send(Message::Text(json.into())).await.is_err() {
                    break; // Client disconnected
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        // Respond to ping with pong
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {} // Ignore other messages
                }
            }
        }
    }
}

/// Get the size of a directory in bytes (recursive)
fn get_dir_size(path: &str) -> std::io::Result<u64> {
    let mut size = 0u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            size += get_dir_size(entry.path().to_str().unwrap_or(""))?;
        } else {
            size += metadata.len();
        }
    }
    Ok(size)
}

// ==================== Cluster Info ====================

#[derive(Debug, Serialize)]
pub struct ClusterInfoResponse {
    pub node_id: String,
    pub is_cluster_mode: bool,
    pub cluster_config: Option<ClusterConfigInfo>,
    // System Stats
    pub cpu_usage: f32,
    pub memory_usage: u64,
    pub memory_total: u64,
    pub uptime: u64,
    pub os_name: String,
    pub os_version: String,
    pub hostname: String,
    pub num_cpus: usize,
}

#[derive(Debug, Serialize)]
pub struct ClusterConfigInfo {
    pub node_id: String,
    pub peers: Vec<String>,
    pub replication_port: u16,
}

pub async fn cluster_info(State(state): State<AppState>) -> Json<ClusterInfoResponse> {
    let node_id = state.storage.node_id().to_string();
    let is_cluster_mode = state.storage.is_cluster_mode();

    let cluster_config = state.storage.cluster_config().map(|c| ClusterConfigInfo {
        node_id: c.node_id.clone(),
        peers: c.peers.clone(),
        replication_port: c.replication_port,
    });

    // Collect System Stats
    let (cpu_usage, memory_usage, memory_total, uptime, os_name, os_version, hostname, num_cpus) = {
        let mut sys = state.system_monitor.lock().unwrap();
        
        // Refresh specific stats
        sys.refresh_cpu();
        sys.refresh_memory();

        let cpu = sys.global_cpu_info().cpu_usage();
        let mem_used = sys.used_memory();
        let mem_total = sys.total_memory();
        let up = sysinfo::System::uptime();
        let name = sysinfo::System::name().unwrap_or_else(|| "Unknown".to_string());
        let version = sysinfo::System::kernel_version().unwrap_or_else(|| "Unknown".to_string());
        let host = sysinfo::System::host_name().unwrap_or_else(|| "Unknown".to_string());
        let cores = sys.cpus().len();
        
        (cpu, mem_used, mem_total, up, name, version, host, cores)
    };

    Json(ClusterInfoResponse {
        node_id,
        is_cluster_mode,
        cluster_config,
        cpu_usage,
        memory_usage,
        memory_total,
        uptime,
        os_name,
        os_version,
        hostname,
        num_cpus,
    })
}

// ==================== System Monitoring WebSocket ====================

pub async fn monitor_ws_handler(
    ws: WebSocketUpgrade,
    AxumQuery(params): AxumQuery<AuthParams>,
    State(state): State<AppState>,
) -> Response {
    if let Err(_) = crate::server::auth::AuthService::validate_token(&params.token) {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .expect("Valid status code should not fail")
            .into_response();
    }

    ws.on_upgrade(|socket| handle_monitor_socket(socket, state))
}

async fn handle_monitor_socket(mut socket: WebSocket, state: AppState) {
    use std::sync::atomic::Ordering;

    tracing::info!("Monitor WS: Client connected");

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));

    loop {
        // Wait for next tick
        interval.tick().await;
        // tracing::debug!("Monitor WS: Sending stats");

        // Check if client is still alive (optional ping/pong could go here)
        
        let stats = {
            let mut sys = state.system_monitor.lock().unwrap();
            
            // Refresh specific stats
            sys.refresh_cpu();
            sys.refresh_memory();

            let cpu = sys.global_cpu_info().cpu_usage();
            let mem_used = sys.used_memory();
            let mem_total = sys.total_memory();
            let up = sysinfo::System::uptime();
            let name = sysinfo::System::name().unwrap_or_else(|| "Unknown".to_string());
            let version = sysinfo::System::kernel_version().unwrap_or_else(|| "Unknown".to_string());
            let host = sysinfo::System::host_name().unwrap_or_else(|| "Unknown".to_string());
            let cores = sys.cpus().len();
            
            serde_json::json!({
                "cpu_usage": cpu,
                "memory_usage": mem_used,
                "memory_total": mem_total,
                "uptime": up,
                "os_name": name,
                "os_version": version,
                "hostname": host,
                "num_cpus": cores,
                "pid": std::process::id(),
                "active_scripts": state.script_stats.active_scripts.load(Ordering::Relaxed),
                "active_ws": state.script_stats.active_ws.load(Ordering::Relaxed)
            })
        };

        let msg = match serde_json::to_string(&stats) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if socket.send(Message::Text(msg.into())).await.is_err() {
            // Client disconnected
            break;
        }
    }
}

// ==================== Cluster Remove Node ====================

#[derive(Debug, Deserialize)]
pub struct RemoveNodeRequest {
    /// The address of the node to remove (e.g., "localhost:6775")
    pub node_address: String,
}

#[derive(Debug, Serialize)]
pub struct RemoveNodeResponse {
    pub success: bool,
    pub message: String,
    pub removed_node: String,
    pub remaining_nodes: Vec<String>,
}

/// Remove a node from the cluster and trigger rebalancing
pub async fn cluster_remove_node(
    State(state): State<AppState>,
    Json(req): Json<RemoveNodeRequest>,
) -> Result<Json<RemoveNodeResponse>, DbError> {
    let node_addr = req.node_address;

    // Get the shard coordinator
    let coordinator = state.shard_coordinator.as_ref()
        .ok_or_else(|| DbError::InternalError("Shard coordinator not available - not in cluster mode".to_string()))?;

    // Remove the node and trigger rebalancing
    // Remove the node and trigger rebalancing
    // Remove the node and trigger rebalancing
    coordinator.remove_node(&node_addr).await?;


    // Get remaining nodes
    let remaining = coordinator.get_node_addresses();

    Ok(Json(RemoveNodeResponse {
        success: true,
        message: format!("Node {} removed, rebalancing complete", node_addr),
        removed_node: node_addr,
        remaining_nodes: remaining,
    }))
}

// ==================== Cluster Rebalance ====================

#[derive(Debug, Serialize)]
pub struct RebalanceResponse {
    pub success: bool,
    pub message: String,
}

/// Trigger cluster rebalancing
pub async fn cluster_rebalance(
    State(state): State<AppState>,
) -> Result<Json<RebalanceResponse>, DbError> {
    let coordinator = state.shard_coordinator.as_ref()
        .ok_or_else(|| DbError::InternalError("Shard coordinator not available - not in cluster mode".to_string()))?;

    coordinator.rebalance().await?;


    Ok(Json(RebalanceResponse {
        success: true,
        message: "Rebalancing complete".to_string(),
    }))
}

/// Trigger cleanup of orphaned shard collections on this node
/// Called by cluster broadcast after resharding contraction
pub async fn cluster_cleanup(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<Vec<crate::sharding::coordinator::ShardTable>>>,
) -> Result<Json<serde_json::Value>, DbError> {
    // Verify cluster secret
    let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
    let request_secret = headers
        .get("X-Cluster-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !secret.is_empty() && request_secret != secret {
        return Err(DbError::BadRequest("Invalid cluster secret".to_string()));
    }

    let coordinator = state.shard_coordinator.as_ref()
        .ok_or_else(|| DbError::InternalError("Shard coordinator not available".to_string()))?;

    // Update shard tables if provided
    if let Some(Json(tables)) = body {
        tracing::info!("CLEANUP: Received {} updated shard tables from coordinator", tables.len());
        for table in tables {
            coordinator.update_shard_table_cache(table);
        }
    }

    let cleaned = coordinator.cleanup_orphaned_shards().await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "cleaned": cleaned
    })))
}

/// Handle reshard request for removed shards during contraction
/// Called by the coordinating node to have this node migrate data from a removed shard
#[derive(Debug, Deserialize)]
pub struct ReshardRequest {
    database: String,
    collection: String,
    old_shards: u16,
    new_shards: u16,
    removed_shard_id: u16,
}

pub async fn cluster_reshard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ReshardRequest>,
) -> Result<Json<serde_json::Value>, DbError> {
    // Verify cluster secret
    let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
    let request_secret = headers
        .get("X-Cluster-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !secret.is_empty() && request_secret != secret {
        return Err(DbError::BadRequest("Invalid cluster secret".to_string()));
    }

    let coordinator = state.shard_coordinator.as_ref()
        .ok_or_else(|| DbError::InternalError("Shard coordinator not available".to_string()))?;

    tracing::info!(
        "RESHARD: Processing migration request for removed shard {}_s{} ({} -> {} shards)",
        request.collection, request.removed_shard_id, request.old_shards, request.new_shards
    );

    // Migrate documents from the removed shard to their new locations
    let physical_name = format!("{}_s{}", request.collection, request.removed_shard_id);

    let db = state.storage.get_database(&request.database)?;
    let physical_coll = match db.get_collection(&physical_name) {
        Ok(c) => c,
        Err(_) => {
            tracing::warn!("RESHARD: Physical shard {} not found locally", physical_name);
            return Ok(Json(serde_json::json!({
                "success": true,
                "message": "Shard not found locally",
                "migrated": 0
            })));
        }
    };

    let main_coll = db.get_collection(&request.collection)?;
    let config = main_coll.get_shard_config()
        .ok_or_else(|| DbError::InternalError("Missing shard config".to_string()))?;

    // Get all documents from the removed shard
    let documents = physical_coll.all();
    let total_docs = documents.len();
    tracing::info!("RESHARD: Migrating {} documents from removed shard {}", total_docs, physical_name);

    // Collect all documents with their new shard destinations
    let mut docs_to_move: Vec<(String, serde_json::Value)> = Vec::new();

    for doc in documents {
        let key = doc.key.clone();
        let route_key = if config.shard_key == "_key" {
            key.clone()
        } else {
            key.clone()
        };

        // Route to new shard
        let new_shard_id = crate::sharding::router::ShardRouter::route(&route_key, request.new_shards);

        // Only move if going to a different shard (which it should, since this shard is being removed)
        if new_shard_id != request.removed_shard_id {
            docs_to_move.push((key, doc.to_value()));
        }
    }

    if docs_to_move.is_empty() {
        return Ok(Json(serde_json::json!({
            "success": true,
            "message": "No documents to migrate",
            "migrated": 0
        })));
    }

    // Use upsert to insert into new shards (via coordinator)
    let mut migrated = 0;
    const BATCH_SIZE: usize = 1000;

    for batch in docs_to_move.chunks(BATCH_SIZE) {
        let batch_keyed: Vec<(String, serde_json::Value)> = batch.to_vec();
        // batch_keys removed as unused

        // Use upsert via coordinator
        match coordinator.upsert_batch_to_shards(
            &request.database,
            &request.collection,
            &config,
            batch_keyed
        ).await {
            Ok(successful_keys) => {
                if !successful_keys.is_empty() {
                    // Delete ONLY successfully migrated documents from source
                    let _ = physical_coll.delete_batch(&successful_keys);
                    migrated += successful_keys.len();
                }

                if successful_keys.len() < batch.len() {
                    tracing::warn!("RESHARD: Batch partial success ({}/{}) - kept failed docs in source",
                        successful_keys.len(), batch.len());
                }
            }
            Err(e) => {
                tracing::error!("RESHARD: Batch migration failed: {}", e);
            }
        }
    }

    tracing::info!("RESHARD: Migrated {} documents from removed shard {}", migrated, physical_name);

    Ok(Json(serde_json::json!({
        "success": true,
        "migrated": migrated
    })))
}

// ==================== Real-time Changefeeds ====================

#[derive(Debug, Deserialize)]
pub struct ChangefeedRequest {
    #[serde(rename = "type")]
    pub type_: String,
    pub collection: Option<String>,
    pub database: Option<String>,
    pub key: Option<String>,
    pub local: Option<bool>,
    /// SDBQL query for live_query mode
    pub query: Option<String>,
}

/// WebSocket handler for real-time changefeeds
pub async fn ws_changefeed_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    AxumQuery(params): AxumQuery<AuthParams>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Check for cluster-internal authentication (bypasses normal JWT validation)
    let is_cluster_internal = {
        let cluster_secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
        let provided_secret = headers
            .get("X-Cluster-Secret")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");

        !cluster_secret.is_empty() && cluster_secret == provided_secret
    };

    // If not cluster-internal, validate the JWT token
    if !is_cluster_internal {
        if let Err(_) = crate::server::auth::AuthService::validate_token(&params.token) {
            return Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("Valid status code should not fail")
                .into_response();
        }
    }

    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: AppState) {
    // Wait for subscription message
    if let Some(Ok(msg)) = socket.recv().await {
        if let Message::Text(text) = msg {
            match serde_json::from_str::<ChangefeedRequest>(&text) {
                Ok(req) if req.type_ == "subscribe" => {
                    let db_name = req.database.clone().unwrap_or("_system".to_string());
                    
                    let coll_name = match req.collection.clone() {
                        Some(c) => c,
                        None => {
                            let _ = socket.send(Message::Text(serde_json::json!({
                                "error": "Collection required for subscribe mode"
                            }).to_string().into())).await;
                            return;
                        }
                    };

                    // Try to get collection from specific database or fallback
                    let collection_result = state.storage.get_database(&db_name).and_then(|db| db.get_collection(&coll_name));

                    match collection_result {
                        Ok(collection) => {
                            // Send confirmation
                            let _ = socket.send(Message::Text(serde_json::json!({
                                "type": "subscribed",
                                "collection": coll_name
                            }).to_string().into())).await;

                            // Set up streams vector for aggregation
                            // We use a channel to merge streams because SelectAll requires Unpin which can be tricky with async streams
                            let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::storage::collection::ChangeEvent>(1000);
                            let req_key = req.key.clone();

                            // 1. Subscribe to local logical collection (always useful for metadata or non-sharded)
                            let mut local_rx = collection.change_sender.subscribe();
                            let tx_local = tx.clone();
                            let req_key_local = req_key.clone();

                            tokio::spawn(async move {
                                loop {
                                    match local_rx.recv().await {
                                        Ok(event) => {
                                            if let Some(ref target_key) = req_key_local {
                                                if &event.key != target_key { continue; }
                                            }
                                            if tx_local.send(event).await.is_err() { break; }
                                        }
                                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                        Err(_) => break,
                                    }
                                }
                            });

                            // 2. Subscribe to PHYSICAL SHARDS (if sharded)
                            if let Some(shard_config) = collection.get_shard_config() {
                                if shard_config.num_shards > 0 {
                                    tracing::debug!("[CHANGEFEED] Subscribing to local physical shards for {}/{}", db_name, coll_name);

                                    // Subscribe to all LOCAL physical shards
                                    // Iterate potential shard IDs and check if they exist locally
                                    if let Ok(database) = state.storage.get_database(&db_name) {
                                        for shard_id in 0..shard_config.num_shards {
                                            let physical_name = format!("{}_s{}", coll_name, shard_id);
                                            // Check if this physical shard collection exists locally
                                            if let Ok(physical_coll) = database.get_collection(&physical_name) {
                                                 tracing::debug!("[CHANGEFEED] Found local shard {}, subscribing", physical_name);

                                                 let mut shard_rx = physical_coll.change_sender.subscribe();
                                                 let tx_shard = tx.clone();
                                                 let req_key_shard = req_key.clone();

                                                 tokio::spawn(async move {
                                                    loop {
                                                        match shard_rx.recv().await {
                                                            Ok(event) => {
                                                                if let Some(ref target_key) = req_key_shard {
                                                                    if &event.key != target_key { continue; }
                                                                }
                                                                if tx_shard.send(event).await.is_err() { break; }
                                                            }
                                                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                                            Err(_) => break,
                                                        }
                                                    }
                                                 });
                                            }
                                        }
                                    }
                                }
                            }

                            // 3. Connect to REMOTE nodes for aggregation (unless local_only=true)
                            let is_local_only = req.local.unwrap_or(false);

                            if !is_local_only {
                                if let Some(shard_config) = collection.get_shard_config() {
                                    if let Some(coordinator) = &state.shard_coordinator {
                                        let my_addr = coordinator.my_address();
                                        // Get ALL nodes relevant for this collection
                                        let all_nodes = coordinator.get_collection_nodes(&shard_config);

                                        // Filter unique nodes that are NOT self
                                        let mut remote_nodes = std::collections::HashSet::new();
                                        for node_addr in all_nodes {
                                            if node_addr != my_addr {
                                                remote_nodes.insert(node_addr);
                                            }
                                        }

                                        for node_addr in remote_nodes {
                                            // Spawn remote listener
                                            let tx_remote = tx.clone();
                                            let db_name_remote = db_name.clone();
                                            let coll_name_remote = coll_name.clone();
                                            let node_addr_clone = node_addr.clone();

                                            tokio::spawn(async move {
                                                use crate::cluster::ClusterWebsocketClient;

                                                tracing::debug!("[CHANGEFEED] connecting to remote {}", node_addr_clone);
                                                // Pass local_only=true to prevent infinite recursion
                                                match ClusterWebsocketClient::connect(
                                                    &node_addr_clone,
                                                    &db_name_remote,
                                                    &coll_name_remote,
                                                    true // <--- IMPORTANT: Ask for local events only
                                                ).await {
                                                    Ok(stream) => {
                                                        tokio::pin!(stream);
                                                        while let Some(result) = stream.next().await {
                                                            match result {
                                                                Ok(event) => {
                                                                    if tx_remote.send(event).await.is_err() {
                                                                        break;
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    tracing::warn!("[CHANGEFEED] Remote stream error from {}: {}", node_addr_clone, e);
                                                                    break;
                                                                }
                                                            }
                                                        }
                                                    }
                                                    Err(e) => {
                                                        tracing::warn!("[CHANGEFEED] Failed to connect to remote {}: {}", node_addr_clone, e);
                                                    }
                                                }
                                            });
                                        }
                                    }
                                }
                            }

                            // Forward aggregated events to client
                            // Heartbeat: Send a Ping every 30 seconds to keep the connection alive
                            let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(30));
                            loop {
                                tokio::select! {
                                    // Heartbeat tick: send a Ping to keep the connection alive
                                    _ = heartbeat_interval.tick() => {
                                        if socket.send(Message::Ping(vec![].into())).await.is_err() {
                                            tracing::debug!("[CHANGEFEED] Failed to send ping, closing connection");
                                            break;
                                        }
                                    }
                                    // Received event from aggregator
                                    Some(event) = rx.recv() => {
                                        // Note: Local events were already filtered by key in the spawned task.
                                        // Remote events should eventually be filtered too, but for now we filter here again just in case
                                        // to be safe (though remote subscription should filter ideally, current impl connects to stream).
                                        // Actually ClusterWebsocketClient subscription sends a filter?
                                        // The current ClusterWebsocketClient connect() sends "subscribe" without key filter.
                                        // So we MUST filter here.
                                        if let Some(ref target_key) = req.key {
                                            if &event.key != target_key {
                                                continue;
                                            }
                                        }

                                        // Optimized payload: send only metadata, not full data
                                        let payload = serde_json::json!({
                                            "type": event.type_,
                                            "key": event.key,
                                            "id": format!("{}/{}", coll_name, event.key)
                                        });

                                        if let Ok(json) = serde_json::to_string(&payload) {
                                            if socket.send(Message::Text(json.into())).await.is_err() {
                                                break;
                                            }
                                        }
                                    }
                                    // Handle incoming messages (e.g. close, pong)
                                    Some(msg) = socket.recv() => {
                                        match msg {
                                            Ok(Message::Close(_)) | Err(_) => break,
                                            Ok(Message::Pong(_)) => { /* heartbeat acknowledged */ }
                                            _ => {} // Ignore other messages
                                        }
                                    }
                                    else => break,
                                }
                            }
                        },
                        Err(_) => {
                             let _ = socket.send(Message::Text(serde_json::json!({
                                 "error": "Collection not found"
                             }).to_string().into())).await;
                        }
                    }
                }
                Ok(req) if req.type_ == "live_query" => {
                    if let Some(query_str) = req.query {
                         let db_name = req.database.clone().unwrap_or("_system".to_string());
                         
                         // 1. Parse query to identify dependencies
                         match crate::sdbql::parser::parse(&query_str) {
                            Ok(query) => {
                                // Extract all referenced collections from FOR clauses
                                let mut dependencies = std::collections::HashSet::new();
                                for clause in &query.for_clauses {
                                    dependencies.insert(clause.collection.clone());
                                }
                                
                                if dependencies.is_empty() {
                                     let _ = socket.send(Message::Text(serde_json::json!({
                                         "error": "Live query must reference at least one collection"
                                     }).to_string().into())).await;
                                     return;
                                }

                                // Send confirmation
                                let _ = socket.send(Message::Text(serde_json::json!({
                                    "type": "subscribed",
                                    "mode": "live_query",
                                    "collections": dependencies
                                }).to_string().into())).await;

                                // 2. Setup aggregated change channel
                                let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::storage::collection::ChangeEvent>(1000);

                                // 3. Subscribe to ALL dependencies
                                // We reuse the logic from standard changefeed but apply it to multiple collections
                                for coll_name in &dependencies {
                                    let coll_name = coll_name.clone(); // Clone for closure
                                    
                                    // Try to get collection
                                    let collection_result = state.storage.get_database(&db_name).and_then(|db| db.get_collection(&coll_name));
                                    
                                    if let Ok(collection) = collection_result {
                                        // A. Subscribe to local logical
                                        let mut local_rx = collection.change_sender.subscribe();
                                        let tx_local = tx.clone();
                                        tokio::spawn(async move {
                                            loop {
                                                match local_rx.recv().await {
                                                    Ok(event) => { if tx_local.send(event).await.is_err() { break; } },
                                                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                                    Err(_) => break,
                                                }
                                            }
                                        });

                                        // B. Subscribe to local physical shards
                                        if let Some(shard_config) = collection.get_shard_config() {
                                            if shard_config.num_shards > 0 {
                                                if let Ok(database) = state.storage.get_database(&db_name) {
                                                    for shard_id in 0..shard_config.num_shards {
                                                        let physical_name = format!("{}_s{}", coll_name, shard_id);
                                                        if let Ok(physical_coll) = database.get_collection(&physical_name) {
                                                            let mut shard_rx = physical_coll.change_sender.subscribe();
                                                            let tx_shard = tx.clone();
                                                            tokio::spawn(async move {
                                                                loop {
                                                                    match shard_rx.recv().await {
                                                                        Ok(event) => { if tx_shard.send(event).await.is_err() { break; } },
                                                                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                                                                        Err(_) => break,
                                                                    }
                                                                }
                                                            });
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        // C. Subscribe to REMOTE nodes
                                        let is_local_only = req.local.unwrap_or(false);
                                        if !is_local_only {
                                            if let Some(shard_config) = collection.get_shard_config() {
                                                if let Some(coordinator) = &state.shard_coordinator {
                                                    let my_addr = coordinator.my_address();
                                                    let all_nodes = coordinator.get_collection_nodes(&shard_config);
                                                    let mut remote_nodes = std::collections::HashSet::new();
                                                    for node_addr in all_nodes {
                                                        if node_addr != my_addr { remote_nodes.insert(node_addr); }
                                                    }

                                                    for node_addr in remote_nodes {
                                                        let tx_remote = tx.clone();
                                                        let db_remote = db_name.clone();
                                                        let c_remote = coll_name.clone();
                                                        let n_addr = node_addr.clone();
                                                        
                                                        tokio::spawn(async move {
                                                            use crate::cluster::ClusterWebsocketClient;
                                                            // For live query dependencies, we just need the events to trigger re-run
                                                            // We subscribe to the collection changefeed on the remote node
                                                            match ClusterWebsocketClient::connect(&n_addr, &db_remote, &c_remote, true).await {
                                                                Ok(stream) => {
                                                                    tokio::pin!(stream);
                                                                    while let Some(result) = stream.next().await {
                                                                        if let Ok(event) = result {
                                                                            if tx_remote.send(event).await.is_err() { break; }
                                                                        } else { break; }
                                                                    }
                                                                }
                                                                Err(_) => {}
                                                            }
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }

// Helper for live query execution
async fn execute_live_query_step(
    socket: &mut WebSocket,
    storage: std::sync::Arc<StorageEngine>,
    query_str: String,
    db_name: String,
    shard_coordinator: Option<std::sync::Arc<crate::sharding::ShardCoordinator>>,
) {
    // Execute SDBQL
    let exec_result = tokio::task::spawn_blocking(move || {
        match crate::sdbql::parser::parse(&query_str) {
            Ok(parsed) => {
                // Security check: Reject mutations in live queries
                for clause in &parsed.body_clauses {
                    match clause {
                        crate::sdbql::BodyClause::Insert(_) |
                        crate::sdbql::BodyClause::Update(_) |
                        crate::sdbql::BodyClause::Remove(_) => {
                             return Err(crate::error::DbError::ExecutionError("Live queries are read-only and cannot contain INSERT, UPDATE, or REMOVE operations".to_string()));
                        },
                        _ => {}
                    }
                }

                let mut executor = crate::sdbql::executor::QueryExecutor::with_database(&storage, db_name);
                if let Some(coord) = shard_coordinator {
                    executor = executor.with_shard_coordinator(coord);
                }
                executor.execute(&parsed)
            },
            Err(e) => Err(crate::error::DbError::ParseError(e.to_string()))
        }
    }).await.unwrap();

    match exec_result {
        Ok(results) => {
            let _ = socket.send(Message::Text(serde_json::json!({
                "type": "query_result",
                "result": results
            }).to_string().into())).await;
        },
        Err(e) => {
            let _ = socket.send(Message::Text(serde_json::json!({
                "type": "error",
                "error": e.to_string()
            }).to_string().into())).await;
        }
    }
}

                                // 5. Initial Execution
                                execute_live_query_step(&mut socket, state.storage.clone(), query_str.clone(), db_name.clone(), state.shard_coordinator.clone()).await;

                                // 6. Reactive Loop with heartbeat
                                // Heartbeat: Send a Ping every 30 seconds to keep the connection alive
                                let mut heartbeat_interval = tokio::time::interval(std::time::Duration::from_secs(30));
                                loop {
                                    tokio::select! {
                                        // Heartbeat tick: send a Ping to keep the connection alive
                                        _ = heartbeat_interval.tick() => {
                                            if socket.send(Message::Ping(vec![].into())).await.is_err() {
                                                tracing::debug!("[LIVE_QUERY] Failed to send ping, closing connection");
                                                break;
                                            }
                                        }
                                        Some(_) = rx.recv() => {
                                            // On ANY change to ANY dependency, re-run query
                                            execute_live_query_step(&mut socket, state.storage.clone(), query_str.clone(), db_name.clone(), state.shard_coordinator.clone()).await;
                                        }
                                        Some(msg) = socket.recv() => {
                                            match msg {
                                                Ok(Message::Close(_)) | Err(_) => break,
                                                Ok(Message::Pong(_)) => { /* heartbeat acknowledged */ }
                                                _ => {} 
                                            }
                                        }
                                        else => break,
                                    }
                                }
                            },
                            Err(e) => {
                                let _ = socket.send(Message::Text(serde_json::json!({
                                    "error": format!("Invalid SDBQL query: {}", e)
                                }).to_string().into())).await;
                            }
                         }
                    } else {
                         let _ = socket.send(Message::Text(serde_json::json!({
                             "error": "Missing 'query' field for live_query"
                         }).to_string().into())).await;
                    }
                }
                _ => {
                    let _ = socket.send(Message::Text(serde_json::json!({
                        "error": "Invalid subscription request"
                    }).to_string().into())).await;
                }

            }
        }
    }
}

/// Distribute blob chunks across the cluster for fault tolerance
/// This provides redundancy without requiring logical sharding of the collection
async fn distribute_blob_chunks_across_cluster(
    coordinator: &crate::sharding::coordinator::ShardCoordinator,
    db_name: &str,
    coll_name: &str,
    blob_key: &str,
    chunks: &[(u32, Vec<u8>)],
    metadata: &serde_json::Value,
    storage: &crate::storage::StorageEngine,
) -> Result<(), DbError> {

    // Get available nodes
    let node_addresses = coordinator.get_node_addresses();
    if node_addresses.is_empty() {
        return Err(DbError::InternalError("No nodes available for blob chunk distribution".to_string()));
    }

    tracing::info!("Distributing blob chunks to {} nodes: {:?}", node_addresses.len(), node_addresses);

    // For each chunk, replicate to multiple nodes for redundancy
    // We'll use a simple round-robin distribution with replication factor of min(3, node_count)
    let replication_factor = std::cmp::min(3, node_addresses.len());

    for (chunk_idx, chunk_data) in chunks {
        // Select target nodes for this chunk using round-robin
        let start_node = (*chunk_idx as usize) % node_addresses.len();
        let target_nodes: Vec<_> = (0..replication_factor)
            .map(|i| &node_addresses[(start_node + i) % node_addresses.len()])
            .collect();

        tracing::debug!("Chunk {} will be stored on nodes: {:?}", chunk_idx, target_nodes);

        // Replicate chunk to each target node
        for node_addr in target_nodes {
            if let Err(e) = replicate_blob_to_node(
                node_addr,
                db_name,
                coll_name,
                blob_key,
                &[(*chunk_idx, chunk_data.clone())],
                None, // No metadata for individual chunks
                "", // No auth needed for internal replication
            ).await {
                tracing::warn!("Failed to replicate chunk {} to {}: {}", chunk_idx, node_addr, e);
                // Continue with other nodes - don't fail the whole operation
            }
        }
    }

    // Store metadata document locally (this will be synced via regular replication)
    let database = storage.get_database(db_name)?;
    let collection = database.get_collection(coll_name)?;
    collection.insert(metadata.clone())?;

    tracing::info!("Successfully distributed {} chunks for blob {} across {} nodes",
        chunks.len(), blob_key, replication_factor);

    Ok(())
}

/// Fetch a blob chunk from other nodes in the cluster
async fn fetch_blob_chunk_from_cluster(
    coordinator: &crate::sharding::coordinator::ShardCoordinator,
    db_name: &str,
    coll_name: &str,
    blob_key: &str,
    chunk_idx: u32,
) -> Result<Option<Vec<u8>>, DbError> {
    let node_addresses = coordinator.get_node_addresses();

    // Try each node to find the chunk
    for node_addr in &node_addresses {
        // Skip local node (we already checked it)
        if node_addr == "local" {
            continue;
        }

        let scheme = std::env::var("SOLIDB_CLUSTER_SCHEME").unwrap_or_else(|_| "http".to_string());
        let url = if node_addr.contains("://") {
            format!("{}/_internal/blob/replicate/{}/{}/{}/chunk/{}", node_addr, db_name, coll_name, blob_key, chunk_idx)
        } else {
            format!("{}://{}/_internal/blob/replicate/{}/{}/{}/chunk/{}", scheme, node_addr, db_name, coll_name, blob_key, chunk_idx)
        };

        let client = reqwest::Client::new();
        let secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();

        match client
            .get(&url)
            .header("X-Cluster-Secret", secret)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                match response.bytes().await {
                    Ok(bytes) => {
                        let data = bytes.to_vec();
                        tracing::debug!("Fetched chunk {} for blob {} from {}", chunk_idx, blob_key, node_addr);
                        return Ok(Some(data));
                    }
                    Err(e) => {
                        tracing::warn!("Failed to read chunk data from {}: {}", node_addr, e);
                    }
                }
            }
            Ok(response) => {
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    // Chunk not on this node, try next
                    continue;
                } else {
                    tracing::warn!("Failed to fetch chunk from {}: status {}", node_addr, response.status());
                }
            }
            Err(e) => {
                tracing::warn!("Network error fetching chunk from {}: {}", node_addr, e);
            }
        }
    }

    // Chunk not found on any node
    tracing::debug!("Chunk {} for blob {} not found on any node", chunk_idx, blob_key);
    Ok(None)
}
