use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::aql::{parse, QueryExecutor};
use crate::error::DbError;
use crate::storage::{StorageEngine, IndexType, IndexStats, GeoIndexStats};
use std::collections::HashMap;

pub type AppState = Arc<StorageEngine>;

// Request/Response types
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
    /// Bind variables for parameterized queries (prevents AQL injection)
    #[serde(rename = "bindVars", default)]
    pub bind_vars: HashMap<String, Value>,
}

#[derive(Debug, Serialize)]
pub struct ExecuteQueryResponse {
    pub result: Vec<Value>,
    pub count: usize,
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

// Handler: Create collection
pub async fn create_collection(
    State(storage): State<AppState>,
    Json(req): Json<CreateCollectionRequest>,
) -> Result<Json<CreateCollectionResponse>, DbError> {
    storage.create_collection(req.name.clone())?;

    Ok(Json(CreateCollectionResponse {
        name: req.name,
        status: "created".to_string(),
    }))
}

// Handler: List collections
pub async fn list_collections(
    State(storage): State<AppState>,
) -> Json<ListCollectionsResponse> {
    let collections = storage.list_collections();
    Json(ListCollectionsResponse { collections })
}

// Handler: Delete collection
pub async fn delete_collection(
    State(storage): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, DbError> {
    storage.delete_collection(&name)?;
    Ok(StatusCode::NO_CONTENT)
}

// Handler: Insert document
pub async fn insert_document(
    State(storage): State<AppState>,
    Path(collection_name): Path<String>,
    Json(data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    let collection = storage.get_collection(&collection_name)?;
    let doc = collection.insert(data)?;

    // Save collection to disk
    storage.save_collection(&collection_name)?;

    Ok(Json(doc.to_value()))
}

// Handler: Get document
pub async fn get_document(
    State(storage): State<AppState>,
    Path((collection_name, key)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let collection = storage.get_collection(&collection_name)?;
    let doc = collection.get(&key)?;
    Ok(Json(doc.to_value()))
}

// Handler: Update document
pub async fn update_document(
    State(storage): State<AppState>,
    Path((collection_name, key)): Path<(String, String)>,
    Json(data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    let collection = storage.get_collection(&collection_name)?;
    let doc = collection.update(&key, data)?;

    // Save collection to disk
    storage.save_collection(&collection_name)?;

    Ok(Json(doc.to_value()))
}

// Handler: Delete document
pub async fn delete_document(
    State(storage): State<AppState>,
    Path((collection_name, key)): Path<(String, String)>,
) -> Result<StatusCode, DbError> {
    let collection = storage.get_collection(&collection_name)?;
    collection.delete(&key)?;

    // Save collection to disk
    storage.save_collection(&collection_name)?;

    Ok(StatusCode::NO_CONTENT)
}

// Handler: Execute AQL query
pub async fn execute_query(
    State(storage): State<AppState>,
    Json(req): Json<ExecuteQueryRequest>,
) -> Result<Json<ExecuteQueryResponse>, DbError> {
    // Parse the query
    let query = parse(&req.query)?;

    // Execute the query with bind variables (prevents AQL injection)
    let executor = if req.bind_vars.is_empty() {
        QueryExecutor::new(&storage)
    } else {
        QueryExecutor::with_bind_vars(&storage, req.bind_vars)
    };
    let result = executor.execute(&query)?;
    let count = result.len();

    Ok(Json(ExecuteQueryResponse { result, count }))
}

// Handler: Explain/Profile AQL query
pub async fn explain_query(
    State(storage): State<AppState>,
    Json(req): Json<ExecuteQueryRequest>,
) -> Result<Json<crate::aql::QueryExplain>, DbError> {
    // Parse the query
    let query = parse(&req.query)?;

    // Execute with profiling
    let executor = if req.bind_vars.is_empty() {
        QueryExecutor::new(&storage)
    } else {
        QueryExecutor::with_bind_vars(&storage, req.bind_vars)
    };
    let explain = executor.explain(&query)?;

    Ok(Json(explain))
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

// Handler: Create index
pub async fn create_index(
    State(storage): State<AppState>,
    Path(collection_name): Path<String>,
    Json(req): Json<CreateIndexRequest>,
) -> Result<Json<CreateIndexResponse>, DbError> {
    let collection = storage.get_collection(&collection_name)?;

    let index_type = match req.index_type.to_lowercase().as_str() {
        "hash" => IndexType::Hash,
        "persistent" | "skiplist" | "btree" => IndexType::Persistent,
        _ => return Err(DbError::InvalidDocument(format!("Unknown index type: {}", req.index_type))),
    };

    collection.create_index(req.name.clone(), req.field.clone(), index_type.clone(), req.unique)?;

    // Save collection to persist the index
    storage.save_collection(&collection_name)?;

    Ok(Json(CreateIndexResponse {
        name: req.name,
        field: req.field,
        index_type,
        unique: req.unique,
        status: "created".to_string(),
    }))
}

// Handler: List indexes
pub async fn list_indexes(
    State(storage): State<AppState>,
    Path(collection_name): Path<String>,
) -> Result<Json<ListIndexesResponse>, DbError> {
    let collection = storage.get_collection(&collection_name)?;
    let indexes = collection.list_indexes();
    Ok(Json(ListIndexesResponse { indexes }))
}

// Handler: Delete index
pub async fn delete_index(
    State(storage): State<AppState>,
    Path((collection_name, index_name)): Path<(String, String)>,
) -> Result<StatusCode, DbError> {
    let collection = storage.get_collection(&collection_name)?;
    collection.drop_index(&index_name)?;

    // Save collection
    storage.save_collection(&collection_name)?;

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
    pub radius: f64, // meters
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

// Handler: Create geo index
pub async fn create_geo_index(
    State(storage): State<AppState>,
    Path(collection_name): Path<String>,
    Json(req): Json<CreateGeoIndexRequest>,
) -> Result<Json<CreateGeoIndexResponse>, DbError> {
    let collection = storage.get_collection(&collection_name)?;
    collection.create_geo_index(req.name.clone(), req.field.clone())?;

    // Save collection to persist the index
    storage.save_collection(&collection_name)?;

    Ok(Json(CreateGeoIndexResponse {
        name: req.name,
        field: req.field,
        index_type: "geo".to_string(),
        status: "created".to_string(),
    }))
}

// Handler: List geo indexes
pub async fn list_geo_indexes(
    State(storage): State<AppState>,
    Path(collection_name): Path<String>,
) -> Result<Json<ListGeoIndexesResponse>, DbError> {
    let collection = storage.get_collection(&collection_name)?;
    let indexes = collection.list_geo_indexes();
    Ok(Json(ListGeoIndexesResponse { indexes }))
}

// Handler: Delete geo index
pub async fn delete_geo_index(
    State(storage): State<AppState>,
    Path((collection_name, index_name)): Path<(String, String)>,
) -> Result<StatusCode, DbError> {
    let collection = storage.get_collection(&collection_name)?;
    collection.drop_geo_index(&index_name)?;

    storage.save_collection(&collection_name)?;

    Ok(StatusCode::NO_CONTENT)
}

// Handler: Geo near query
pub async fn geo_near(
    State(storage): State<AppState>,
    Path((collection_name, field)): Path<(String, String)>,
    Json(req): Json<GeoNearRequest>,
) -> Result<Json<GeoQueryResponse>, DbError> {
    let collection = storage.get_collection(&collection_name)?;

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

// Handler: Geo within query
pub async fn geo_within(
    State(storage): State<AppState>,
    Path((collection_name, field)): Path<(String, String)>,
    Json(req): Json<GeoWithinRequest>,
) -> Result<Json<GeoQueryResponse>, DbError> {
    let collection = storage.get_collection(&collection_name)?;

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
