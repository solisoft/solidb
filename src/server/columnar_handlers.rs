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

    // Created here and logged for the peers.
    let count = columns.len();
    let col = crate::sync::columnar::create(
        &db,
        state.replication_log.as_deref(),
        &req.name,
        columns,
        compression,
    )?;

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
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;

    crate::sync::columnar::drop(&db, state.replication_log.as_deref(), &collection)?;

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
    Json(req): Json<InsertColumnarRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;

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
) -> Result<impl IntoResponse, DbError> {
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
    Json(req): Json<DeleteColumnarRowsRequest>,
) -> Result<impl IntoResponse, DbError> {
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
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;
    crate::sync::columnar::truncate(&db, state.replication_log.as_deref(), &collection)?;
    Ok(Json(
        serde_json::json!({"status": "truncated", "name": collection}),
    ))
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
    Json(req): Json<AggregateRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;

    let col = ColumnarCollection::load(collection, &db_name, db.db_arc())?;

    let op = AggregateOp::from_str(&req.operation).ok_or_else(|| {
        DbError::BadRequest(format!(
            "Unknown aggregation operation: {}. Supported: SUM, AVG, COUNT, MIN, MAX, COUNT_DISTINCT",
            req.operation
        ))
    })?;

    if let Some(group_cols) = req.group_by {
        // Group by aggregation
        use crate::storage::columnar::GroupByColumn;
        let group_defs: Vec<GroupByColumn> = group_cols
            .iter()
            .map(|s| GroupByColumn::Simple(s.clone()))
            .collect();

        let groups = col.group_by(&group_defs, &req.column, op)?;

        Ok(Json(AggregateResponse {
            result: Value::Null,
            groups: Some(groups),
        }))
    } else {
        // Simple aggregation
        let result = col.aggregate(&req.column, op)?;

        Ok(Json(AggregateResponse {
            result,
            groups: None,
        }))
    }
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
    Json(req): Json<QueryColumnarRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;

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

/// Create an index on a columnar collection column
///
/// # Endpoint
/// `POST /_api/database/{db}/columnar/{collection}/index`
///
/// # Request Body
/// - `column` - Column name to index (required)
/// - `index_type` - Index type: "sorted" (default) or "hash"
///
/// # Response
/// ```json
/// {"status": "created", "column": "host", "index_type": "sorted"}
/// ```
pub async fn create_columnar_index_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
    Json(req): Json<CreateIndexRequest>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;

    let index_type = match req.index_type.as_deref() {
        Some("hash") => crate::storage::columnar::ColumnarIndexType::Hash,
        Some("sorted") | None => crate::storage::columnar::ColumnarIndexType::Sorted,
        Some("bitmap") => crate::storage::columnar::ColumnarIndexType::Bitmap,
        Some("minmax") => crate::storage::columnar::ColumnarIndexType::MinMax,
        Some("bloom") => crate::storage::columnar::ColumnarIndexType::Bloom,
        Some(other) => {
            return Err(DbError::BadRequest(format!(
                "Unknown index type: {}. Supported: sorted, hash, bitmap, minmax, bloom",
                other
            )))
        }
    };

    crate::sync::columnar::create_index(
        &db,
        state.replication_log.as_deref(),
        &collection,
        &req.column,
        index_type.clone(),
    )?;

    Ok(Json(CreateIndexResponse {
        status: "created".to_string(),
        column: req.column,
        index_type: format!("{:?}", index_type).to_lowercase(),
    }))
}

/// List indexes on a columnar collection
///
/// # Endpoint
/// `GET /_api/database/{db}/columnar/{collection}/indexes`
///
/// # Response
/// ```json
/// {"indexes": [{"column": "host", "index_type": "sorted", "created_at": 1234567890}], "count": 1}
/// ```
pub async fn list_columnar_indexes_handler(
    State(state): State<AppState>,
    Path((db_name, collection)): Path<(String, String)>,
) -> Result<impl IntoResponse, DbError> {
    let db = state.storage.get_database(&db_name)?;

    let col = ColumnarCollection::load(collection, &db_name, db.db_arc())?;
    let indexes = col.list_indexes()?;

    let indexes_json: Vec<Value> = indexes
        .into_iter()
        .map(|idx| {
            serde_json::json!({
                "column": idx.column,
                "index_type": format!("{:?}", idx.index_type).to_lowercase(),
                "created_at": idx.created_at
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "indexes": indexes_json,
        "count": indexes_json.len()
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
