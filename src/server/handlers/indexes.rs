use super::system::AppState;
use crate::{
    error::DbError,
    storage::{
        GeoIndexStats, IndexKind, IndexRef, IndexSpec, IndexStats, IndexType, TtlIndexStats,
        VectorIndexStats,
    },
    sync::{log::LogEntry, protocol::Operation},
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ==================== Index propagation ====================

/// Physical shard names for a sharded collection, or empty when it is not
/// sharded.
///
/// A sharded collection keeps its documents in `<name>_s0..._sN`; the logical
/// collection itself holds nothing. An index applied only to the logical
/// collection therefore indexes an empty collection — which is what happened
/// before this existed.
fn physical_shard_names(state: &AppState, db_name: &str, coll_name: &str) -> Vec<String> {
    let Ok(db) = state.storage.get_database(db_name) else {
        return Vec::new();
    };
    let Ok(coll) = db.get_collection(coll_name) else {
        return Vec::new();
    };
    match coll.get_shard_config() {
        Some(config) if config.num_shards > 0 => (0..config.num_shards)
            .map(|shard_id| format!("{}_s{}", coll_name, shard_id))
            .collect(),
        _ => Vec::new(),
    }
}

/// Apply an index to every physical shard this node holds, and replicate the
/// definition to the rest of the cluster.
///
/// Index creation used to be purely local and purely logical: nothing reached
/// the replication log, so peers never built the index (an unindexed peer runs
/// full scans, and a peer without a replicated TTL index never expires its
/// documents); and nothing reached the shards, so indexes on sharded
/// collections indexed nothing.
///
/// One log entry is emitted per target collection — the logical one plus each
/// shard — and each node applies whichever of those it actually holds.
async fn propagate_index_create(
    state: &AppState,
    db_name: &str,
    coll_name: &str,
    spec: &IndexSpec,
) {
    let payload = match serde_json::to_vec(spec) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to serialise index spec for replication: {}", e);
            return;
        }
    };

    let mut targets = vec![coll_name.to_string()];
    targets.extend(physical_shard_names(state, db_name, coll_name));

    for target in &targets {
        // Apply locally to shards we hold. The logical collection was already
        // handled by the caller.
        if target != coll_name {
            if let Ok(db) = state.storage.get_database(db_name) {
                if let Ok(shard) = db.get_collection(target) {
                    if let Err(e) = shard.apply_index_spec(spec) {
                        tracing::warn!(
                            "Failed to create index '{}' on local shard {}.{}: {}",
                            spec.name(),
                            db_name,
                            target,
                            e
                        );
                    }
                }
            }
        }

        if let Some(ref log) = state.replication_log {
            let _ = log.append(LogEntry {
                sequence: 0,
                node_id: String::new(),
                database: db_name.to_string(),
                collection: target.clone(),
                operation: Operation::CreateIndex,
                key: spec.name().to_string(),
                data: Some(payload.clone()),
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            });
        }
    }
}

