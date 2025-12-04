use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::aql::{parse, QueryExecutor, BodyClause, Query};
use crate::error::DbError;
use crate::storage::{StorageEngine, IndexType, IndexStats, GeoIndexStats};
use crate::server::cursor_store::CursorStore;
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

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<StorageEngine>,
    pub cursor_store: CursorStore,
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
}

#[derive(Debug, Serialize)]
pub struct CreateCollectionResponse {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ListCollectionsResponse {
    pub collections: Vec<String>,
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
            DbError::InvalidDocument(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };

        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

// ==================== Database Handlers ====================

pub async fn create_database(
    State(state): State<AppState>,
    Json(req): Json<CreateDatabaseRequest>,
) -> Result<Json<CreateDatabaseResponse>, DbError> {
    state.storage.create_database(req.name.clone())?;

    Ok(Json(CreateDatabaseResponse {
        name: req.name,
        status: "created".to_string(),
    }))
}

pub async fn list_databases(
    State(state): State<AppState>,
) -> Json<ListDatabasesResponse> {
    let databases = state.storage.list_databases();
    Json(ListDatabasesResponse { databases })
}

pub async fn delete_database(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, DbError> {
    state.storage.delete_database(&name)?;
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

    Ok(Json(CreateCollectionResponse {
        name: req.name,
        status: "created".to_string(),
    }))
}

pub async fn list_collections(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
) -> Result<Json<ListCollectionsResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collections = database.list_collections();
    Ok(Json(ListCollectionsResponse { collections }))
}

pub async fn delete_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<StatusCode, DbError> {
    let database = state.storage.get_database(&db_name)?;
    database.delete_collection(&coll_name)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn truncate_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Run in blocking task since this can be slow for large collections
    let coll = collection.clone();
    let count = tokio::task::spawn_blocking(move || {
        coll.truncate()
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task error: {}", e)))??;

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

// ==================== Document Handlers ====================

pub async fn insert_document(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let doc = collection.insert(data)?;

    Ok(Json(doc.to_value()))
}

pub async fn get_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let doc = collection.get(&key)?;
    Ok(Json(doc.to_value()))
}

pub async fn update_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
    Json(data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let doc = collection.update(&key, data)?;

    Ok(Json(doc.to_value()))
}

pub async fn delete_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
) -> Result<StatusCode, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    collection.delete(&key)?;

    Ok(StatusCode::NO_CONTENT)
}

// ==================== Query Handlers ====================

pub async fn execute_query(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<ExecuteQueryRequest>,
) -> Result<Json<ExecuteQueryResponse>, DbError> {
    let query = parse(&req.query)?;
    let batch_size = req.batch_size;

    // Only use spawn_blocking for potentially long-running queries
    // (mutations or range iterations). Simple reads run directly.
    let (result, execution_time_ms) = if is_long_running_query(&query) {
        let storage = state.storage.clone();
        let bind_vars = req.bind_vars.clone();

        tokio::task::spawn_blocking(move || {
            let executor = if bind_vars.is_empty() {
                QueryExecutor::with_database(&storage, db_name)
            } else {
                QueryExecutor::with_database_and_bind_vars(&storage, db_name, bind_vars)
            };

            let start = std::time::Instant::now();
            let result = executor.execute(&query)?;
            let execution_time_ms = start.elapsed().as_secs_f64() * 1000.0;
            Ok::<_, DbError>((result, execution_time_ms))
        })
        .await
        .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??
    } else {
        let executor = if req.bind_vars.is_empty() {
            QueryExecutor::with_database(&state.storage, db_name)
        } else {
            QueryExecutor::with_database_and_bind_vars(&state.storage, db_name, req.bind_vars)
        };

        let start = std::time::Instant::now();
        let result = executor.execute(&query)?;
        let execution_time_ms = start.elapsed().as_secs_f64() * 1000.0;
        (result, execution_time_ms)
    };

    let total_count = result.len();

    if total_count > batch_size {
        let cursor_id = state.cursor_store.store(result, batch_size);
        let (first_batch, has_more) = state.cursor_store.get_next_batch(&cursor_id)
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
    Json(req): Json<ExecuteQueryRequest>,
) -> Result<Json<crate::aql::QueryExplain>, DbError> {
    let query = parse(&req.query)?;

    // explain() is fast - no need for spawn_blocking
    let executor = if req.bind_vars.is_empty() {
        QueryExecutor::with_database(&state.storage, db_name)
    } else {
        QueryExecutor::with_database_and_bind_vars(&state.storage, db_name, req.bind_vars)
    };
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
        Err(DbError::DocumentNotFound(format!("Cursor not found or expired: {}", cursor_id)))
    }
}

pub async fn delete_cursor(
    State(state): State<AppState>,
    Path(cursor_id): Path<String>,
) -> Result<StatusCode, DbError> {
    if state.cursor_store.delete(&cursor_id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(DbError::DocumentNotFound(format!("Cursor not found: {}", cursor_id)))
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
        _ => return Err(DbError::InvalidDocument(format!("Unknown index type: {}", req.index_type))),
    };

    collection.create_index(req.name.clone(), req.field.clone(), index_type.clone(), req.unique)?;

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
    let count = tokio::task::spawn_blocking(move || {
        coll.rebuild_all_indexes()
    })
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

    let results = collection.geo_near(&field, req.lat, req.lon, req.limit)
        .ok_or_else(|| DbError::InvalidDocument(format!("No geo index found on field '{}'", field)))?;

    let geo_results: Vec<GeoResult> = results
        .into_iter()
        .map(|(doc, dist)| GeoResult {
            document: doc.to_value(),
            distance: dist,
        })
        .collect();

    let count = geo_results.len();

    Ok(Json(GeoQueryResponse { results: geo_results, count }))
}

pub async fn geo_within(
    State(state): State<AppState>,
    Path((db_name, coll_name, field)): Path<(String, String, String)>,
    Json(req): Json<GeoWithinRequest>,
) -> Result<Json<GeoQueryResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let results = collection.geo_within(&field, req.lat, req.lon, req.radius)
        .ok_or_else(|| DbError::InvalidDocument(format!("No geo index found on field '{}'", field)))?;

    let geo_results: Vec<GeoResult> = results
        .into_iter()
        .map(|(doc, dist)| GeoResult {
            document: doc.to_value(),
            distance: dist,
        })
        .collect();

    let count = geo_results.len();

    Ok(Json(GeoQueryResponse { results: geo_results, count }))
}
