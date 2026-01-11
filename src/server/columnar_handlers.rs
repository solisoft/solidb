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
    pub columns: Vec<String>,
    #[serde(default)]
    pub filter: Option<FilterRequest>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct FilterRequest {
    pub column: String,
    pub op: String,
    pub value: Value,
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
        .map(|c| ColumnDef {
            name: c.name,
            data_type: parse_column_type(&c.data_type),
            nullable: c.nullable,
            indexed: c.indexed,
            index_type: None, // Default to None when creating from API (will be set when creating index)
        })
        .collect();

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

    // Create column family for columnar collection
    let cf_name = format!("_columnar_{}", req.name);
    db.create_collection(cf_name.clone(), None)?;

    // Create the columnar collection
    let col = ColumnarCollection::new(
        req.name.clone(),
        &db_name,
        db.db_arc(),
        columns.clone(),
        compression,
    )?;

    // Store metadata reference in a system collection if needed
    // For now, the metadata is stored in the column family itself

    Ok(Json(CreateColumnarResponse {
        status: "created".to_string(),
        name: col.name,
        columns: columns.len(),
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

    let cf_name = format!("_columnar_{}", collection);
    db.delete_collection(&cf_name)?;

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

    let col = ColumnarCollection::load(collection.clone(), &db_name, db.db_arc())?;
    let rows_for_log = req.rows.clone();
    let inserted_ids = col.insert_rows(req.rows)?;

    // Log to replication sync log
    if let Some(ref log) = state.replication_log {
        for (id, row) in inserted_ids.iter().zip(rows_for_log.iter()) {
            let row_data = serde_json::to_vec(row).ok();
            log.append_columnar(
                &db_name,
                &collection,
                crate::sync::protocol::Operation::ColumnarInsert,
                id.clone(),
                row_data,
            );
        }
    }

    Ok(Json(InsertColumnarResponse {
        status: "ok".to_string(),
        inserted: inserted_ids.len(),
        ids: inserted_ids,
    }))
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

    let col = ColumnarCollection::load(collection, &db_name, db.db_arc())?;
    let col_refs: Vec<&str> = req.columns.iter().map(|s| s.as_str()).collect();

    let mut results = if let Some(filter_req) = req.filter {
        let filter = parse_filter(&filter_req)?;
        col.scan_filtered(&filter, &col_refs)?
    } else {
        col.read_columns(&col_refs, None)?
    };

    // Apply limit if specified
    if let Some(limit) = req.limit {
        results.truncate(limit);
    }

    let count = results.len();

    Ok(Json(QueryColumnarResponse {
        result: results,
        count,
    }))
}

// ==================== Helper Functions ====================

fn parse_column_type(type_str: &str) -> crate::storage::columnar::ColumnType {
    use crate::storage::columnar::ColumnType;

    match type_str.to_uppercase().as_str() {
        "INT64" | "INTEGER" | "INT" | "BIGINT" => ColumnType::Int64,
        "FLOAT64" | "FLOAT" | "DOUBLE" | "NUMBER" => ColumnType::Float64,
        "STRING" | "TEXT" | "VARCHAR" => ColumnType::String,
        "BOOL" | "BOOLEAN" => ColumnType::Bool,
        "TIMESTAMP" | "DATETIME" | "DATE" => ColumnType::Timestamp,
        "JSON" | "OBJECT" | "ARRAY" => ColumnType::Json,
        _ => ColumnType::String, // Default to string
    }
}

fn parse_filter(req: &FilterRequest) -> Result<ColumnFilter, DbError> {
    match req.op.to_uppercase().as_str() {
        "EQ" | "=" | "==" => Ok(ColumnFilter::Eq(req.column.clone(), req.value.clone())),
        "NE" | "!=" | "<>" => Ok(ColumnFilter::Ne(req.column.clone(), req.value.clone())),
        "GT" | ">" => Ok(ColumnFilter::Gt(req.column.clone(), req.value.clone())),
        "GTE" | ">=" => Ok(ColumnFilter::Gte(req.column.clone(), req.value.clone())),
        "LT" | "<" => Ok(ColumnFilter::Lt(req.column.clone(), req.value.clone())),
        "LTE" | "<=" => Ok(ColumnFilter::Lte(req.column.clone(), req.value.clone())),
        "IN" => {
            if let Value::Array(arr) = &req.value {
                Ok(ColumnFilter::In(req.column.clone(), arr.clone()))
            } else {
                Err(DbError::BadRequest(
                    "IN operator requires an array value".to_string(),
                ))
            }
        }
        other => Err(DbError::BadRequest(format!(
            "Unknown filter operator: {}. Supported: EQ, NE, GT, GTE, LT, LTE, IN",
            other
        ))),
    }
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

    let col = ColumnarCollection::load(collection.clone(), &db_name, db.db_arc())?;

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

    col.create_index(&req.column, index_type.clone())?;

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

    let col = ColumnarCollection::load(collection, &db_name, db.db_arc())?;
    col.drop_index(&column)?;

    Ok(Json(serde_json::json!({
        "status": "deleted",
        "column": column
    })))
}
