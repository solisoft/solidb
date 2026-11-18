//! HTTP API Handlers for Columnar Collections
//!
//! Provides endpoints for creating, querying, and managing columnar collections
//! optimized for analytics and reporting workloads.
//!
//! # Overview
//!
//! Columnar collections store data in a column-oriented format, which is optimized for:
//! - **Analytics queries** - Aggregations (SUM, AVG, COUNT, MIN, MAX) run efficiently
//! - **Read-heavy workloads** - Column pruning reads only needed data
//! - **Compression** - LZ4 compression provides 2-4x space savings
//! - **Time-series data** - Efficient storage and querying of metrics
//!
//! # API Endpoints
//!
//! | Method | Endpoint | Description |
//! |--------|----------|-------------|
//! | POST | `/_api/database/{db}/columnar` | Create a columnar collection |
//! | GET | `/_api/database/{db}/columnar` | List all columnar collections |
//! | GET | `/_api/database/{db}/columnar/{collection}` | Get collection metadata |
//! | DELETE | `/_api/database/{db}/columnar/{collection}` | Delete a collection |
//! | POST | `/_api/database/{db}/columnar/{collection}/insert` | Insert rows |
//! | POST | `/_api/database/{db}/columnar/{collection}/aggregate` | Run aggregation |
//! | POST | `/_api/database/{db}/columnar/{collection}/query` | Query with filters |
//!
//! # Column Types
//!
//! - `INT64` / `INTEGER` / `INT` / `BIGINT` - 64-bit integers
//! - `FLOAT64` / `FLOAT` / `DOUBLE` / `NUMBER` - 64-bit floating point
//! - `STRING` / `TEXT` / `VARCHAR` - UTF-8 strings
//! - `BOOL` / `BOOLEAN` - Boolean values
//! - `TIMESTAMP` / `DATETIME` / `DATE` - ISO 8601 timestamps
//! - `JSON` / `OBJECT` / `ARRAY` - Nested JSON data
//!
//! # Compression
//!
//! - `lz4` (default) - Fast compression with good ratios
//! - `none` - No compression (for already-compressed data)
//!
//! # Example Usage
//!
//! ## Create a columnar collection
//!
//! ```json
//! POST /_api/database/mydb/columnar
//! {
//!   "name": "metrics",
//!   "columns": [
//!     {"name": "timestamp", "type": "TIMESTAMP"},
//!     {"name": "value", "type": "FLOAT64"},
//!     {"name": "host", "type": "STRING"}
//!   ],
//!   "compression": "lz4"
//! }
//! ```
//!
//! ## Insert rows
//!
//! ```json
//! POST /_api/database/mydb/columnar/metrics/insert
//! {
//!   "rows": [
//!     {"timestamp": "2024-01-15T10:00:00Z", "value": 42.5, "host": "server1"},
//!     {"timestamp": "2024-01-15T10:01:00Z", "value": 43.2, "host": "server1"}
//!   ]
//! }
//! ```
//!
//! ## Run aggregation
//!
//! ```json
//! POST /_api/database/mydb/columnar/metrics/aggregate
//! {
//!   "column": "value",
//!   "operation": "AVG",
//!   "group_by": ["host"]
//! }
//! ```
//!
//! ## Query with filter
//!
//! ```json
//! POST /_api/database/mydb/columnar/metrics/query
//! {
//!   "columns": ["timestamp", "value"],
//!   "filter": {"column": "host", "op": "EQ", "value": "server1"},
//!   "limit": 100
//! }
//! ```

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::DbError;
use crate::server::handlers::AppState;
use crate::storage::columnar::{
    AggregateOp, ColumnDef, ColumnFilter, ColumnarCollection, CompressionType,
};

// ==================== Request/Response Types ====================

#[derive(Debug, Deserialize)]
pub struct CreateColumnarRequest {
    pub name: String,
    pub columns: Vec<ColumnDefRequest>,
    #[serde(default)]
    pub compression: Option<String>,
    /// Split the collection into this many shards (card #899), by the
    /// column `shard_key`, each on `replication_factor` nodes.
    #[serde(default)]
    pub num_shards: Option<u16>,
    #[serde(default)]
    pub shard_key: Option<String>,
    #[serde(default)]
    pub replication_factor: Option<u16>,
}

/// A request a coordinating node sends a shard's node: written here only,
/// never logged (`sharding::columnar`).
fn shard_direct(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("X-Shard-Direct")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "true")
}

/// Whether the columnar collection `name` is sharded.
fn sharded(
    db: &crate::storage::Database,
    name: &str,
) -> Option<crate::storage::columnar::ColumnarShards> {
    crate::storage::ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())
        .ok()
        .and_then(|col| col.sharding())
}

#[derive(Debug, Deserialize)]
pub struct ColumnDefRequest {
    pub name: String,
    #[serde(rename = "type")]
    pub data_type: String,
    #[serde(default)]
    pub nullable: bool,
    #[serde(default)]
    pub indexed: bool,
}

#[derive(Debug, Serialize)]
pub struct CreateColumnarResponse {
    pub status: String,
    pub name: String,
    pub columns: usize,
}

#[derive(Debug, Deserialize)]
pub struct InsertColumnarRequest {
    pub rows: Vec<Value>,
}

