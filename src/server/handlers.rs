use axum::{
    extract::{Multipart, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    body::Body,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::aql::{parse, BodyClause, Query, QueryExecutor};
use crate::cluster::{Operation, ReplicationService};
use crate::error::DbError;
use crate::server::cursor_store::CursorStore;
use crate::storage::{GeoIndexStats, IndexStats, IndexType, StorageEngine};
use crate::transaction::TransactionId;
use std::collections::HashMap;

/// Check if a query is potentially long-running (contains mutations or range iterations)
#[inline]
fn is_long_running_query(query: &Query) -> bool {
    query.body_clauses.iter().any(|clause| match clause {
        BodyClause::Insert(_) | BodyClause::Update(_) | BodyClause::Remove(_) => true,
        BodyClause::For(f) => f.source_expression.is_some(),
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

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<StorageEngine>,
    pub cursor_store: CursorStore,
    pub replication: Option<ReplicationService>,
    pub shard_coordinator: Option<crate::sharding::ShardCoordinator>,
    pub startup_time: std::time::Instant,
    pub request_counter: Arc<std::sync::atomic::AtomicU64>,
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
    
    collection.update(&claims.sub, updated_value)?;

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
        db.create_collection(crate::server::auth::API_KEYS_COLL.to_string())?;
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
    
    collection.insert(doc_value)?;
    
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
    
    tracing::info!("API key '{}' deleted", key_id);
    
    Ok(Json(DeleteApiKeyResponse { deleted: true }))
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
    #[serde(rename = "shardConfig", skip_serializing_if = "Option::is_none")]
    pub shard_config: Option<crate::sharding::coordinator::CollectionShardConfig>,
}

#[derive(Debug, Serialize)]
pub struct ListCollectionsResponse {
    pub collections: Vec<CollectionSummary>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCollectionPropertiesRequest {
    /// Number of shards (updating this triggers rebalance)
    #[serde(rename = "numShards")]
    pub num_shards: Option<u16>,
    /// Replication factor (optional, default: 1 = no replicas)
    #[serde(rename = "replicationFactor")]
    pub replication_factor: Option<u16>,
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

// ==================== Auth Handlers ====================

pub async fn login_handler(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, DbError> {
    // 1. Get _system database
    let db = state.storage.get_database("_system")?;
    
    // 2. Get _admins collection (create with default admin if missing)
    let collection = match db.get_collection("_admins") {
        Ok(c) => c,
        Err(DbError::CollectionNotFound(_)) => {
            // Collection doesn't exist - initialize auth (creates collection and default admin)
            tracing::warn!("_admins collection not found, initializing...");
            crate::server::auth::AuthService::init(&state.storage)?;
            db.get_collection("_admins")?
        }
        Err(e) => return Err(e),
    };
    
    // 3. Check if collection is empty (also create default admin)
    if collection.count() == 0 {
        tracing::warn!("_admins collection empty, creating default admin...");
        crate::server::auth::AuthService::init(&state.storage)?;
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

// ==================== Database Handlers ====================

pub async fn create_database(
    State(state): State<AppState>,
    Json(req): Json<CreateDatabaseRequest>,
) -> Result<Json<CreateDatabaseResponse>, DbError> {
    state.storage.create_database(req.name.clone())?;

    // Record to replication log
    if let Some(ref repl) = state.replication {
        repl.record_write(&req.name, "", Operation::CreateDatabase, "", None, None);
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
    if let Some(ref repl) = state.replication {
        repl.record_write(&name, "", Operation::DeleteDatabase, "", None, None);
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
    database.create_collection(req.name.clone())?;

    // Store sharding configuration if specified
    if let Some(num_shards) = req.num_shards {
        let collection = database.get_collection(&req.name)?;
        let shard_config = crate::sharding::coordinator::CollectionShardConfig {
            num_shards,
            shard_key: req.shard_key.clone().unwrap_or_else(|| "_key".to_string()),
            replication_factor: req.replication_factor.unwrap_or(1),
        };
        // Store shard config in collection metadata
        collection.set_shard_config(&shard_config)?;
    }

    // Record to replication log
    if let Some(ref repl) = state.replication {
        repl.record_write(
            &db_name,
            &req.name,
            Operation::CreateCollection,
            "",
            None,
            None,
        );
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
) -> Result<Json<ListCollectionsResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let names = database.list_collections();
    
    let mut collections = Vec::with_capacity(names.len());
    for name in names {
        if let Ok(coll) = database.get_collection(&name) {
            let count = coll.count();
            let shard_config = coll.get_shard_config();
            collections.push(CollectionSummary {
                name,
                count,
                shard_config,
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
) -> Result<StatusCode, DbError> {
    // Protect system collections
    if is_protected_collection(&db_name, &coll_name) {
        return Err(DbError::BadRequest(format!("Cannot delete protected system collection: {}", coll_name)));
    }

    let database = state.storage.get_database(&db_name)?;
    database.delete_collection(&coll_name)?;

    // Record to replication log
    if let Some(ref repl) = state.replication {
        repl.record_write(
            &db_name,
            &coll_name,
            Operation::DeleteCollection,
            "",
            None,
            None,
        );
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn truncate_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    // Protect system collections
    if is_protected_collection(&db_name, &coll_name) {
        return Err(DbError::BadRequest(format!("Cannot truncate protected system collection: {}", coll_name)));
    }

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Run in blocking task since this can be slow for large collections
    let coll = collection.clone();
    let count = tokio::task::spawn_blocking(move || coll.truncate())
        .await
        .map_err(|e| DbError::InternalError(format!("Task error: {}", e)))??;

    // Record to replication log
    if let Some(ref repl) = state.replication {
        repl.record_write(
            &db_name,
            &coll_name,
            Operation::TruncateCollection,
            "",
            None,
            None,
        );
    }

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "deleted": count,
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

pub async fn get_collection_stats(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let stats = collection.stats();

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "document_count": stats.document_count,
        "disk_usage": {
            "sst_files_size": stats.disk_usage.sst_files_size,
            "live_data_size": stats.disk_usage.live_data_size,
            "num_sst_files": stats.disk_usage.num_sst_files,
            "memtable_size": stats.disk_usage.memtable_size,
            "total_size": stats.disk_usage.sst_files_size + stats.disk_usage.memtable_size
        }
    })))
}

pub async fn update_collection_properties(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(payload): Json<UpdateCollectionPropertiesRequest>,
) -> Result<Json<CollectionPropertiesResponse>, DbError> {
    tracing::info!("update_collection_properties called: db={}, coll={}, payload={:?}", db_name, coll_name, payload);
    
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Get existing config or create new one if not sharded yet
    let mut config = collection.get_shard_config()
        .unwrap_or_else(|| crate::sharding::coordinator::CollectionShardConfig::default());

    tracing::info!("Current config before update: {:?}", config);

    let old_num_shards = config.num_shards;
    let mut shard_count_changed = false;

    // Update num_shards if specified
    if let Some(ns) = payload.num_shards {
        if ns < 1 {
            return Err(DbError::BadRequest("Number of shards must be >= 1".to_string()));
        }
        if ns != old_num_shards {
            tracing::info!("Updating num_shards from {} to {}", old_num_shards, ns);
            config.num_shards = ns;
            shard_count_changed = true;
        }
    }

    // Update replication_factor if specified
    if let Some(rf) = payload.replication_factor {
        if rf < 1 {
            return Err(DbError::BadRequest("Replication factor must be >= 1".to_string()));
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
            let coordinator_clone = coordinator.clone();
            tokio::spawn(async move {
                if let Err(e) = coordinator_clone.rebalance().await {
                    tracing::error!("Rebalance failed: {}", e);
                }
            });
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
) -> Result<Response, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let docs = collection.scan(None);
    let db_name_clone = db_name.clone();
    let coll_name_clone = coll_name.clone();
    let shard_config = collection.get_shard_config();

    let stream = async_stream::stream! {
        for doc in docs {
            let mut val = doc.to_value();
            if let Some(obj) = val.as_object_mut() {
                if let Some(ref config) = shard_config {
                     obj.insert("_shardConfig".to_string(), serde_json::to_value(config).unwrap_or_default());
                }
            }
            if let Ok(json) = serde_json::to_string(&val) {
                yield Ok::<_, std::io::Error>(axum::body::Bytes::from(format!("{}\n", json)));
            }
        }
    };

    let body = Body::from_stream(stream);

    Ok(Response::builder()
        .header("Content-Type", "application/x-ndjson")
        .header("Content-Disposition", format!("attachment; filename=\"{}-{}.jsonl\"", db_name, coll_name))
        .body(body)
        .unwrap())
}

pub async fn import_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    mut multipart: Multipart,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = match database.get_collection(&coll_name) {
        Ok(c) => c,
        Err(DbError::CollectionNotFound(_)) => {
            tracing::info!("Auto-creating collection '{}' during import", coll_name);
            match database.create_collection(coll_name.clone()) {
                Ok(_) => database.get_collection(&coll_name)?,
                Err(e) => return Err(e),
            }
        },
        Err(e) => return Err(e),
    };

    let mut success_count = 0;
    let mut error_count = 0;

    while let Some(field) = multipart.next_field().await.map_err(|e| DbError::BadRequest(e.to_string()))? {
        if field.name() == Some("file") {
            let text = field.text().await.map_err(|e| DbError::BadRequest(e.to_string()))?;
            
            // Detect format
            let first_char = text.trim().chars().next();
            
            let docs: Vec<Value> = if first_char == Some('[') {
                // JSON Array
                serde_json::from_str(&text).map_err(|e| DbError::BadRequest(format!("Invalid JSON Array: {}", e)))?
            } else if first_char == Some('{') {
                // JSONL
                text.lines()
                    .filter(|l| !l.trim().is_empty())
                    .map(|l| serde_json::from_str(l))
                    .collect::<Result<Vec<Value>, _>>()
                    .map_err(|e| DbError::BadRequest(format!("Invalid JSONL: {}", e)))?
            } else {
                 // CSV (Basic inference)
                 let mut reader = csv::Reader::from_reader(text.as_bytes());
                 let headers = reader.headers().map_err(|e| DbError::BadRequest(e.to_string()))?.clone();
                 let mut csv_docs = Vec::new();
                 
                 for result in reader.records() {
                     let record = result.map_err(|e| DbError::BadRequest(e.to_string()))?;
                     let mut map = serde_json::Map::new();
                     for (i, field) in record.iter().enumerate() {
                         if i < headers.len() {
                             // Try to infer numbers/bools
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

            let mut batch_docs = Vec::with_capacity(10000);
            
            for mut doc in docs {
                // Remove metadata if present
                if let Some(obj) = doc.as_object_mut() {
                    obj.remove("_database");
                    obj.remove("_collection");
                    obj.remove("_shardConfig");
                }
                
                batch_docs.push(doc);
                
                if batch_docs.len() >= 10000 {
                    match collection.insert_batch(batch_docs.clone()) {
                        Ok(inserted) => {
                            if let Err(e) = collection.index_documents(&inserted) {
                                tracing::error!("Failed to index batch: {}", e);
                            }
                            success_count += inserted.len();
                        }
                        Err(e) => {
                            tracing::error!("Failed to insert batch: {}", e);
                            error_count += batch_docs.len();
                        }
                    }
                    batch_docs.clear();
                }
            }
            
            // Insert remaining docs
            if !batch_docs.is_empty() {
                match collection.insert_batch(batch_docs.clone()) {
                    Ok(inserted) => {
                        if let Err(e) = collection.index_documents(&inserted) {
                            tracing::error!("Failed to index batch: {}", e);
                        }
                        success_count += inserted.len();
                    }
                    Err(e) => {
                        tracing::error!("Failed to insert batch: {}", e);
                        error_count += batch_docs.len();
                    }
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "imported": success_count,
        "failed": error_count,
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
        let mut tx = tx_arc.write().unwrap();
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
                    return Ok(Json(doc.to_value()));
                } else {
                    tracing::info!("[INSERT] X-Shard-Direct header present, skipping coordinator");
                }
            } else {
                tracing::info!("[INSERT] No shard_coordinator available");
            }
        }
    } else {
        tracing::info!("[INSERT] No shard_config for {}/{}", db_name, coll_name);
    }

    let doc = collection.insert(data)?;

    // Record to replication log ONLY if not a shard-directed insert
    // Shard-directed inserts are already handled by the ShardCoordinator replication
    let is_shard_direct = headers.contains_key("X-Shard-Direct");
    if !is_shard_direct {
        if let Some(ref repl) = state.replication {
            let doc_bytes = serde_json::to_vec(&doc.to_value()).ok();
            repl.record_write(
                &db_name,
                &coll_name,
                Operation::Insert,
                &doc.key,
                doc_bytes.as_deref(),
                None,
            );
        }
    }

    Ok(Json(doc.to_value()))
}

pub async fn get_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check for sharding
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                let doc = coordinator.get(
                    &db_name,
                    &coll_name,
                    &shard_config,
                    &key
                ).await?;
                
                let mut doc_value = doc.to_value();
                let replicas = coordinator.get_replicas(&key, &shard_config);
                if let Value::Object(ref mut map) = doc_value {
                    map.insert("_replicas".to_string(), serde_json::json!(replicas));
                }
                
                return Ok(Json(doc_value));
            }
        }
    }

    let doc = collection.get(&key)?;
    Ok(Json(doc.to_value()))
}

pub async fn update_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check for transaction context
    if let Some(tx_id) = get_transaction_id(&headers) {
        let tx_manager = state.storage.transaction_manager()?;
        let tx_arc = tx_manager.get(tx_id)?;
        let mut tx = tx_arc.write().unwrap();
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
                    return Ok(Json(doc.to_value()));
                }
            }
        }
    }

    let doc = collection.update(&key, data)?;

    // Record to replication log
    if let Some(ref repl) = state.replication {
        let doc_bytes = serde_json::to_vec(&doc.to_value()).ok();
        repl.record_write(
            &db_name,
            &coll_name,
            Operation::Update,
            &doc.key,
            doc_bytes.as_deref(),
            Some(&doc.rev),
        );
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
        let mut tx = tx_arc.write().unwrap();
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

    // Record to replication log
    if let Some(ref repl) = state.replication {
        repl.record_write(&db_name, &coll_name, Operation::Delete, &key, None, None);
    }

    Ok(StatusCode::NO_CONTENT)
}

// ==================== Query Handlers ====================

pub async fn execute_query(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ExecuteQueryRequest>,
) -> Result<Json<ExecuteQueryResponse>, DbError> {
    // Check for transaction context
    if let Some(tx_id) = get_transaction_id(&headers) {
        // Execute transactional AQL query
        use crate::aql::ast::BodyClause;
        
        let query = parse(&req.query)?;
        
        // Get transaction manager
        let tx_manager = state.storage.transaction_manager()?;
        let tx_arc = tx_manager.get(tx_id)?;
        let mut tx = tx_arc.write().unwrap();
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
            return Ok(Json(ExecuteQueryResponse {
                result: results.clone(),
                count: results.len(),
                has_more: false,
                id: None,
                cached: false,
                execution_time_ms: 0.0,
            }));
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
        return Ok(Json(ExecuteQueryResponse {
            result: vec![serde_json::json!({
                "mutationCount": mutation_count,
                "message": format!("{} operation(s) staged in transaction. Commit to apply changes.", mutation_count)
            })],
            count: 1,
            has_more: false,
            id: None,
            cached: false,
            execution_time_ms: 0.0,
        }));
    }

    // Non-transactional execution (existing logic)
    let query = parse(&req.query)?;
    let batch_size = req.batch_size;

    // Only use spawn_blocking for potentially long-running queries
    // (mutations or range iterations). Simple reads run directly.
    let (result, execution_time_ms) = if is_long_running_query(&query) {
        let storage = state.storage.clone();
        let bind_vars = req.bind_vars.clone();
        let replication = state.replication.clone();
        let shard_coordinator = state.shard_coordinator.clone();
        let is_scatter_gather = headers.contains_key("X-Scatter-Gather");

        tokio::task::spawn_blocking(move || {
            let mut executor = if bind_vars.is_empty() {
                QueryExecutor::with_database(&storage, db_name)
            } else {
                QueryExecutor::with_database_and_bind_vars(&storage, db_name, bind_vars)
            };

            // Add replication service for mutation logging
            if let Some(ref repl) = replication {
                executor = executor.with_replication(repl);
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
        .await
        .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??
    } else {
        let mut executor = if req.bind_vars.is_empty() {
            QueryExecutor::with_database(&state.storage, db_name)
        } else {
            QueryExecutor::with_database_and_bind_vars(&state.storage, db_name, req.bind_vars)
        };

        // Add replication service for mutation logging
        if let Some(ref repl) = state.replication {
            executor = executor.with_replication(repl);
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

        Ok(Json(ExecuteQueryResponse {
            result: first_batch,
            count: total_count,
            has_more,
            id: if has_more { Some(cursor_id) } else { None },
            cached: false,
            execution_time_ms,
        }))
    } else {
        Ok(Json(ExecuteQueryResponse {
            result,
            count: total_count,
            has_more: false,
            id: None,
            cached: false,
            execution_time_ms,
        }))
    }
}

pub async fn explain_query(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ExecuteQueryRequest>,
) -> Result<Json<crate::aql::QueryExplain>, DbError> {
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
    pub field: String,
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

    collection.create_index(
        req.name.clone(),
        req.field.clone(),
        index_type.clone(),
        req.unique,
    )?;

    Ok(Json(CreateIndexResponse {
        name: req.name,
        field: req.field,
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
    collection.drop_index(&index_name)?;

    Ok(StatusCode::NO_CONTENT)
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

// ==================== Cluster Status ====================

#[derive(Debug, Serialize)]
pub struct PeerStatusResponse {
    pub address: String,
    pub is_connected: bool,
    pub last_seen_secs_ago: u64,
    pub replication_lag: u64,
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

pub async fn cluster_status(State(state): State<AppState>) -> Json<ClusterStatusResponse> {
    use sysinfo::System;
    use std::sync::atomic::Ordering;

    let node_id = state.storage.node_id().to_string();
    let data_dir = state.storage.data_dir().to_string();

    let replication_port = state
        .storage
        .cluster_config()
        .map(|c| c.replication_port)
        .unwrap_or(6746);

    // Calculate stats
    let databases = state.storage.list_databases();
    let database_count = databases.len();
    
    let mut collection_count = 0;
    let mut document_count: u64 = 0;
    
    for db_name in &databases {
        if let Ok(db) = state.storage.get_database(db_name) {
            let coll_names = db.list_collections();
            collection_count += coll_names.len();
            for coll_name in coll_names {
                if let Ok(coll) = db.get_collection(&coll_name) {
                    document_count += coll.count() as u64;
                }
            }
        }
    }

    // Storage size (approximate from data directory)
    let storage_bytes = get_dir_size(&data_dir).unwrap_or(0);

    // Uptime
    let uptime_secs = state.startup_time.elapsed().as_secs();

    // Memory and CPU usage
    let mut sys = System::new_all();
    let pid = sysinfo::get_current_pid().ok();
    
    let (memory_used_mb, cpu_usage_percent) = pid
        .and_then(|p| sys.process(p))
        .map(|p| (p.memory() / (1024 * 1024), p.cpu_usage()))
        .unwrap_or((0, 0.0));
    
    let memory_total_mb = sys.total_memory() / (1024 * 1024);

    // Request count
    let request_count = state.request_counter.load(Ordering::Relaxed);

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
    };

    // Get live status from replication service if available
    if let Some(ref replication) = state.replication {
        let cluster_status = replication.get_status();

        let status = if cluster_status.peers.iter().any(|p| p.is_connected) {
            "cluster".to_string()
        } else if !cluster_status.peers.is_empty() {
            "cluster-connecting".to_string()
        } else {
            "cluster-ready".to_string()
        };

        let peers: Vec<PeerStatusResponse> = cluster_status
            .peers
            .into_iter()
            .map(|p| PeerStatusResponse {
                address: p.address,
                is_connected: p.is_connected,
                last_seen_secs_ago: p.last_seen_secs_ago,
                replication_lag: p.replication_lag,
            })
            .collect();

        Json(ClusterStatusResponse {
            node_id: cluster_status.node_id,
            status,
            replication_port,
            current_sequence: cluster_status.current_sequence,
            log_entries: cluster_status.log_entries,
            peers,
            data_dir,
            stats,
        })
    } else {
        Json(ClusterStatusResponse {
            node_id,
            status: "standalone".to_string(),
            replication_port,
            current_sequence: 0,
            log_entries: 0,
            peers: vec![],
            data_dir,
            stats,
        })
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

    Json(ClusterInfoResponse {
        node_id,
        is_cluster_mode,
        cluster_config,
    })
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