/// Mirror of [`propagate_index_create`] for drops.
async fn propagate_index_drop(
    state: &AppState,
    db_name: &str,
    coll_name: &str,
    kind: IndexKind,
    name: &str,
) {
    let index_ref = IndexRef {
        kind,
        name: name.to_string(),
    };
    let payload = match serde_json::to_vec(&index_ref) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to serialise index ref for replication: {}", e);
            return;
        }
    };

    let mut targets = vec![coll_name.to_string()];
    targets.extend(physical_shard_names(state, db_name, coll_name));

    for target in &targets {
        if target != coll_name {
            if let Ok(db) = state.storage.get_database(db_name) {
                if let Ok(shard) = db.get_collection(target) {
                    if let Err(e) = shard.apply_index_drop(kind, name) {
                        tracing::warn!(
                            "Failed to drop index '{}' on local shard {}.{}: {}",
                            name,
                            db_name,
                            target,
                            e
                        );
                    }
                }
            }
        }

        if let Some(ref log) = state.replication_log {
            let _ = log.append(LogEntry {
                sequence: 0,
                node_id: String::new(),
                database: db_name.to_string(),
                collection: target.clone(),
                operation: Operation::DropIndex,
                key: name.to_string(),
                data: Some(payload.clone()),
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            });
        }
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
        "bloom" => IndexType::Bloom,
        "cuckoo" => IndexType::Cuckoo,
        _ => {
            return Err(DbError::InvalidDocument(format!(
                "Unknown index type: {}",
                req.index_type
            )))
        }
    };

    let spec = match index_type {
        IndexType::Fulltext => IndexSpec::Fulltext {
            name: req.name.clone(),
            fields: fields.clone(),
            min_length: None, // Use default
        },
        _ => IndexSpec::Regular {
            name: req.name.clone(),
            fields: fields.clone(),
            index_type: index_type.clone(),
            unique: req.unique,
        },
    };

    collection.create_index_from_spec(&spec)?;
    propagate_index_create(&state, &db_name, &coll_name, &spec).await;

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
        propagate_index_drop(
            &state,
            &db_name,
            &coll_name,
            IndexKind::Regular,
            &index_name,
        )
        .await;
        return Ok(StatusCode::NO_CONTENT);
    }

    // Try dropping as fulltext index
    if collection.drop_fulltext_index(&index_name).is_ok() {
        // Fulltext lives in the Regular family for drop purposes.
        propagate_index_drop(
            &state,
            &db_name,
            &coll_name,
            IndexKind::Regular,
            &index_name,
        )
        .await;
        return Ok(StatusCode::NO_CONTENT);
    }

    // Try dropping as geo index
    if collection.drop_geo_index(&index_name).is_ok() {
        propagate_index_drop(&state, &db_name, &coll_name, IndexKind::Geo, &index_name).await;
        return Ok(StatusCode::NO_CONTENT);
    }

    // Try dropping as TTL index
    if collection.drop_ttl_index(&index_name).is_ok() {
        propagate_index_drop(&state, &db_name, &coll_name, IndexKind::Ttl, &index_name).await;
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
    let spec = IndexSpec::Geo {
        name: req.name.clone(),
        field: req.field.clone(),
    };
    collection.create_index_from_spec(&spec)?;
    propagate_index_create(&state, &db_name, &coll_name, &spec).await;

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
    propagate_index_drop(&state, &db_name, &coll_name, IndexKind::Geo, &index_name).await;

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

// ==================== Vector Index Handlers ====================

#[derive(Debug, Deserialize)]
pub struct CreateVectorIndexRequest {
    pub name: String,
    pub field: String,
    pub dimension: usize,
    #[serde(default)]
    pub metric: Option<String>,
    #[serde(default)]
    pub m: Option<usize>,
    #[serde(default)]
    pub ef_construction: Option<usize>,
    /// Quantization method: "none" (default) or "scalar" (4x compression)
    #[serde(default)]
    pub quantization: Option<String>,
    /// Optional text field to auto-generate embeddings from (enables native auto-embeddings).
    #[serde(default)]
    pub embedding_source: Option<String>,
    /// Optional provider for embeddings ("openai", "ollama", "gemini").
    #[serde(default)]
    pub embedding_provider: Option<String>,
    /// Optional embedding model name.
    #[serde(default)]
    pub embedding_model: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateVectorIndexResponse {
    pub name: String,
    pub field: String,
    pub dimension: usize,
    pub metric: String,
    pub quantization: String,
    #[serde(rename = "type")]
    pub index_type: String,
    pub status: String,
    pub indexed_vectors: usize,
    pub memory_bytes: usize,
    pub compression_ratio: f32,
}

#[derive(Debug, Serialize)]
pub struct ListVectorIndexesResponse {
    pub indexes: Vec<VectorIndexStats>,
}

#[derive(Debug, Deserialize)]
pub struct VectorSearchRequest {
    pub vector: Vec<f32>,
    pub limit: usize,
    /// Optional ef_search parameter for HNSW (higher = better recall, slower)
    #[serde(default)]
    pub ef_search: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct VectorSearchResult {
    pub doc_key: String,
    pub score: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct VectorSearchResponse {
    pub results: Vec<VectorSearchResult>,
    pub count: usize,
}

pub async fn create_vector_index(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<CreateVectorIndexRequest>,
) -> Result<Json<CreateVectorIndexResponse>, DbError> {
    use crate::storage::index::{VectorIndexConfig, VectorMetric, VectorQuantization};

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Parse metric
    let metric = match req.metric.as_deref() {
        Some("euclidean") => VectorMetric::Euclidean,
        Some("dot") | Some("dotproduct") => VectorMetric::DotProduct,
        _ => VectorMetric::Cosine, // default
    };

    // Parse quantization
    let quantization = match req.quantization.as_deref() {
        Some("scalar") => VectorQuantization::Scalar,
        _ => VectorQuantization::None, // default
    };

    let mut config = VectorIndexConfig::new(req.name.clone(), req.field.clone(), req.dimension)
        .with_metric(metric)
        .with_quantization(quantization);

    if let Some(m) = req.m {
        config = config.with_m(m);
    }
    if let Some(ef) = req.ef_construction {
        config = config.with_ef_construction(ef);
    }
    if let Some(src) = req.embedding_source {
        config = config.with_embedding_source(src);
    }
    if let Some(p) = req.embedding_provider {
        config = config.with_embedding_provider(p);
    }
    if let Some(mdl) = req.embedding_model {
        config = config.with_embedding_model(mdl);
    }

    let spec = IndexSpec::Vector(config.clone());
    let stats = collection.create_vector_index(config)?;
    propagate_index_create(&state, &db_name, &coll_name, &spec).await;

    let metric_str = match stats.metric {
        VectorMetric::Cosine => "cosine",
        VectorMetric::Euclidean => "euclidean",
        VectorMetric::DotProduct => "dot",
    };

    let quantization_str = match stats.quantization {
        VectorQuantization::None => "none",
        VectorQuantization::Scalar => "scalar",
    };

    Ok(Json(CreateVectorIndexResponse {
        name: stats.name,
        field: stats.field,
        dimension: stats.dimension,
        metric: metric_str.to_string(),
        quantization: quantization_str.to_string(),
        index_type: "vector".to_string(),
        status: "created".to_string(),
        indexed_vectors: stats.indexed_vectors,
        memory_bytes: stats.memory_bytes,
        compression_ratio: stats.compression_ratio,
    }))
}

pub async fn list_vector_indexes(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<ListVectorIndexesResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let indexes = collection.list_vector_indexes();
    Ok(Json(ListVectorIndexesResponse { indexes }))
}

pub async fn delete_vector_index(
    State(state): State<AppState>,
    Path((db_name, coll_name, index_name)): Path<(String, String, String)>,
) -> Result<StatusCode, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    collection.drop_vector_index(&index_name)?;
    propagate_index_drop(&state, &db_name, &coll_name, IndexKind::Vector, &index_name).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn vector_search(
    State(state): State<AppState>,
    Path((db_name, coll_name, index_name)): Path<(String, String, String)>,
    Json(req): Json<VectorSearchRequest>,
) -> Result<Json<VectorSearchResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let results = collection.vector_search(&index_name, &req.vector, req.limit, req.ef_search)?;

    // Fetch documents for each result
    let search_results: Vec<VectorSearchResult> = results
        .into_iter()
        .map(|r| {
            let document = collection.get(&r.doc_key).ok().map(|doc| doc.to_value());
            VectorSearchResult {
                doc_key: r.doc_key,
                score: r.score,
                document,
            }
        })
        .collect();

    let count = search_results.len();

    Ok(Json(VectorSearchResponse {
        results: search_results,
        count,
    }))
}

/// Response for quantize operation
#[derive(Debug, Serialize)]
pub struct QuantizeVectorIndexResponse {
    pub name: String,
    pub vectors_quantized: usize,
    pub memory_before: usize,
    pub memory_after: usize,
    pub compression_ratio: f32,
    pub status: String,
}

/// Quantize an existing vector index for memory compression
pub async fn quantize_vector_index(
    State(state): State<AppState>,
    Path((db_name, coll_name, index_name)): Path<(String, String, String)>,
) -> Result<Json<QuantizeVectorIndexResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Get the index and quantize it
    // Default to Scalar quantization for this endpoint
    let stats = collection.quantize_vector_index(
        &index_name,
        crate::storage::index::VectorQuantization::Scalar,
    )?;

    Ok(Json(QuantizeVectorIndexResponse {
        name: index_name,
        vectors_quantized: 0, // Not provided by stats yet, TODO
        memory_before: stats.original_size,
        memory_after: stats.compressed_size,
        compression_ratio: stats.compression_ratio,
        status: "quantized".to_string(),
    }))
}

/// Response for dequantize operation
#[derive(Debug, Serialize)]
pub struct DequantizeVectorIndexResponse {
    pub name: String,
    pub memory_before: usize,
    pub memory_after: usize,
    pub status: String,
}

/// Dequantize an existing vector index (restore to f32 precision)
pub async fn dequantize_vector_index(
    State(state): State<AppState>,
    Path((db_name, coll_name, index_name)): Path<(String, String, String)>,
) -> Result<Json<DequantizeVectorIndexResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Get the index and dequantize it
    collection.dequantize_vector_index(&index_name)?;

    Ok(Json(DequantizeVectorIndexResponse {
        name: index_name,
        memory_before: 0, // Stats not returned by dequantize
        memory_after: 0,
        status: "dequantized".to_string(),
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
    // TTL is the index family where non-replication was worst: the expiry
    // sweep skips a collection with no TTL index, so before this the same
    // documents expired on one node and lived forever on every other.
    let spec = IndexSpec::Ttl {
        name: req.name.clone(),
        field: req.field.clone(),
        expire_after_seconds: req.expire_after_seconds,
    };
    collection.create_index_from_spec(&spec)?;
    propagate_index_create(&state, &db_name, &coll_name, &spec).await;

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
    propagate_index_drop(&state, &db_name, &coll_name, IndexKind::Ttl, &index_name).await;
    Ok(StatusCode::NO_CONTENT)
}

// ==================== Hybrid Search ====================

#[derive(Debug, Deserialize)]
pub struct HybridSearchRequest {
    pub vector: Vec<f32>,
    pub text_query: String,
    pub vector_index: String,
    pub fulltext_field: String,
    #[serde(default)]
    pub vector_weight: Option<f32>, // 0.0 to 1.0 (default 0.5)
    #[serde(default)]
    pub text_weight: Option<f32>, // 0.0 to 1.0 (default 0.5)
    #[serde(default)]
    pub limit: Option<usize>, // default 10
    #[serde(default)]
    pub fusion: Option<String>, // "weighted" (default) or "rrf"
}

#[derive(Debug, Serialize)]
pub struct HybridSearchResponse {
    pub results: Vec<crate::storage::HybridSearchResult>,
    pub count: usize,
}

pub async fn hybrid_search(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<HybridSearchRequest>,
) -> Result<Json<HybridSearchResponse>, DbError> {
    use crate::storage::{FusionMethod, HybridSearchOptions};

    let fusion = match req.fusion.as_deref() {
        None => FusionMethod::default(),
        Some(s) => FusionMethod::parse(s).ok_or_else(|| {
            DbError::BadRequest(format!(
                "Invalid fusion method '{}': expected \"weighted\" or \"rrf\"",
                s
            ))
        })?,
    };
    let defaults = HybridSearchOptions::default();
    let opts = HybridSearchOptions {
        vector_weight: req.vector_weight.unwrap_or(defaults.vector_weight),
        text_weight: req.text_weight.unwrap_or(defaults.text_weight),
        limit: req.limit.unwrap_or(defaults.limit),
        fusion,
    };

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let results = collection.hybrid_search(
        &req.vector_index,
        &req.fulltext_field,
        &req.vector,
        &req.text_query,
        &opts,
    )?;

    let count = results.len();
    Ok(Json(HybridSearchResponse { results, count }))
}