#[derive(Debug, Serialize)]
pub struct InsertColumnarResponse {
    pub status: String,
    pub inserted: usize,
    pub ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AggregateRequest {
    pub column: String,
    pub operation: String,
    #[serde(default)]
    pub group_by: Option<Vec<String>>,
    /// A filter as `/query` takes: only the rows it keeps are aggregated.
    #[serde(default)]
    pub filter: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct AggregateResponse {
    pub result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<Value>>,
}

#[derive(Debug, Deserialize)]
pub struct QueryColumnarRequest {
    /// The columns to return; all of them when left out.
    #[serde(default)]
    pub columns: Option<Vec<String>>,
    /// A comparison `{"column", "op", "value"}`, or `{"and": [..]}` /
    /// `{"or": [..]}` of filters.
    #[serde(default)]
    pub filter: Option<Value>,
    /// `"column"` or `"column ASC|DESC"`, on a returned column.
    #[serde(default)]
    pub order_by: Option<String>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct QueryColumnarResponse {
    pub result: Vec<Value>,
    pub count: usize,
}

// ==================== Handlers ====================

/// A columnar write is stored at once and is not undone by a rollback: one
/// sent as part of a transaction (`X-Transaction-ID`, the header the
/// document routes stage a write with) is refused, as SDBQL refuses an
/// `INSERT INTO` a columnar collection inside one. It used to be stored
/// as if the header were not there.
/// Whether the caller is an admin, whom a row policy does not bind; a
/// request with no claims comes from an instance that authenticates
/// nothing.
fn admin(claims: &Option<axum::Extension<crate::server::auth::Claims>>) -> bool {
    claims.as_ref().is_none_or(|claims| {
        crate::server::handlers::query::principal_from_claims(&claims.0).can_admin
    })
}

fn outside_a_transaction(headers: &axum::http::HeaderMap, what: &str) -> Result<(), DbError> {
    if headers.contains_key("X-Transaction-ID") {
        return Err(DbError::OperationNotSupported(format!(
            "{} a columnar collection is not supported in a transaction",
            what
        )));
    }
    Ok(())
}

/// Create a new columnar collection
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar`
///
/// # Request Body
/// - `name` - Collection name (required)
/// - `columns` - Array of column definitions (required)
///   - `name` - Column name
///   - `type` - Data type (INT64, FLOAT64, STRING, BOOL, TIMESTAMP, JSON)
///   - `nullable` - Whether column allows null values (default: false)
///   - `indexed` - Whether to create an index (default: false)
/// - `compression` - Compression type: "lz4" (default) or "none"
///
/// # Response
/// ```json
/// {"status": "created", "name": "metrics", "columns": 3}
/// ```
pub async fn create_columnar_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CreateColumnarRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Parse column definitions
    let columns: Vec<ColumnDef> = req
        .columns
        .into_iter()
        .map(|c| {
            Ok(ColumnDef {
                data_type: parse_column_type(&c.data_type)?,
                name: c.name,
                nullable: c.nullable,
                // An index is made by `POST …/index`: marking the column
                // here made none, and then refused to make one.
                indexed: false,
                index_type: None,
            })
        })
        .collect::<Result<_, DbError>>()?;

    // Parse compression type
    let compression = match req.compression.as_deref() {
        Some("none") => CompressionType::None,
        Some("lz4") | None => CompressionType::Lz4,
        Some(other) => {
            return Err(DbError::BadRequest(format!(
                "Unknown compression type: {}. Supported: none, lz4",
                other
            )))
        }
    };

    let count = columns.len();
    if let Some(num_shards) = req.num_shards.filter(|n| *n > 0) {
        let shard_key = req.shard_key.clone().ok_or_else(|| {
            DbError::BadRequest(
                "A sharded columnar collection needs a shard_key column".to_string(),
            )
        })?;
        let name = req.name.clone();
        let replication_factor = req.replication_factor.unwrap_or(1);
        let state = state.clone();
        let col = tokio::task::spawn_blocking(move || {
            crate::sharding::columnar::create(
                state.shard_coordinator.as_deref(),
                &db,
                state.replication_log.as_deref(),
                &name,
                columns,
                compression,
                num_shards,
                &shard_key,
                replication_factor,
            )
        })
        .await
        .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
        return Ok(Json(CreateColumnarResponse {
            status: "created".to_string(),
            name: col.name,
            columns: count,
        }));
    }
    // Created here and logged for the peers; a shard a coordinator asks
    // for is not logged.
    let log = if shard_direct(&headers) {
        None
    } else {
        state.replication_log.as_deref()
    };
    let col = crate::sync::columnar::create(&db, log, &req.name, columns, compression)?;

    // Store metadata reference in a system collection if needed
    // For now, the metadata is stored in the column family itself

    Ok(Json(CreateColumnarResponse {
        status: "created".to_string(),
        name: col.name,
        columns: count,
    }))
}

/// List all columnar collections in a database
///
/// # Endpoint
/// `GET /_api/database/{db}/columnar`
///
/// # Response
/// ```json
/// {"collections": [{"name": "metrics", "row_count": 100, ...}], "count": 1}
/// ```
pub async fn list_columnar_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Find all column families that start with _columnar_
    let collection_names: Vec<String> = db
        .list_collections()
        .into_iter()
        .filter(|name| name.starts_with("_columnar_"))
        .map(|name| name.trim_start_matches("_columnar_").to_string())
        .collect();

    // Load metadata for each collection
    let mut collections = Vec::new();
    for name in &collection_names {
        if let Ok(col) = ColumnarCollection::load(name.clone(), &db_name, db.db_arc()) {
            if let Ok(meta) = col.metadata() {
                collections.push(serde_json::json!({
                    "name": name,
                    "row_count": meta.row_count,
                    "columns": meta.columns,
                    "compression": format!("{:?}", meta.compression),
                    "created_at": meta.created_at
                }));
            }
        }
    }

    Ok(Json(serde_json::json!({
        "collections": collections,
        "count": collections.len()
    })))
}

/// Get columnar collection metadata and statistics
///
/// # Endpoint
/// `GET /_api/database/{db}/columnar/{collection}`
///
/// # Response
/// Returns collection metadata including columns, row count, compression settings,
/// and storage statistics (compressed/uncompressed sizes, compression ratio).
pub async fn get_columnar_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;

    let col = ColumnarCollection::load(collection, &db_name, db.db_arc())?;
    let meta = col.metadata()?;
    let stats = col.stats()?;

    Ok(Json(serde_json::json!({
        "name": meta.name,
        "columns": meta.columns,
        "row_count": meta.row_count,
        "compression": meta.compression,
        "created_at": meta.created_at,
        "last_updated_at": meta.last_updated_at,
        "retention_secs": meta.retention_secs,
        "branch_of": meta.parent,
        "stats": {
            "compressed_size_bytes": stats.compressed_size_bytes,
            "uncompressed_size_bytes": stats.uncompressed_size_bytes,
            "compression_ratio": stats.compression_ratio
        }
    })))
}

/// Delete a columnar collection
///
/// # Endpoint
/// `DELETE /_api/database/{db}/columnar/{collection}`
///
/// # Response
/// ```json
/// {"status": "deleted", "name": "metrics"}
/// ```
pub async fn delete_columnar_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let log = if shard_direct(&headers) {
        None
    } else {
        state.replication_log.as_deref()
    };
    if let Some(shards) = sharded(&db, &collection) {
        let state = state.clone();
        let name = collection.clone();
        tokio::task::spawn_blocking(move || {
            crate::sharding::columnar::drop(
                state.shard_coordinator.as_deref(),
                &db,
                state.replication_log.as_deref(),
                &name,
                &shards,
            )
        })
        .await
        .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
        return Ok(Json(
            serde_json::json!({"status": "deleted", "name": collection}),
        ));
    }
    crate::sync::columnar::drop(&db, log, &collection)?;

    Ok(Json(serde_json::json!({
        "status": "deleted",
        "name": collection
    })))
}

/// Insert rows into a columnar collection
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/insert`
///
/// # Request Body
/// - `rows` - Array of JSON objects to insert
///
/// # Response
/// ```json
/// {"status": "ok", "inserted": 100}
/// ```
///
/// # Notes
/// - Rows are stored in columnar format with LZ4 compression
/// - Missing columns are stored as null
/// - Supports bulk inserts for efficiency
pub async fn insert_columnar_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    Json(req): Json<InsertColumnarRequest>,
) -> Result<impl IntoResponse, DbError> {
    outside_a_transaction(&headers, "inserting into")?;
    let db = state.storage.get_database(&db_name)?;
    if sharded(&db, &collection).is_some() {
        let state = state.clone();
        let rows = req.rows;
        let ids = tokio::task::spawn_blocking(move || {
            crate::sharding::columnar::insert(
                state.shard_coordinator.as_deref(),
                &db,
                &collection,
                rows,
            )
        })
        .await
        .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
        return Ok(Json(InsertColumnarResponse {
            status: "ok".to_string(),
            inserted: ids.len(),
            ids,
        }));
    }

    let inserted_ids = crate::sync::columnar::insert(
        &db,
        state.replication_log.as_deref(),
        &collection,
        req.rows,
    )?;

    Ok(Json(InsertColumnarResponse {
        status: "ok".to_string(),
        inserted: inserted_ids.len(),
        ids: inserted_ids,
    }))
}

/// Delete one row by its id
///
/// # Endpoint
/// `DELETE /_api/database/{db}/columnar/{collection}/row/{id}`
///
/// # Response
/// ```json
/// {"status": "deleted", "id": "01a1…"}
/// ```
/// `404` when the collection has no row of that id.
pub async fn delete_columnar_row_handler(
    State(state): State<AppState>,
    Path((db_name, collection, id)): Path<(String, String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, DbError> {
    outside_a_transaction(&headers, "deleting from")?;
    let db = state.storage.get_database(&db_name)?;
    let deleted = crate::sync::columnar::delete_rows(
        &db,
        state.replication_log.as_deref(),
        &collection,
        std::slice::from_ref(&id),
    )?;
    if deleted.is_empty() {
        return Err(DbError::DocumentNotFound(id));
    }
    Ok(Json(serde_json::json!({"status": "deleted", "id": id})))
}

#[derive(Debug, Deserialize)]
pub struct DeleteColumnarRowsRequest {
    /// Row ids, as an insert returned them.
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    /// A filter as `/query` takes, `{"and": […]}` / `{"or": […]}` included.
    #[serde(default)]
    pub filter: Option<Value>,
}

/// Delete rows, by id or by filter
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/delete`
///
/// # Request Body
/// Exactly one of `ids` (an array of row ids) and `filter`
/// (`{"column": c, "op": o, "value": v}`, or `{"and": [..]}` / `{"or": [..]}`).
/// Neither is refused: emptying a collection is `PUT …/truncate`.
///
/// # Response
/// ```json
/// {"status": "ok", "deleted": 12}
/// ```
pub async fn delete_columnar_rows_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    Json(req): Json<DeleteColumnarRowsRequest>,
) -> Result<impl IntoResponse, DbError> {
    outside_a_transaction(&headers, "deleting from")?;
    let db = state.storage.get_database(&db_name)?;
    let log = state.replication_log.as_deref();
    let deleted = match (req.ids, req.filter) {
        (Some(ids), None) => crate::sync::columnar::delete_rows(&db, log, &collection, &ids)?.len(),
        (None, Some(filter)) => {
            let filter = ColumnFilter::from_json(&filter)?;
            crate::sync::columnar::delete_where(&db, log, &collection, &filter)?
        }
        _ => {
            return Err(DbError::BadRequest(
                "give either 'ids' or 'filter' (to empty the collection: PUT …/truncate)"
                    .to_string(),
            ))
        }
    };
    Ok(Json(
        serde_json::json!({"status": "ok", "deleted": deleted}),
    ))
}

/// Delete every row of a columnar collection, keeping its columns
///
/// # Endpoint
/// `PUT /_api/database/{db}/columnar/{collection}/truncate` (Admin)
///
/// # Response
/// ```json
/// {"status": "truncated", "name": "metrics"}
/// ```
pub async fn truncate_columnar_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, DbError> {
    outside_a_transaction(&headers, "truncating")?;
    let db = state.storage.get_database(&db_name)?;
    crate::sync::columnar::truncate(&db, state.replication_log.as_deref(), &collection)?;
    Ok(Json(
        serde_json::json!({"status": "truncated", "name": collection}),
    ))
}

/// Merge the blocks that are smaller than they could be (many small
/// inserts, rows deleted since). The rows and what queries answer do not
/// change.
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/compact`
pub async fn compact_columnar_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let (before, after) =
        tokio::task::spawn_blocking(move || crate::sync::columnar::compact(&db, &collection))
            .await
            .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    Ok(Json(serde_json::json!({
        "status": "ok",
        "blocks_before": before,
        "blocks_after": after
    })))
}

/// `{"name": "new_name"}`.
#[derive(Debug, Deserialize)]
pub struct RenameColumnRequest {
    pub name: String,
}

/// Add a column: `{"name", "type"}`. The rows already stored read null in
/// it, so it is nullable whatever the request says of that.
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/columns`
pub async fn add_columnar_column_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    Json(req): Json<ColumnDefRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let column = ColumnDef {
        name: req.name.clone(),
        data_type: parse_column_type(&req.data_type)?,
        nullable: true,
        indexed: false,
        index_type: None,
    };
    crate::sync::columnar::add_column(&db, state.replication_log.as_deref(), &collection, column)?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({"status": "added", "column": req.name})),
    ))
}

/// Drop a column and its values.
///
/// # Endpoint
/// `DELETE /_api/database/{db}/columnar/{collection}/columns/{column}`
pub async fn drop_columnar_column_handler(
    State(state): State<AppState>,
    Path((db_name, collection, column)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let log = state.replication_log.clone();
    let dropped = column.clone();
    tokio::task::spawn_blocking(move || {
        crate::sync::columnar::drop_column(&db, log.as_deref(), &collection, &dropped)
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    Ok(Json(
        serde_json::json!({"status": "dropped", "column": column}),
    ))
}

/// Rename a column.
///
/// # Endpoint
/// `PUT /_api/database/{db}/columnar/{collection}/columns/{column}`
pub async fn rename_columnar_column_handler(
    State(state): State<AppState>,
    Path((db_name, collection, column)): Path<(String, String, String)>,
    Json(req): Json<RenameColumnRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let log = state.replication_log.clone();
    let (from, to) = (column.clone(), req.name.clone());
    tokio::task::spawn_blocking(move || {
        crate::sync::columnar::rename_column(&db, log.as_deref(), &collection, &from, &to)
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    Ok(Json(
        serde_json::json!({"status": "renamed", "column": req.name, "was": column}),
    ))
}

/// `{"seconds": n}`, or `{"seconds": null}` to keep every row.
#[derive(Debug, Deserialize)]
pub struct RetentionRequest {
    #[serde(default)]
    pub seconds: Option<u64>,
}

/// Set how long a columnar collection keeps its rows, counted from the
/// instant each row was inserted.
///
/// # Endpoint
/// `PUT /_api/database/{db}/columnar/{collection}/retention`
pub async fn set_columnar_retention_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    Json(req): Json<RetentionRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    crate::sync::columnar::set_retention(
        &db,
        state.replication_log.as_deref(),
        &collection,
        req.seconds,
    )?;
    Ok(Json(serde_json::json!({
        "name": collection,
        "retention_secs": req.seconds
    })))
}

/// Remove now the rows a collection's retention no longer keeps, instead
/// of waiting for the next pass of the background worker.
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/prune`
pub async fn prune_columnar_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let deleted =
        tokio::task::spawn_blocking(move || crate::sync::columnar::prune(&db, &collection))
            .await
            .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    Ok(Json(
        serde_json::json!({"status": "ok", "deleted": deleted}),
    ))
}

/// Execute aggregation on a columnar collection
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/aggregate`
///
/// # Request Body
/// - `column` - Column to aggregate (required)
/// - `operation` - Aggregation operation: SUM, AVG, COUNT, MIN, MAX, COUNT_DISTINCT
/// - `group_by` - Optional array of columns to group by
/// - `filter` - Optional filter, as `/query` takes (`{"column", "op",
///   "value"}`, or `{"and": [..]}` / `{"or": [..]}`): the rows it keeps
///   are the ones aggregated
///
/// # Response (simple aggregation)
/// ```json
/// {"result": 42.5, "groups": null}
/// ```
///
/// # Response (with group_by)
/// ```json
/// {"result": null, "groups": [{"host": "server1", "_agg": 42.5}, ...]}
/// ```
pub async fn aggregate_columnar_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    claims: Option<axum::Extension<crate::server::auth::Claims>>,
    Json(req): Json<AggregateRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    db.columnar_policy_lets(&collection, admin(&claims))?;
    crate::sync::columnar::refuse_if_sharded(&db, &collection, "this endpoint (use SDBQL)")?;
    let op = AggregateOp::from_str(&req.operation).ok_or_else(|| {
        DbError::BadRequest(format!(
            "Unknown aggregation operation: {}. Supported: SUM, AVG, COUNT, MIN, MAX, \
             COUNT_DISTINCT. STDDEV, VARIANCE, MEDIAN, PERCENTILE and APPROX_COUNT_DISTINCT \
             are aggregates of a query: FOR r IN <collection> COLLECT ... AGGREGATE ...",
            req.operation
        ))
    })?;
    let filter = req
        .filter
        .as_ref()
        .map(ColumnFilter::from_json)
        .transpose()?;

    // Off the async workers: a pass over every row of a large collection.
    let answer = tokio::task::spawn_blocking(move || -> Result<AggregateResponse, DbError> {
        use crate::storage::columnar::GroupByColumn;
        let col = ColumnarCollection::load(collection, &db_name, db.db_arc())?;
        let Some(group_cols) = req.group_by else {
            let result = match &filter {
                Some(filter) => col.aggregate_where(filter, &req.column, op)?,
                None => col.aggregate(&req.column, op)?,
            };
            return Ok(AggregateResponse {
                result,
                groups: None,
            });
        };
        let group_defs: Vec<GroupByColumn> =
            group_cols.into_iter().map(GroupByColumn::Simple).collect();
        let groups = match &filter {
            Some(filter) => col.group_by_where(filter, &group_defs, &req.column, op)?,
            None => col.group_by(&group_defs, &req.column, op)?,
        };
        Ok(AggregateResponse {
            result: Value::Null,
            groups: Some(groups),
        })
    })
    .await
    .map_err(|e| DbError::InternalError(e.to_string()))??;
    Ok(Json(answer))
}

/// Query a columnar collection with optional filtering
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/query`
///
/// # Request Body
/// - `columns` - Array of column names to return (required)
/// - `filter` - Optional filter object:
///   - `column` - Column to filter on
///   - `op` - Operator: EQ, NE, GT, GTE, LT, LTE, IN
///   - `value` - Value to compare against
/// - `limit` - Optional maximum number of rows to return
///
/// # Response
/// ```json
/// {"result": [{"name": "Alice", "age": 30}, ...], "count": 100}
/// ```
///
/// # Notes
/// - Column pruning only reads requested columns
/// - Filters are applied before projection for efficiency
pub async fn query_columnar_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    claims: Option<axum::Extension<crate::server::auth::Claims>>,
    Json(req): Json<QueryColumnarRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    db.columnar_policy_lets(&collection, admin(&claims))?;
    crate::sync::columnar::refuse_if_sharded(&db, &collection, "this endpoint (use SDBQL)")?;

    let filter = req
        .filter
        .as_ref()
        .map(ColumnFilter::from_json)
        .transpose()?;
    let results = db.query_columnar_rows(
        &collection,
        req.columns,
        filter,
        req.order_by,
        req.offset.unwrap_or(0),
        req.limit,
    )?;

    let count = results.len();

    Ok(Json(QueryColumnarResponse {
        result: results,
        count,
    }))
}

// ==================== Helper Functions ====================

fn parse_column_type(type_str: &str) -> Result<crate::storage::columnar::ColumnType, DbError> {
    use crate::storage::columnar::ColumnType;

    Ok(match type_str.to_uppercase().as_str() {
        "INT64" | "INTEGER" | "INT" | "BIGINT" => ColumnType::Int64,
        "FLOAT64" | "FLOAT" | "DOUBLE" | "NUMBER" => ColumnType::Float64,
        "STRING" | "TEXT" | "VARCHAR" => ColumnType::String,
        "BOOL" | "BOOLEAN" => ColumnType::Bool,
        "TIMESTAMP" | "DATETIME" | "DATE" => ColumnType::Timestamp,
        "JSON" | "OBJECT" | "ARRAY" => ColumnType::Json,
        // It used to become a string column without a word.
        _ => {
            return Err(DbError::BadRequest(format!(
                "Unknown column type: {}. Supported: int64, float64, string, bool, timestamp, json",
                type_str
            )))
        }
    })
}

// ==================== Index Handlers ====================

#[derive(Debug, Deserialize)]
pub struct CreateIndexRequest {
    pub column: String,
    #[serde(default)]
    pub index_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateIndexResponse {
    pub status: String,
    pub column: String,
    pub index_type: String,
}

/// What `POST …/index` takes in its query string.
#[derive(Debug, Deserialize, Default)]
pub struct CreateIndexParams {
    /// `false`: answer at once with the build's id, and build in the
    /// background. Left out or `true`: answer when the index is built.
    #[serde(default)]
    pub wait: Option<bool>,
    /// `true`: build the index again in place of the one the column has.
    #[serde(default)]
    pub rebuild: Option<bool>,
}

/// A background build as the endpoints report it.
fn build_json(build: &crate::storage::columnar::IndexBuild) -> Value {
    use crate::storage::columnar::BuildState;
    let (rows_done, rows_total) = build.progress();
    let (status, error) = match build.state() {
        BuildState::Building => ("building", None),
        BuildState::Done => ("created", None),
        BuildState::Failed(error) => ("failed", Some(error)),
    };
    let mut answer = serde_json::json!({
        "status": status,
        "job": build.id,
        "column": build.column,
        "index_type": format!("{:?}", build.index_type).to_lowercase(),
        "rows_done": rows_done,
        "rows_total": rows_total,
        "started_at": build.started_at,
    });
    if let Some(error) = error {
        answer["error"] = Value::String(error);
    }
    answer
}

/// Create an index on a columnar collection column
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/index`
///
/// # Request Body
/// - `column` - Column name to index (required)
/// - `index_type` - Index type: "sorted" (default), "hash", "bitmap" or "bloom"
///
/// # Query string
/// - `wait=false` - answer at once, `202` with the build's `job` id; the
///   build is followed with `GET …/index/{column}`. By default the answer
///   comes when the index is built.
/// - `rebuild=true` - build the index again in place of the one the
///   column has, from the rows as they are now.
///
/// # Response
/// ```json
/// {"status": "created", "column": "host", "index_type": "sorted"}
/// ```
/// With `wait=false`:
/// ```json
/// {"status": "building", "job": "0193…", "column": "host", "index_type": "sorted",
///  "rows_done": 0, "rows_total": 3650000, "started_at": 1794300000}
/// ```
pub async fn create_columnar_index_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<CreateIndexParams>,
    Json(req): Json<CreateIndexRequest>,
) -> Result<axum::response::Response, DbError> {
    let db = state.storage.get_database(&db_name)?;

    let index_type = match req.index_type.as_deref() {
        Some("hash") => crate::storage::columnar::ColumnarIndexType::Hash,
        Some("sorted") | None => crate::storage::columnar::ColumnarIndexType::Sorted,
        Some("bitmap") => crate::storage::columnar::ColumnarIndexType::Bitmap,
        Some("minmax") => {
            return Err(DbError::BadRequest(
                "The minmax index type was removed: a range is answered from the bounds \
                 each block keeps of its column, and needs no index. Supported: sorted, \
                 hash, bitmap, bloom"
                    .to_string(),
            ))
        }
        Some("bloom") => crate::storage::columnar::ColumnarIndexType::Bloom,
        Some(other) => {
            return Err(DbError::BadRequest(format!(
                "Unknown index type: {}. Supported: sorted, hash, bitmap, bloom",
                other
            )))
        }
    };
    let rebuild = params.rebuild.unwrap_or(false);
    let log = state.replication_log.clone();

    if params.wait == Some(false) {
        let build = crate::sync::columnar::start_index_build(
            &db,
            log.as_deref(),
            &collection,
            &req.column,
            index_type,
            rebuild,
        )?;
        return Ok((axum::http::StatusCode::ACCEPTED, Json(build_json(&build))).into_response());
    }

    // Off the async workers: the build reads every row.
    let column = req.column.clone();
    let kind = index_type.clone();
    tokio::task::spawn_blocking(move || {
        if rebuild {
            crate::sync::columnar::rebuild_index(&db, log.as_deref(), &collection, &column, kind)
        } else {
            crate::sync::columnar::create_index(&db, log.as_deref(), &collection, &column, kind)
        }
    })
    .await
    .map_err(|e| DbError::InternalError(e.to_string()))??;

    Ok(Json(CreateIndexResponse {
        status: "created".to_string(),
        column: req.column,
        index_type: format!("{:?}", index_type).to_lowercase(),
    })
    .into_response())
}

/// The index of one column, or the build of it
///
/// # Endpoint
/// `GET /_api/database/{db}/columnar/{collection}/index/{column}`
///
/// # Response
/// While it is being built (`status` is then `building`, and `failed`
/// with an `error` when the build gave up):
/// ```json
/// {"status": "building", "job": "0193…", "column": "host", "index_type": "sorted",
///  "rows_done": 1310720, "rows_total": 3650000, "started_at": 1794300000}
/// ```
/// Once it exists:
/// ```json
/// {"status": "created", "column": "host", "index_type": "sorted",
///  "created_at": 1794300004, "rows": 3650000, "size_bytes": 5123456}
/// ```
/// `404` for a column with neither.
pub async fn get_columnar_index_handler(
    State(state): State<AppState>,
    Path((db_name, collection, column)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, DbError> {
    use crate::storage::columnar::BuildState;
    let db = state.storage.get_database(&db_name)?;
    let col = ColumnarCollection::load(collection, &db_name, db.db_arc())?;
    let build = col.index_build(&column);
    if let Some(build) = build.as_ref().filter(|b| b.state() != BuildState::Done) {
        // A build that failed and was followed by an index tells of the
        // index.
        if build.state() == BuildState::Building || !col.has_index(&column) {
            return Ok(Json(build_json(build)));
        }
    }
    let Some(index) = col
        .list_indexes()?
        .into_iter()
        .find(|index| index.column == column)
    else {
        return Err(DbError::DocumentNotFound(format!(
            "Column '{}' has no index",
            column
        )));
    };
    let mut answer = index_json(&col, &index);
    answer["status"] = Value::String("created".to_string());
    if let Some(build) = build {
        answer["job"] = Value::String(build.id.clone());
    }
    Ok(Json(answer))
}

/// An index as the endpoints report it.
fn index_json(
    col: &ColumnarCollection,
    index: &crate::storage::columnar::ColumnarIndexMeta,
) -> Value {
    serde_json::json!({
        "column": index.column,
        "index_type": format!("{:?}", index.index_type).to_lowercase(),
        "created_at": index.created_at,
        // The rows it answers for; those after them are read in their blocks.
        "rows": col.index_coverage(&index.column),
        // Of how many: an index holds the rows present when it was built,
        // and nothing rebuilds it. `missing` says how far behind it is.
        "collection_rows": col.count(),
        "missing": (col.count() as u64).saturating_sub(col.index_coverage(&index.column)),
        "size_bytes": index.size_bytes,
    })
}

/// List indexes on a columnar collection
///
/// # Endpoint
/// `GET /_api/database/{db}/columnar/{collection}/indexes`
///
/// # Response
/// ```json
/// {"indexes": [{"column": "host", "index_type": "sorted", "created_at": 1234567890,
///               "rows": 3650000, "size_bytes": 5123456}],
///  "count": 1,
///  "builds": [{"status": "building", "job": "0193…", "column": "day", …}]}
/// ```
/// `builds` holds the background builds that have not ended in an index.
pub async fn list_columnar_indexes_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
) -> Result<impl IntoResponse, DbError> {
    use crate::storage::columnar::BuildState;
    let db = state.storage.get_database(&db_name)?;

    let col = ColumnarCollection::load(collection, &db_name, db.db_arc())?;
    let indexes_json: Vec<Value> = col
        .list_indexes()?
        .iter()
        .map(|index| index_json(&col, index))
        .collect();
    let builds: Vec<Value> = col
        .index_builds()
        .iter()
        .filter(|build| build.state() != BuildState::Done)
        .map(|build| build_json(build))
        .collect();

    Ok(Json(serde_json::json!({
        "indexes": indexes_json,
        "count": indexes_json.len(),
        "builds": builds
    })))
}

/// Delete an index from a columnar collection
///
/// # Endpoint
/// `DELETE /_api/database/{db}/columnar/{collection}/index/{column}`
///
/// # Response
/// ```json
/// {"status": "deleted", "column": "host"}
/// ```
pub async fn delete_columnar_index_handler(
    State(state): State<AppState>,
    Path((db_name, collection, column)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;

    crate::sync::columnar::drop_index(&db, state.replication_log.as_deref(), &collection, &column)?;

    Ok(Json(serde_json::json!({
        "status": "deleted",
        "column": column
    })))
}

/// `{"tag": "q3"}`.
#[derive(Debug, Deserialize)]
pub struct TagRequest {
    pub tag: String,
}

/// Take a tag of a columnar collection: a name for its rows as they are
/// now, read later with `FOR r IN c OPTIONS {tag: "q3"}` and restored with
/// `…/tags/{tag}/restore`. Nothing is copied but the rows not yet sealed
/// into a block. Local to this node (card #981).
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/tags`
pub async fn create_columnar_tag_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    Json(req): Json<TagRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let info = tokio::task::spawn_blocking(move || {
        crate::storage::ColumnarCollection::load(collection, &db.name, db.db_arc())?
            .create_tag(&req.tag)
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    Ok((axum::http::StatusCode::CREATED, Json(info)))
}

/// The tags of a columnar collection.
///
/// # Endpoint
/// `GET /_api/database/{db}/columnar/{collection}/tags`
pub async fn list_columnar_tags_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let tags =
        crate::storage::ColumnarCollection::load(collection, &db.name, db.db_arc())?.list_tags()?;
    Ok(Json(serde_json::json!({ "tags": tags })))
}

/// Drop a tag; the blocks only it kept are freed. Admin.
///
/// # Endpoint
/// `DELETE /_api/database/{db}/columnar/{collection}/tags/{tag}`
pub async fn drop_columnar_tag_handler(
    State(state): State<AppState>,
    Path((db_name, collection, tag)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    tokio::task::spawn_blocking(move || {
        crate::storage::ColumnarCollection::load(collection, &db.name, db.db_arc())?.drop_tag(&tag)
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    Ok(Json(serde_json::json!({ "status": "dropped" })))
}

/// Make a tag the collection's rows again. Admin. Refused while this node
/// replicates: a tag is local, and the peers would keep the rows it
/// replaced.
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/tags/{tag}/restore`
pub async fn restore_columnar_tag_handler(
    State(state): State<AppState>,
    Path((db_name, collection, tag)): Path<(String, String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<impl IntoResponse, DbError> {
    outside_a_transaction(&headers, "restoring a tag")?;
    if state
        .replication_log
        .as_deref()
        .is_some_and(|log| log.is_active())
    {
        return Err(DbError::OperationNotSupported(
            "Restoring a tag is refused while this node replicates: tags are local, \
             and the peers would keep the rows it replaces"
                .to_string(),
        ));
    }
    let db = state.storage.get_database(&db_name)?;
    let name = collection.clone();
    let rows = tokio::task::spawn_blocking(move || {
        crate::storage::ColumnarCollection::load(collection, &db.name, db.db_arc())?
            .restore_tag(&tag)
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    crate::storage::query_cache::invalidate_collection(&db_name, &format!("_columnar_{name}"));
    crate::storage::query_cache::invalidate_collection(&db_name, &name);
    Ok(Json(
        serde_json::json!({ "status": "restored", "rows": rows }),
    ))
}

/// `{"branch": "trial", "from": "q3"}`; without `from`, a tag is taken now.
#[derive(Debug, Deserialize)]
pub struct BranchRequest {
    pub branch: String,
    #[serde(default)]
    pub from: Option<String>,
}

/// Create a branch of a columnar collection: a new columnar collection that
/// starts with the rows of one of its tags, read in place, and writes its
/// own from then on. Local to this node: neither it nor its writes are
/// replicated (card #981). Dropped as any columnar collection.
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/branches`
pub async fn create_columnar_branch_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    Json(req): Json<BranchRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let branch = req.branch.clone();
    let rows = tokio::task::spawn_blocking(move || {
        crate::sync::columnar::create_branch(&db, &collection, &req.branch, req.from.as_deref())
            .map(|col| col.count())
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(serde_json::json!({ "status": "created", "name": branch, "rows": rows })),
    ))
}

/// The branches of a columnar collection.
///
/// # Endpoint
/// `GET /_api/database/{db}/columnar/{collection}/branches`
pub async fn list_columnar_branches_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let branches =
        crate::storage::ColumnarCollection::load(collection, &db.name, db.db_arc())?.branches()?;
    Ok(Json(serde_json::json!({ "branches": branches })))
}

/// Export a columnar collection: `?format=parquet` (one zstd-compressed
/// file) or `?format=arrow` (an Arrow IPC stream), the row id first in
/// `_id`. Read by pyarrow, pandas, Polars, DuckDB (card #980).
///
/// # Endpoint
/// `GET /_api/database/{db}/columnar/{collection}/export`
pub async fn export_columnar_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<
        crate::server::handlers::import_export::ExportQuery,
    >,
) -> Result<axum::response::Response, DbError> {
    let format = query.format.unwrap_or_else(|| "parquet".to_string());
    crate::server::handlers::import_export::export_typed(state, db_name, collection, &format, true)
        .await
}

/// The rows of a shard, for the node that coordinates a sharded collection
/// (card #899): MessagePack `[[id, row], …]`, ascending by id. Only on a
/// request that says it is one (`X-Shard-Direct`).
///
/// # Endpoint
/// `GET /_api/database/{db}/columnar/{collection}/rows`
pub async fn shard_rows_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, DbError> {
    if !shard_direct(&headers) {
        return Err(DbError::BadRequest(
            "This endpoint serves the nodes of a cluster (X-Shard-Direct)".to_string(),
        ));
    }
    let db = state.storage.get_database(&db_name)?;
    let bytes = tokio::task::spawn_blocking(move || {
        let rows = crate::storage::ColumnarCollection::load(collection, &db.name, db.db_arc())?
            .rows_with_ids()?;
        rmp_serde::to_vec(&rows).map_err(|e| DbError::InternalError(e.to_string()))
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    axum::response::Response::builder()
        .header("Content-Type", "application/msgpack")
        .body(axum::body::Body::from(bytes))
        .map_err(|e| DbError::InternalError(e.to_string()))
}

/// Rows a coordinating node writes into a shard under the ids it gave them
/// (card #899): MessagePack `[[id, row], …]`, checked against the schema
/// there. Not logged. Only with `X-Shard-Direct`.
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/rows`
pub async fn shard_insert_rows_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, DbError> {
    if !shard_direct(&headers) {
        return Err(DbError::BadRequest(
            "This endpoint serves the nodes of a cluster (X-Shard-Direct)".to_string(),
        ));
    }
    let rows: Vec<crate::sharding::columnar::IdRow> = rmp_serde::from_slice(&body)
        .map_err(|e| DbError::BadRequest(format!("shard rows: {e}")))?;
    let db = state.storage.get_database(&db_name)?;
    let written = tokio::task::spawn_blocking(move || {
        let col = crate::storage::ColumnarCollection::load(collection, &db.name, db.db_arc())?;
        // Checked here too: the header is a client's to send.
        let (ids, rows): (Vec<String>, Vec<serde_json::Value>) = rows
            .into_iter()
            .map(|(id, row)| (id, serde_json::Value::Object(row)))
            .unzip();
        if ids.iter().any(|id| uuid::Uuid::parse_str(id).is_err()) {
            return Err(DbError::BadRequest(
                "a shard row's id is a UUID".to_string(),
            ));
        }
        let rows = col.conform_rows(rows)?;
        col.insert_rows_with_ids(ids.into_iter().zip(rows).collect())
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    Ok(Json(
        serde_json::json!({ "status": "ok", "inserted": written }),
    ))
}
