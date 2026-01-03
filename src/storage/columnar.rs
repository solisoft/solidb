//! Columnar Storage for Analytics/Reporting Workloads
//!
//! This module provides column-oriented storage optimized for:
//! - Read-heavy workloads (analytics, reporting)
//! - Aggregations (SUM, AVG, COUNT, MIN, MAX)
//! - Column pruning (only read needed columns)
//! - LZ4 compression for efficient storage

use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use rocksdb::DB;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::error::{DbError, DbResult};

/// Key prefixes for columnar storage
const COL_DATA_PREFIX: &str = "col:";      // col:{column}:{row_id} -> compressed_value
const COL_META_PREFIX: &str = "col_meta:"; // col_meta:{collection} -> ColumnarCollectionMeta
const COL_ROW_PREFIX: &str = "col_row:";   // col_row:{row_id} -> full row for reconstruction
const COL_IDX_PREFIX: &str = "col_idx:";   // col_idx:{column}:{encoded_value} -> [row_ids]
const COL_IDX_META_PREFIX: &str = "col_idx_meta:"; // col_idx_meta:{column} -> ColumnarIndexMeta
const COL_IDX_BITMAP_PREFIX: &str = "col_idx_bmp:"; // col_idx_bmp:{column}:{encoded_value} -> [compressed_bitset]
const COL_IDX_MINMAX_PREFIX: &str = "col_idx_mm:";  // col_idx_mm:{column}:{chunk_id} -> {min, max}

const MINMAX_CHUNK_SIZE: u64 = 1000;

/// Column data types supported in columnar storage
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ColumnType {
    Int64,
    Float64,
    String,
    Bool,
    Timestamp,
    Json, // For complex nested data
}

impl ColumnType {
    /// Infer column type from a JSON value
    pub fn infer_from_value(value: &Value) -> Self {
        match value {
            Value::Bool(_) => ColumnType::Bool,
            Value::Number(n) => {
                if n.is_i64() {
                    ColumnType::Int64
                } else {
                    ColumnType::Float64
                }
            }
            Value::String(s) => {
                // Check if it looks like a timestamp
                if chrono::DateTime::parse_from_rfc3339(s).is_ok() {
                    ColumnType::Timestamp
                } else {
                    ColumnType::String
                }
            }
            Value::Array(_) | Value::Object(_) => ColumnType::Json,
            Value::Null => ColumnType::String, // Default for null
        }
    }
}

/// Column definition for columnar collections
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub data_type: ColumnType,
    pub nullable: bool,
    #[serde(default)]
    pub indexed: bool,
    #[serde(default)]
    pub index_type: Option<ColumnarIndexType>,
}

/// Compression type for column data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CompressionType {
    None,
    #[default]
    Lz4,
}

/// Metadata for a columnar collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnarCollectionMeta {
    pub name: String,
    pub columns: Vec<ColumnDef>,
    pub row_count: u64,
    pub compression: CompressionType,
    pub created_at: i64,
    #[serde(default)]
    pub last_updated_at: i64,
}

/// Aggregation operations supported on columnar data
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AggregateOp {
    Sum,
    Avg,
    Count,
    Min,
    Max,
    CountDistinct,
}

impl AggregateOp {
    /// Parse aggregate operation from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "SUM" => Some(AggregateOp::Sum),
            "AVG" | "AVERAGE" => Some(AggregateOp::Avg),
            "COUNT" => Some(AggregateOp::Count),
            "MIN" => Some(AggregateOp::Min),
            "MAX" => Some(AggregateOp::Max),
            "COUNT_DISTINCT" => Some(AggregateOp::CountDistinct),
            _ => None,
        }
    }
}

/// Filter for columnar scans
#[derive(Debug, Clone)]
pub enum ColumnFilter {
    Eq(String, Value),
    Ne(String, Value),
    Gt(String, Value),
    Gte(String, Value),
    Lt(String, Value),
    Lte(String, Value),
    In(String, Vec<Value>),
    And(Vec<ColumnFilter>),
    Or(Vec<ColumnFilter>),
}

/// Statistics for a columnar collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnarStats {
    pub name: String,
    pub row_count: u64,
    pub column_count: usize,
    pub compressed_size_bytes: u64,
    pub uncompressed_size_bytes: u64,
    pub compression_ratio: f64,
}

/// Index type for columnar collections
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ColumnarIndexType {
    #[default]
    Sorted,  // Supports equality + range queries (binary-comparable keys)
    Hash,    // Equality-only (faster for high-cardinality)
    Bitmap,  // Low cardinality, fast boolean ops, compressed
    MinMax,  // Range pruning for high cardinality/time-series
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinMaxChunk {
    pub min: Value,
    pub max: Value,
    pub count: u64,
}

/// Metadata for a columnar index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnarIndexMeta {
    pub column: String,
    pub index_type: ColumnarIndexType,
    pub created_at: i64,
}

/// Column grouping definition
#[derive(Debug, Clone)]
pub enum GroupByColumn {
    /// Simple column grouping
    Simple(String),
    /// Time bucket grouping: (column, interval_str)
    /// Interval examples: "1h", "5m", "1d"
    TimeBucket(String, String),
}

impl GroupByColumn {
    pub fn name(&self) -> &str {
        match self {
            GroupByColumn::Simple(name) => name,
            GroupByColumn::TimeBucket(name, _) => name,
        }
    }
}

/// Columnar collection handle
pub struct ColumnarCollection {
    pub name: String,
    db: Arc<RwLock<DB>>,
    cf_name: String, // Column family name for this columnar collection
    meta: Arc<RwLock<ColumnarCollectionMeta>>,
}

impl Clone for ColumnarCollection {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            db: self.db.clone(),
            cf_name: self.cf_name.clone(),
            meta: self.meta.clone(),
        }
    }
}

impl std::fmt::Debug for ColumnarCollection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ColumnarCollection")
            .field("name", &self.name)
            .field("cf_name", &self.cf_name)
            .finish()
    }
}

impl ColumnarCollection {
    /// Create a new columnar collection
    ///
    /// # Arguments
    /// * `name` - Collection name (without prefix)
    /// * `db_name` - Database name for CF naming
    /// * `db` - RocksDB Arc
    /// * `columns` - Column definitions
    /// * `compression` - Compression type (default LZ4)
    pub fn new(
        name: String,
        db_name: &str,
        db: Arc<RwLock<DB>>,
        columns: Vec<ColumnDef>,
        compression: CompressionType,
    ) -> DbResult<Self> {
        // CF name follows the database collection naming pattern
        let cf_name = format!("{}:_columnar_{}", db_name, name);
        let now = chrono::Utc::now().timestamp();

        let meta = ColumnarCollectionMeta {
            name: name.clone(),
            columns,
            row_count: 0,
            compression,
            created_at: now,
            last_updated_at: now,
        };

        // Store metadata
        {
            let db_guard = db.write().map_err(|e| DbError::InternalError(e.to_string()))?;
            let cf = db_guard.cf_handle(&cf_name).ok_or_else(|| {
                DbError::CollectionNotFound(format!("Columnar CF '{}' not found", cf_name))
            })?;

            let meta_key = format!("{}{}", COL_META_PREFIX, name);
            let meta_bytes = serde_json::to_vec(&meta)?;
            db_guard
                .put_cf(cf, meta_key.as_bytes(), &meta_bytes)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        Ok(Self {
            name,
            db,
            cf_name,
            meta: Arc::new(RwLock::new(meta)),
        })
    }

    /// Load an existing columnar collection
    pub fn load(name: String, db_name: &str, db: Arc<RwLock<DB>>) -> DbResult<Self> {
        let cf_name = format!("{}:_columnar_{}", db_name, name);

        let meta = {
            let db_guard = db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
            let cf = db_guard.cf_handle(&cf_name).ok_or_else(|| {
                DbError::CollectionNotFound(format!("Columnar collection '{}' not found", name))
            })?;

            let meta_key = format!("{}{}", COL_META_PREFIX, name);
            let meta_bytes = db_guard
                .get_cf(cf, meta_key.as_bytes())
                .map_err(|e| DbError::InternalError(e.to_string()))?
                .ok_or_else(|| {
                    DbError::CollectionNotFound(format!(
                        "Columnar collection metadata '{}' not found",
                        name
                    ))
                })?;

            serde_json::from_slice::<ColumnarCollectionMeta>(&meta_bytes)?
        };

        Ok(Self {
            name,
            db,
            cf_name,
            meta: Arc::new(RwLock::new(meta)),
        })
    }

    /// Insert rows into columnar storage
    /// Returns a vector of UUIDs for the inserted rows (for replication)
    pub fn insert_rows(&self, rows: Vec<Value>) -> DbResult<Vec<String>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }

        let mut meta = self.meta.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let mut inserted_ids = Vec::new();

        for row in rows.iter() {
            // Generate UUID v7 (time-ordered) for each row
            let row_uuid = uuid7::uuid7().to_string();

            if let Value::Object(obj) = row {
                // Store each column value separately
                for col_def in &meta.columns {
                    let value = obj.get(&col_def.name).unwrap_or(&Value::Null);
                    let col_key = format!("{}{}:{}", COL_DATA_PREFIX, col_def.name, row_uuid);

                    let value_bytes = serde_json::to_vec(value)?;
                    let stored_bytes = self.compress_data(&value_bytes, &meta.compression);

                    db_guard
                        .put_cf(cf, col_key.as_bytes(), &stored_bytes)
                        .map_err(|e| DbError::InternalError(e.to_string()))?;
                }

                // Update indexes for indexed columns (using UUID string)
                self.update_indexes_for_row_uuid(&db_guard, cf, &meta.columns, obj, &row_uuid)?;

                // Also store full row for reconstruction
                let row_key = format!("{}{}", COL_ROW_PREFIX, row_uuid);
                let row_bytes = serde_json::to_vec(row)?;
                let stored_row = self.compress_data(&row_bytes, &meta.compression);
                db_guard
                    .put_cf(cf, row_key.as_bytes(), &stored_row)
                    .map_err(|e| DbError::InternalError(e.to_string()))?;

                inserted_ids.push(row_uuid);
            }
        }

        // Update metadata
        meta.row_count += inserted_ids.len() as u64;
        meta.last_updated_at = chrono::Utc::now().timestamp();

        let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
        let meta_bytes = serde_json::to_vec(&*meta)?;
        db_guard
            .put_cf(cf, meta_key.as_bytes(), &meta_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(inserted_ids)
    }

    /// Insert a row with a specific UUID (for replication)
    /// This is idempotent - if the row already exists, it's skipped
    pub fn insert_row_with_id(&self, row_uuid: &str, row: Value) -> DbResult<bool> {
        let mut meta = self.meta.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        // Check if row already exists (idempotency for replication)
        let row_key = format!("{}{}", COL_ROW_PREFIX, row_uuid);
        if db_guard.get_cf(cf, row_key.as_bytes())
            .map_err(|e| DbError::InternalError(e.to_string()))?
            .is_some()
        {
            return Ok(false); // Already exists, skip
        }

        if let Value::Object(obj) = &row {
            // Store each column value separately
            for col_def in &meta.columns {
                let value = obj.get(&col_def.name).unwrap_or(&Value::Null);
                let col_key = format!("{}{}:{}", COL_DATA_PREFIX, col_def.name, row_uuid);

                let value_bytes = serde_json::to_vec(value)?;
                let stored_bytes = self.compress_data(&value_bytes, &meta.compression);

                db_guard
                    .put_cf(cf, col_key.as_bytes(), &stored_bytes)
                    .map_err(|e| DbError::InternalError(e.to_string()))?;
            }

            // Update indexes for indexed columns
            self.update_indexes_for_row_uuid(&db_guard, cf, &meta.columns, obj, row_uuid)?;

            // Store full row for reconstruction
            let row_bytes = serde_json::to_vec(&row)?;
            let stored_row = self.compress_data(&row_bytes, &meta.compression);
            db_guard
                .put_cf(cf, row_key.as_bytes(), &stored_row)
                .map_err(|e| DbError::InternalError(e.to_string()))?;

            // Update metadata
            meta.row_count += 1;
            meta.last_updated_at = chrono::Utc::now().timestamp();

            let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
            let meta_bytes = serde_json::to_vec(&*meta)?;
            db_guard
                .put_cf(cf, meta_key.as_bytes(), &meta_bytes)
                .map_err(|e| DbError::InternalError(e.to_string()))?;

            Ok(true)
        } else {
            Err(DbError::BadRequest("Row must be a JSON object".to_string()))
        }
    }

    /// Delete a row by UUID
    pub fn delete_row(&self, row_uuid: &str) -> DbResult<bool> {
        let mut meta = self.meta.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        // Check if row exists
        let row_key = format!("{}{}", COL_ROW_PREFIX, row_uuid);
        if db_guard.get_cf(cf, row_key.as_bytes())
            .map_err(|e| DbError::InternalError(e.to_string()))?
            .is_none()
        {
            return Ok(false); // Doesn't exist
        }

        // Delete column values
        for col_def in &meta.columns {
            let col_key = format!("{}{}:{}", COL_DATA_PREFIX, col_def.name, row_uuid);
            db_guard.delete_cf(cf, col_key.as_bytes())
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        // Delete full row
        db_guard.delete_cf(cf, row_key.as_bytes())
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        // Update metadata
        if meta.row_count > 0 {
            meta.row_count -= 1;
        }
        meta.last_updated_at = chrono::Utc::now().timestamp();

        let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
        let meta_bytes = serde_json::to_vec(&*meta)?;
        db_guard
            .put_cf(cf, meta_key.as_bytes(), &meta_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(true)
    }

    /// List all row UUIDs in the collection
    pub fn list_row_uuids(&self) -> DbResult<Vec<String>> {
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let prefix = COL_ROW_PREFIX.as_bytes();
        let iter = db_guard.prefix_iterator_cf(cf, prefix);

        let mut uuids = Vec::new();
        for item in iter {
            if let Ok((key, _)) = item {
                if !key.starts_with(prefix) {
                    break;
                }
                let key_str = String::from_utf8_lossy(&key);
                if let Some(uuid) = key_str.strip_prefix(COL_ROW_PREFIX) {
                    uuids.push(uuid.to_string());
                }
            }
        }

        Ok(uuids)
    }

    /// Read a single column for all rows or specific row IDs
    pub fn read_column(&self, column: &str, row_uuids: Option<&[String]>) -> DbResult<Vec<Value>> {
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        // Validate column exists
        if !meta.columns.iter().any(|c| c.name == column) {
            return Err(DbError::BadRequest(format!("Column '{}' not found", column)));
        }

        // Get UUIDs to read
        let uuids: Vec<String> = match row_uuids {
            Some(ids) => ids.to_vec(),
            None => {
                // List all row UUIDs from col_row: prefix
                drop(meta);
                drop(db_guard);
                self.list_row_uuids()?
            }
        };

        // Re-acquire locks if we had to get UUIDs
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let mut values = Vec::with_capacity(uuids.len());

        for row_uuid in &uuids {
            let col_key = format!("{}{}:{}", COL_DATA_PREFIX, column, row_uuid);

            match db_guard.get_cf(cf, col_key.as_bytes()) {
                Ok(Some(bytes)) => {
                    let decompressed = self.decompress_data(&bytes, &meta.compression)?;
                    let value: Value = serde_json::from_slice(&decompressed)?;
                    values.push(value);
                }
                Ok(None) => values.push(Value::Null),
                Err(e) => return Err(DbError::InternalError(e.to_string())),
            }
        }

        Ok(values)
    }

    /// Read multiple columns (projection) for all rows or specific row UUIDs
    pub fn read_columns(&self, columns: &[&str], row_uuids: Option<&[String]>) -> DbResult<Vec<Value>> {
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        // Validate columns exist
        for col in columns {
            if !meta.columns.iter().any(|c| &c.name == col) {
                return Err(DbError::BadRequest(format!("Column '{}' not found", col)));
            }
        }

        // Get UUIDs to read
        let uuids: Vec<String> = match row_uuids {
            Some(ids) => ids.to_vec(),
            None => {
                // List all row UUIDs from col_row: prefix
                drop(meta);
                drop(db_guard);
                self.list_row_uuids()?
            }
        };

        // Re-acquire locks if we had to get UUIDs
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let mut results = Vec::with_capacity(uuids.len());

        for row_uuid in &uuids {
            let mut row_obj = serde_json::Map::new();

            for col in columns {
                let col_key = format!("{}{}:{}", COL_DATA_PREFIX, col, row_uuid);

                let value = match db_guard.get_cf(cf, col_key.as_bytes()) {
                    Ok(Some(bytes)) => {
                        let decompressed = self.decompress_data(&bytes, &meta.compression)?;
                        serde_json::from_slice(&decompressed)?
                    }
                    Ok(None) => Value::Null,
                    Err(e) => return Err(DbError::InternalError(e.to_string())),
                };

                row_obj.insert(col.to_string(), value);
            }

            results.push(Value::Object(row_obj));
        }

        Ok(results)
    }

    /// Get total row count
    pub fn count(&self) -> usize {
        self.meta.read().map(|m| m.row_count as usize).unwrap_or(0)
    }

    /// Read a single column value for a specific row UUID
    fn read_column_value(&self, column: &str, row_uuid: &str) -> DbResult<Value> {
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let col_key = format!("{}{}:{}", COL_DATA_PREFIX, column, row_uuid);

        match db_guard.get_cf(cf, col_key.as_bytes()) {
            Ok(Some(bytes)) => {
                let decompressed = self.decompress_data(&bytes, &meta.compression)?;
                let value: Value = serde_json::from_slice(&decompressed)?;
                Ok(value)
            }
            Ok(None) => Ok(Value::Null),
            Err(e) => Err(DbError::InternalError(e.to_string())),
        }
    }

    /// Convert position indices to UUIDs (positions are indices into list_row_uuids result)
    fn positions_to_uuids(&self, positions: &[u64]) -> DbResult<Vec<String>> {
        let all_uuids = self.list_row_uuids()?;
        let mut result = Vec::with_capacity(positions.len());
        for &pos in positions {
            if let Some(uuid) = all_uuids.get(pos as usize) {
                result.push(uuid.clone());
            }
        }
        Ok(result)
    }

    /// Read column values by position indices (internal use for filtering)
    fn read_column_by_positions(&self, column: &str, positions: &[u64]) -> DbResult<Vec<Value>> {
        let uuids = self.positions_to_uuids(positions)?;
        self.read_column(column, Some(&uuids))
    }

    /// Aggregate a column with the specified operation
    /// Aggregate a column with the specified operation
    pub fn aggregate(&self, column: &str, op: AggregateOp) -> DbResult<Value> {
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        // Optimization: Use prefix iterator directly to avoid loading all values into memory
        // This relies on the fact that aggregation is commutative/associative and order doesn't matter
        let prefix = format!("{}{}:", COL_DATA_PREFIX, column);
        let iter = db_guard.prefix_iterator_cf(cf, prefix.as_bytes());

        match op {
            AggregateOp::Count => {
                let mut count = 0;
                for item in iter {
                    if let Ok((key, _)) = item {
                        if !key.starts_with(prefix.as_bytes()) {
                            break;
                        }
                        count += 1;
                    } else {
                        break;
                    }
                }
                Ok(Value::Number(count.into()))
            }

            AggregateOp::Sum => {
                let mut sum = 0.0;
                for item in iter {
                    if let Ok((key, val_bytes)) = item {
                        if !key.starts_with(prefix.as_bytes()) {
                            break;
                        }
                        if let Ok(decompressed) = self.decompress_data(&val_bytes, &meta.compression) {
                            if let Ok(value) = serde_json::from_slice::<Value>(&decompressed) {
                                if let Some(n) = value.as_f64() {
                                    sum += n;
                                }
                            }
                        }
                    }
                }
                Ok(json_number(sum))
            }

            AggregateOp::Avg => {
                let mut sum = 0.0;
                let mut count = 0;
                for item in iter {
                    if let Ok((key, val_bytes)) = item {
                        if !key.starts_with(prefix.as_bytes()) {
                            break;
                        }
                        if let Ok(decompressed) = self.decompress_data(&val_bytes, &meta.compression) {
                            if let Ok(value) = serde_json::from_slice::<Value>(&decompressed) {
                                if let Some(n) = value.as_f64() {
                                    sum += n;
                                    count += 1;
                                }
                            }
                        }
                    }
                }
                if count == 0 {
                    Ok(Value::Null)
                } else {
                    Ok(json_number(sum / count as f64))
                }
            }

            AggregateOp::Min => {
                let mut min_val: Option<f64> = None;
                for item in iter {
                    if let Ok((key, val_bytes)) = item {
                        if !key.starts_with(prefix.as_bytes()) {
                            break;
                        }
                        if let Ok(decompressed) = self.decompress_data(&val_bytes, &meta.compression) {
                            if let Ok(value) = serde_json::from_slice::<Value>(&decompressed) {
                                if let Some(n) = value.as_f64() {
                                    match min_val {
                                        Some(cur_min) => {
                                            if n < cur_min {
                                                min_val = Some(n);
                                            }
                                        }
                                        None => min_val = Some(n),
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(min_val.map(json_number).unwrap_or(Value::Null))
            }

            AggregateOp::Max => {
                let mut max_val: Option<f64> = None;
                for item in iter {
                    if let Ok((key, val_bytes)) = item {
                        if !key.starts_with(prefix.as_bytes()) {
                            break;
                        }
                        if let Ok(decompressed) = self.decompress_data(&val_bytes, &meta.compression) {
                            if let Ok(value) = serde_json::from_slice::<Value>(&decompressed) {
                                if let Some(n) = value.as_f64() {
                                    match max_val {
                                        Some(cur_max) => {
                                            if n > cur_max {
                                                max_val = Some(n);
                                            }
                                        }
                                        None => max_val = Some(n),
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(max_val.map(json_number).unwrap_or(Value::Null))
            }

            AggregateOp::CountDistinct => {
                let mut distinct = std::collections::HashSet::new();
                for item in iter {
                    if let Ok((key, val_bytes)) = item {
                        if !key.starts_with(prefix.as_bytes()) {
                            break;
                        }
                        if let Ok(decompressed) = self.decompress_data(&val_bytes, &meta.compression) {
                            if let Ok(value) = serde_json::from_slice::<Value>(&decompressed) {
                                distinct.insert(value.to_string());
                            }
                        }
                    }
                }
                Ok(Value::Number(distinct.len().into()))
            }
        }
    }

    /// Group by with aggregation
    /// Group by with aggregation
    pub fn group_by(
        &self,
        group_columns: &[GroupByColumn],
        agg_column: &str,
        op: AggregateOp,
    ) -> DbResult<Vec<Value>> {
        // Get all row UUIDs
        let all_uuids = self.list_row_uuids()?;

        // Read group columns and aggregation column
        let mut group_data: HashMap<String, Vec<String>> = HashMap::new();

        for row_uuid in &all_uuids {
            // Build group key
            let mut group_key_parts = Vec::new();
            for col_def in group_columns {
                let v = self.read_column_value(col_def.name(), row_uuid)?;
                match col_def {
                    GroupByColumn::Simple(_) => {
                        group_key_parts.push(v.to_string());
                    }
                    GroupByColumn::TimeBucket(_, interval) => {
                        // Try to parse timestamp and bucket it
                        let s = v.as_str().unwrap_or("");
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                            // Simple bucketing logic (naive implementation for POC)
                            // Parse interval
                            let (num, unit) = parsers::parse_interval(interval)
                                .unwrap_or((1, "h")); // Default to 1h if invalid

                            let timestamp = dt.timestamp();
                            let bucket_size = match unit {
                                "s" => num,
                                "m" => num * 60,
                                "h" => num * 3600,
                                "d" => num * 86400,
                                _ => num,
                            };

                            let bucketed = (timestamp / bucket_size) * bucket_size;
                            // Convert back to ISO string
                            if let Some(dt_utc) = chrono::DateTime::from_timestamp(bucketed, 0) {
                                 group_key_parts.push(dt_utc.to_rfc3339());
                            } else {
                                 group_key_parts.push(v.to_string());
                            }
                        } else {
                            group_key_parts.push(v.to_string());
                        }
                    }
                }
            }
            let group_key = group_key_parts.join("|");

            group_data.entry(group_key).or_default().push(row_uuid.clone());
        }

        // Compute aggregates for each group
        let mut results = Vec::new();

        for (group_key, row_uuids) in group_data {
            let agg_values = self.read_column(agg_column, Some(&row_uuids))?;
            let agg_result = self.compute_aggregate(&agg_values, op);

            // Build result object
            let mut result_obj = serde_json::Map::new();

            // Parse group key back into parts
            let key_parts: Vec<&str> = group_key.split('|').collect();
            for (i, col_def) in group_columns.iter().enumerate() {
                if let Some(part) = key_parts.get(i) {
                    // Try to parse as number first if it's not a TimeBucket (TimeBucket produced string)
                    let use_number = match col_def {
                        GroupByColumn::TimeBucket(_, _) => false,
                        _ => part.parse::<f64>().is_ok(),
                    };

                    if use_number {
                        if let Ok(n) = part.parse::<f64>() {
                            result_obj.insert(col_def.name().to_string(), Value::Number(serde_json::Number::from_f64(n).unwrap_or(serde_json::Number::from(0))));
                        } else {
                            result_obj.insert(col_def.name().to_string(), Value::String(part.to_string()));
                        }
                    } else {
                        result_obj.insert(col_def.name().to_string(), Value::String(part.to_string()));
                    }
                }
            }

            result_obj.insert("_agg".to_string(), agg_result);
            results.push(Value::Object(result_obj));
        }

        Ok(results)
    }

    /// Scan with filter and column projection
    pub fn scan_filtered(
        &self,
        filter: &ColumnFilter,
        columns: &[&str],
    ) -> DbResult<Vec<Value>> {
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;

        // Find matching row indices (positions)
        let matching_positions = self.apply_filter(filter, 0..meta.row_count)?;

        // Convert positions to UUIDs
        let matching_uuids = self.positions_to_uuids(&matching_positions)?;

        // Return projected columns for matching rows
        self.read_columns(columns, Some(&matching_uuids))
    }

    /// Get collection statistics
    pub fn stats(&self) -> DbResult<ColumnarStats> {
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let mut compressed_size = 0u64;
        let mut uncompressed_size = 0u64;

        // Iterate over all column data to calculate sizes
        let prefix = COL_DATA_PREFIX.as_bytes();
        let iter = db_guard.prefix_iterator_cf(cf, prefix);

        for item in iter {
            if let Ok((_, value)) = item {
                compressed_size += value.len() as u64;
                if let Ok(decompressed) = self.decompress_data(&value, &meta.compression) {
                    uncompressed_size += decompressed.len() as u64;
                }
            }
        }

        let compression_ratio = if compressed_size > 0 {
            uncompressed_size as f64 / compressed_size as f64
        } else {
            1.0
        };

        Ok(ColumnarStats {
            name: self.name.clone(),
            row_count: meta.row_count,
            column_count: meta.columns.len(),
            compressed_size_bytes: compressed_size,
            uncompressed_size_bytes: uncompressed_size,
            compression_ratio,
        })
    }

    /// Get collection metadata
    pub fn metadata(&self) -> DbResult<ColumnarCollectionMeta> {
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        Ok(meta.clone())
    }

    /// Truncate all data from the collection (preserves schema)
    pub fn truncate(&self) -> DbResult<()> {
        let mut meta = self.meta.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        // Delete all row data (col: prefix)
        let prefix = COL_DATA_PREFIX.as_bytes();
        let iter = db_guard.prefix_iterator_cf(cf, prefix);
        for item in iter {
            if let Ok((key, _)) = item {
                db_guard.delete_cf(cf, &key)
                    .map_err(|e| DbError::InternalError(format!("Failed to delete: {}", e)))?;
            }
        }

        // Delete all row entries (col_row: prefix)
        let row_prefix = COL_ROW_PREFIX.as_bytes();
        let iter = db_guard.prefix_iterator_cf(cf, row_prefix);
        for item in iter {
            if let Ok((key, _)) = item {
                db_guard.delete_cf(cf, &key)
                    .map_err(|e| DbError::InternalError(format!("Failed to delete: {}", e)))?;
            }
        }

        // Delete all index data (idx: prefix)
        let idx_prefix = b"idx:";
        let iter = db_guard.prefix_iterator_cf(cf, idx_prefix);
        for item in iter {
            if let Ok((key, _)) = item {
                db_guard.delete_cf(cf, &key)
                    .map_err(|e| DbError::InternalError(format!("Failed to delete: {}", e)))?;
            }
        }

        // Reset row count
        meta.row_count = 0;

        // Save updated metadata
        let meta_bytes = serde_json::to_vec(&*meta)
            .map_err(|e| DbError::InternalError(format!("Failed to serialize meta: {}", e)))?;
        db_guard.put_cf(cf, b"meta", &meta_bytes)
            .map_err(|e| DbError::InternalError(format!("Failed to save meta: {}", e)))?;

        Ok(())
    }

    /// Drop the entire columnar collection (removes everything including schema)
    pub fn drop(&self) -> DbResult<()> {
        let mut db_guard = self.db.write().map_err(|e| DbError::InternalError(e.to_string()))?;

        // Drop the column family - this removes all data
        db_guard.drop_cf(&self.cf_name)
            .map_err(|e| DbError::InternalError(format!("Failed to drop CF: {}", e)))?;

        Ok(())
    }

    // ==================== Index Methods ====================

    /// Create an index on a column
    pub fn create_index(&self, column: &str, index_type: ColumnarIndexType) -> DbResult<()> {
        let mut meta = self.meta.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        // Validate column exists and check if already indexed
        let col_idx = meta.columns.iter().position(|c| c.name == column).ok_or_else(|| {
            DbError::BadRequest(format!("Column '{}' not found", column))
        })?;

        if meta.columns[col_idx].indexed {
            return Err(DbError::BadRequest(format!("Column '{}' already has an index", column)));
        }

        // Copy values we need before building index
        let row_count = meta.row_count;
        let compression = meta.compression.clone();

        // Build index from existing data
        for row_id in 0..row_count {
            let col_key = format!("{}{}:{}", COL_DATA_PREFIX, column, row_id);
            if let Ok(Some(bytes)) = db_guard.get_cf(cf, col_key.as_bytes()) {
                if let Ok(decompressed) = self.decompress_data(&bytes, &compression) {
                    if let Ok(value) = serde_json::from_slice::<Value>(&decompressed) {

                        match index_type {
                            ColumnarIndexType::Bitmap => {
                                let idx_key = self.encode_bitmap_key(column, &value);
                                self.append_row_to_bitmap_index(&db_guard, cf, &idx_key, row_id, &compression)?;
                            }
                            ColumnarIndexType::MinMax => {
                                self.update_minmax_index(&db_guard, cf, column, &value, row_id)?;
                            }
                            _ => {
                                let idx_key = self.encode_index_key(column, &value);
                                self.append_row_to_index(&db_guard, cf, &idx_key, row_id)?;
                            }
                        }
                    }
                }
            }
        }

        // Store index metadata
        let idx_meta = ColumnarIndexMeta {
            column: column.to_string(),
            index_type: index_type.clone(),
            created_at: chrono::Utc::now().timestamp(),
        };
        let idx_meta_key = format!("{}{}", COL_IDX_META_PREFIX, column);
        let idx_meta_bytes = serde_json::to_vec(&idx_meta)?;
        db_guard
            .put_cf(cf, idx_meta_key.as_bytes(), &idx_meta_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        // Mark column as indexed
        meta.columns[col_idx].indexed = true;
        meta.columns[col_idx].index_type = Some(index_type.clone());

        // Update collection metadata
        let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
        let meta_bytes = serde_json::to_vec(&*meta)?;
        db_guard
            .put_cf(cf, meta_key.as_bytes(), &meta_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// Drop an index from a column
    pub fn drop_index(&self, column: &str) -> DbResult<()> {
        let mut meta = self.meta.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        // Validate column exists and has index
        let col_idx = meta.columns.iter().position(|c| c.name == column).ok_or_else(|| {
            DbError::BadRequest(format!("Column '{}' not found", column))
        })?;

        if !meta.columns[col_idx].indexed {
            return Err(DbError::BadRequest(format!("Column '{}' has no index", column)));
        }

        // Delete all index entries for this column
        let prefix = format!("{}{}:", COL_IDX_PREFIX, column);
        let iter = db_guard.prefix_iterator_cf(cf, prefix.as_bytes());
        for item in iter {
            if let Ok((key, _)) = item {
                if !key.starts_with(prefix.as_bytes()) {
                    break;
                }
                db_guard.delete_cf(cf, &key).map_err(|e| DbError::InternalError(e.to_string()))?;
            }
        }

        // Delete index metadata
        let idx_meta_key = format!("{}{}", COL_IDX_META_PREFIX, column);
        db_guard.delete_cf(cf, idx_meta_key.as_bytes()).map_err(|e| DbError::InternalError(e.to_string()))?;

        // Mark column as not indexed
        meta.columns[col_idx].indexed = false;

        // Update collection metadata
        let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
        let meta_bytes = serde_json::to_vec(&*meta)?;
        db_guard
            .put_cf(cf, meta_key.as_bytes(), &meta_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// List all indexes on this collection
    pub fn list_indexes(&self) -> DbResult<Vec<ColumnarIndexMeta>> {
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let mut indexes = Vec::new();
        for col_def in &meta.columns {
            if col_def.indexed {
                let idx_meta_key = format!("{}{}", COL_IDX_META_PREFIX, col_def.name);
                if let Ok(Some(bytes)) = db_guard.get_cf(cf, idx_meta_key.as_bytes()) {
                    if let Ok(idx_meta) = serde_json::from_slice::<ColumnarIndexMeta>(&bytes) {
                        indexes.push(idx_meta);
                    }
                }
            }
        }

        Ok(indexes)
    }

    /// Check if a column has an index
    pub fn has_index(&self, column: &str) -> bool {
        self.meta.read()
            .map(|m| m.columns.iter().any(|c| c.name == column && c.indexed))
            .unwrap_or(false)
    }

    /// Check if a column has a sorted index (for range queries)
    pub fn has_sorted_index(&self, column: &str) -> bool {
        if !self.has_index(column) {
            return false;
        }
        // Check index type
        if let Ok(db_guard) = self.db.read() {
            if let Some(cf) = db_guard.cf_handle(&self.cf_name) {
                let idx_meta_key = format!("{}{}", COL_IDX_META_PREFIX, column);
                if let Ok(Some(bytes)) = db_guard.get_cf(cf, idx_meta_key.as_bytes()) {
                    if let Ok(idx_meta) = serde_json::from_slice::<ColumnarIndexMeta>(&bytes) {
                        return idx_meta.index_type == ColumnarIndexType::Sorted;
                    }
                }
            }
        }
        false
    }

    /// Get index type/info for a column
    pub fn get_index_type(&self, column: &str) -> Option<ColumnarIndexType> {
        self.meta.read().ok().and_then(|m| {
            m.columns.iter()
                .find(|c| c.name == column && c.indexed)
                .and_then(|c| c.index_type.clone())
        })
    }



    /// Lookup rows using Bitmap index
    fn index_lookup_bitmap(&self, column: &str, value: &Value) -> DbResult<Vec<u64>> {
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let idx_key = self.encode_bitmap_key(column, value);
        let mut row_ids = Vec::new();

        if let Ok(Some(bytes)) = db_guard.get_cf(cf, idx_key.as_bytes()) {
            if let Ok(bitmap) = self.decompress_data(&bytes, &CompressionType::Lz4) {
                for (byte_idx, &byte) in bitmap.iter().enumerate() {
                    if byte == 0 { continue; }
                    for bit_idx in 0..8 {
                        if (byte & (1 << bit_idx)) != 0 {
                            row_ids.push((byte_idx * 8 + bit_idx) as u64);
                        }
                    }
                }
            }
        }
        
        Ok(row_ids)
    }

    /// Get candidate chunks/rows from MinMax index
    fn get_candidate_rows_from_minmax(&self, column: &str, filter: &ColumnFilter) -> DbResult<Vec<u64>> {
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let row_count = meta.row_count;
        let chunk_count = (row_count + MINMAX_CHUNK_SIZE - 1) / MINMAX_CHUNK_SIZE;

        let mut candidate_ids = Vec::new();

        for chunk_id in 0..chunk_count {
            let idx_key = self.encode_minmax_key(column, chunk_id);
            if let Ok(Some(bytes)) = db_guard.get_cf(cf, idx_key.as_bytes()) {
                if let Ok(chunk) = serde_json::from_slice::<MinMaxChunk>(&bytes) {
                    let mut matches = false;
                    match filter {
                        ColumnFilter::Gt(_, val) => {
                            matches = compare_values(&chunk.max, val) == std::cmp::Ordering::Greater;
                        }
                        ColumnFilter::Gte(_, val) => {
                            matches = compare_values(&chunk.max, val) != std::cmp::Ordering::Less;
                        }
                        ColumnFilter::Lt(_, val) => {
                            matches = compare_values(&chunk.min, val) == std::cmp::Ordering::Less;
                        }
                        ColumnFilter::Lte(_, val) => {
                            matches = compare_values(&chunk.min, val) != std::cmp::Ordering::Greater;
                        }
                        _ => {}
                    }

                    if matches {
                        let start = chunk_id * MINMAX_CHUNK_SIZE;
                        let end = (start + MINMAX_CHUNK_SIZE).min(row_count);
                        candidate_ids.extend(start..end);
                    }
                }
            }
        }

        Ok(candidate_ids)
    }

    /// Lookup rows matching a value using index (O(1) for equality)
    fn index_lookup_eq(&self, column: &str, value: &Value) -> DbResult<Vec<u64>> {
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let idx_key = self.encode_index_key(column, value);
        match db_guard.get_cf(cf, idx_key.as_bytes()) {
            Ok(Some(bytes)) => {
                let row_ids: Vec<u64> = serde_json::from_slice(&bytes)?;
                Ok(row_ids)
            }
            Ok(None) => Ok(Vec::new()),
            Err(e) => Err(DbError::InternalError(e.to_string())),
        }
    }

    /// Range scan using sorted index
    fn index_range_scan(&self, column: &str, filter: &ColumnFilter) -> DbResult<Vec<u64>> {
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let prefix = format!("{}{}:", COL_IDX_PREFIX, column);
        let iter = db_guard.prefix_iterator_cf(cf, prefix.as_bytes());

        let mut result = Vec::new();

        for item in iter {
            if let Ok((key, val_bytes)) = item {
                if !key.starts_with(prefix.as_bytes()) {
                    break;
                }

                // Extract the encoded value from key
                let key_str = String::from_utf8_lossy(&key);
                if let Some(encoded_val) = key_str.strip_prefix(&prefix) {
                    // Decode value for comparison
                    if let Some(val) = self.decode_index_value(encoded_val) {
                        let matches = match filter {
                            ColumnFilter::Gt(_, threshold) => {
                                compare_values(&val, threshold) == std::cmp::Ordering::Greater
                            }
                            ColumnFilter::Gte(_, threshold) => {
                                compare_values(&val, threshold) != std::cmp::Ordering::Less
                            }
                            ColumnFilter::Lt(_, threshold) => {
                                compare_values(&val, threshold) == std::cmp::Ordering::Less
                            }
                            ColumnFilter::Lte(_, threshold) => {
                                compare_values(&val, threshold) != std::cmp::Ordering::Greater
                            }
                            _ => false,
                        };

                        if matches {
                            if let Ok(row_ids) = serde_json::from_slice::<Vec<u64>>(&val_bytes) {
                                result.extend(row_ids);
                            }
                        }
                    }
                }
            }
        }

        result.sort();
        result.dedup();
        Ok(result)
    }

    // ==================== Private Helper Methods ====================

    /// Encode a value for index key (binary-comparable for sorted indexes)
    fn encode_index_key(&self, column: &str, value: &Value) -> String {
        let encoded = self.encode_value_for_key(value);
        format!("{}{}:{}", COL_IDX_PREFIX, column, encoded)
    }

    /// Encode key for bitmap index (similar to standard index but different prefix)
    fn encode_bitmap_key(&self, column: &str, value: &Value) -> String {
        let encoded = self.encode_value_for_key(value);
        format!("{}{}:{}", COL_IDX_BITMAP_PREFIX, column, encoded)
    }

    /// Encode key for MinMax index chunk
    fn encode_minmax_key(&self, column: &str, chunk_id: u64) -> String {
        format!("{}{}:{}", COL_IDX_MINMAX_PREFIX, column, chunk_id)
    }

    /// Encode value for key generation (shared logic)
    fn encode_value_for_key(&self, value: &Value) -> String {
        match value {
            Value::Null => "0:".to_string(),
            Value::Bool(b) => format!("1:{}", if *b { "1" } else { "0" }),
            Value::Number(n) => {
                // Encode numbers for lexicographic sorting
                if let Some(i) = n.as_i64() {
                    // Offset to handle negative numbers (add i64::MAX + 1)
                    let encoded = (i as u64).wrapping_add(0x8000000000000000);
                    format!("2:{:016x}", encoded)
                } else if let Some(f) = n.as_f64() {
                    // IEEE 754 float encoding for sorting
                    let bits = f.to_bits();
                    let encoded = if f >= 0.0 {
                        bits ^ 0x8000000000000000
                    } else {
                        !bits
                    };
                    format!("3:{:016x}", encoded)
                } else {
                    format!("9:{}", n)
                }
            }
            Value::String(s) => format!("4:{}", hex::encode(s.as_bytes())),
            _ => format!("9:{}", value),
        }
    }


    /// Decode index value from encoded string
    fn decode_index_value(&self, encoded: &str) -> Option<Value> {
        let parts: Vec<&str> = encoded.splitn(2, ':').collect();
        if parts.len() != 2 {
            return None;
        }

        match parts[0] {
            "0" => Some(Value::Null),
            "1" => Some(Value::Bool(parts[1] == "1")),
            "2" => {
                // Decode i64
                if let Ok(encoded) = u64::from_str_radix(parts[1], 16) {
                    let val = encoded.wrapping_sub(0x8000000000000000) as i64;
                    Some(Value::Number(val.into()))
                } else {
                    None
                }
            }
            "3" => {
                // Decode f64
                if let Ok(encoded) = u64::from_str_radix(parts[1], 16) {
                    let bits = if encoded & 0x8000000000000000 != 0 {
                        encoded ^ 0x8000000000000000
                    } else {
                        !encoded
                    };
                    let f = f64::from_bits(bits);
                    serde_json::Number::from_f64(f).map(Value::Number)
                } else {
                    None
                }
            }
            "4" => {
                // Decode string
                hex::decode(parts[1])
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .map(Value::String)
            }
            _ => None,
        }
    }

    /// Append a row ID to an index entry
    fn append_row_to_index(
        &self,
        db_guard: &DB,
        cf: &rocksdb::ColumnFamily,
        idx_key: &str,
        row_id: u64,
    ) -> DbResult<()> {
        let mut row_ids: Vec<u64> = match db_guard.get_cf(cf, idx_key.as_bytes()) {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes)?,
            Ok(None) => Vec::new(),
            Err(e) => return Err(DbError::InternalError(e.to_string())),
        };

        row_ids.push(row_id);

        let row_ids_bytes = serde_json::to_vec(&row_ids)?;
        db_guard
            .put_cf(cf, idx_key.as_bytes(), &row_ids_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// Update Bitmap index with a new row
    fn append_row_to_bitmap_index(
        &self,
        db_guard: &DB,
        cf: &rocksdb::ColumnFamily,
        idx_key: &str,
        row_id: u64,
        compression: &CompressionType,
    ) -> DbResult<()> {
        let bmp_compression = CompressionType::Lz4;
        let mut bitmap = match db_guard.get_cf(cf, idx_key.as_bytes()) {
            Ok(Some(bytes)) => self.decompress_data(&bytes, &bmp_compression)?,
            Ok(None) => Vec::new(),
            Err(e) => return Err(DbError::InternalError(e.to_string())),
        };

        // Ensure bitmap is large enough
        let byte_idx = (row_id / 8) as usize;
        if byte_idx >= bitmap.len() {
            bitmap.resize(byte_idx + 1, 0);
        }

        // Set bit
        let bit_offset = (row_id % 8) as usize;
        bitmap[byte_idx] |= 1 << bit_offset;

        // Compress and store
        let compressed = self.compress_data(&bitmap, &bmp_compression);
        db_guard
            .put_cf(cf, idx_key.as_bytes(), &compressed)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// Update MinMax index with a new row
    fn update_minmax_index(
        &self,
        db_guard: &DB,
        cf: &rocksdb::ColumnFamily,
        column: &str,
        value: &Value,
        row_id: u64,
    ) -> DbResult<()> {
        let chunk_id = row_id / MINMAX_CHUNK_SIZE;
        let idx_key = self.encode_minmax_key(column, chunk_id);

        let mut chunk: MinMaxChunk = match db_guard.get_cf(cf, idx_key.as_bytes()) {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes)?,
            Ok(None) => MinMaxChunk {
                min: value.clone(),
                max: value.clone(),
                count: 0,
            },
            Err(e) => return Err(DbError::InternalError(e.to_string())),
        };

        // Update min/max
        if compare_values(value, &chunk.min) == std::cmp::Ordering::Less {
            chunk.min = value.clone();
        }
        if compare_values(value, &chunk.max) == std::cmp::Ordering::Greater {
            chunk.max = value.clone();
        }
        chunk.count += 1;

        let chunk_bytes = serde_json::to_vec(&chunk)?;
        db_guard
            .put_cf(cf, idx_key.as_bytes(), &chunk_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// Update indexes when inserting a row (called from insert_rows)
    fn update_indexes_for_row(
        &self,
        db_guard: &DB,
        cf: &rocksdb::ColumnFamily,
        columns: &[ColumnDef],
        row: &serde_json::Map<String, Value>,
        row_id: u64,
    ) -> DbResult<()> {
        for col_def in columns {
            if col_def.indexed {
                let value = row.get(&col_def.name).unwrap_or(&Value::Null);
                
                match col_def.index_type {
                    Some(ColumnarIndexType::Bitmap) => {
                        let idx_key = self.encode_bitmap_key(&col_def.name, value);
                        // Using explicit compression for bitmap even if collection uncompressed, 
                        // but here using collection's setting is consistent.
                        // Actually, bitmap MUST be compressed to be efficient. using LZ4 always for bitmap?
                        // The `append_row_to_bitmap_index` takes compression param.
                        // Let's use LZ4 by default for bitmaps if collection has None? 
                        // Or just use collection's. Plan says "LZ4 Compressed Bitset". 
                        // We'll use Lz4 explicitly for Bitmaps if we want to force it, 
                        // but passing current config is cleaner.
                        // Let's assume we use collection compression.
                        // Fetch compression from somewhere? We don't have it passed here.
                        // We need to pass it or read it.
                        // update_indexes_for_row doesn't take compression. See fix below.
                        // For now, let's assume Lz4.
                        self.append_row_to_bitmap_index(db_guard, cf, &idx_key, row_id, &CompressionType::Lz4)?;
                    }
                    Some(ColumnarIndexType::MinMax) => {
                        self.update_minmax_index(db_guard, cf, &col_def.name, value, row_id)?;
                    }
                    _ => {
                        // Default to Sorted/Hash (Storage is same: Inverted Index)
                        let idx_key = self.encode_index_key(&col_def.name, value);
                        self.append_row_to_index(db_guard, cf, &idx_key, row_id)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Update indexes when inserting a row with UUID (for UUID-based storage)
    /// Note: Bitmap indexes are not supported with UUIDs (they require positional IDs)
    fn update_indexes_for_row_uuid(
        &self,
        db_guard: &DB,
        cf: &rocksdb::ColumnFamily,
        columns: &[ColumnDef],
        row: &serde_json::Map<String, Value>,
        row_uuid: &str,
    ) -> DbResult<()> {
        for col_def in columns {
            if col_def.indexed {
                let value = row.get(&col_def.name).unwrap_or(&Value::Null);

                match col_def.index_type {
                    Some(ColumnarIndexType::Bitmap) => {
                        // Bitmap indexes require positional IDs, not UUIDs
                        // Fall back to standard inverted index for UUID-based rows
                        let idx_key = self.encode_index_key(&col_def.name, value);
                        self.append_uuid_to_index(db_guard, cf, &idx_key, row_uuid)?;
                    }
                    Some(ColumnarIndexType::MinMax) => {
                        // MinMax indexes track min/max per chunk - skip for UUID rows
                        // or we could track globally, but skip for now
                    }
                    _ => {
                        // Sorted/Hash: Use inverted index with UUIDs
                        let idx_key = self.encode_index_key(&col_def.name, value);
                        self.append_uuid_to_index(db_guard, cf, &idx_key, row_uuid)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Append a UUID to an index entry (for UUID-based storage)
    fn append_uuid_to_index(
        &self,
        db_guard: &DB,
        cf: &rocksdb::ColumnFamily,
        idx_key: &str,
        row_uuid: &str,
    ) -> DbResult<()> {
        // Use a different key suffix to distinguish UUID indexes from u64 indexes
        let uuid_idx_key = format!("{}_uuids", idx_key);

        let mut uuids: Vec<String> = match db_guard.get_cf(cf, uuid_idx_key.as_bytes()) {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes)?,
            Ok(None) => Vec::new(),
            Err(e) => return Err(DbError::InternalError(e.to_string())),
        };

        uuids.push(row_uuid.to_string());

        let uuids_bytes = serde_json::to_vec(&uuids)?;
        db_guard
            .put_cf(cf, uuid_idx_key.as_bytes(), &uuids_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    // Private helper methods

    fn compress_data(&self, data: &[u8], compression: &CompressionType) -> Vec<u8> {
        match compression {
            CompressionType::None => data.to_vec(),
            CompressionType::Lz4 => compress_prepend_size(data),
        }
    }

    fn decompress_data(&self, data: &[u8], compression: &CompressionType) -> DbResult<Vec<u8>> {
        match compression {
            CompressionType::None => Ok(data.to_vec()),
            CompressionType::Lz4 => decompress_size_prepended(data)
                .map_err(|e| DbError::InternalError(format!("LZ4 decompression failed: {}", e))),
        }
    }

    fn compute_aggregate(&self, values: &[Value], op: AggregateOp) -> Value {
        match op {
            AggregateOp::Count => Value::Number(values.len().into()),

            AggregateOp::Sum => {
                let sum: f64 = values
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .sum();
                json_number(sum)
            }

            AggregateOp::Avg => {
                let nums: Vec<f64> = values.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    Value::Null
                } else {
                    let avg = nums.iter().sum::<f64>() / nums.len() as f64;
                    json_number(avg)
                }
            }

            AggregateOp::Min => {
                values
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                    .map(json_number)
                    .unwrap_or(Value::Null)
            }

            AggregateOp::Max => {
                values
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                    .map(json_number)
                    .unwrap_or(Value::Null)
            }

            AggregateOp::CountDistinct => {
                let distinct: std::collections::HashSet<String> = values
                    .iter()
                    .map(|v| v.to_string())
                    .collect();
                Value::Number(distinct.len().into())
            }
        }
    }

    fn apply_filter(
        &self,
        filter: &ColumnFilter,
        row_range: std::ops::Range<u64>,
    ) -> DbResult<Vec<u64>> {
        match filter {
            ColumnFilter::Eq(col, val) => {
                // Check index type
                if let Some(idx_type) = self.get_index_type(col) {
                    match idx_type {
                        ColumnarIndexType::Bitmap => return self.index_lookup_bitmap(col, val),
                        ColumnarIndexType::Sorted | ColumnarIndexType::Hash => return self.index_lookup_eq(col, val),
                        _ => {} // MinMax bad for equality usually, fall back to scan
                    }
                }
                
                // Fallback to full scan
                let values = self.read_column(col, None)?;
                Ok(row_range
                    .filter(|&i| values.get(i as usize).map(|v| v == val).unwrap_or(false))
                    .collect())
            }

            ColumnFilter::Ne(col, val) => {
                let values = self.read_column(col, None)?;
                Ok(row_range
                    .filter(|&i| values.get(i as usize).map(|v| v != val).unwrap_or(false))
                    .collect())
            }

            ColumnFilter::Gt(col, val) => {
                // Use sorted index if available
                if self.has_sorted_index(col) {
                    return self.index_range_scan(col, filter);
                }

                // Use MinMax to get candidates
                let mut candidates = None;
                if let Some(ColumnarIndexType::MinMax) = self.get_index_type(col) {
                    candidates = Some(self.get_candidate_rows_from_minmax(col, filter)?);
                }

                // If we have candidates, use them to restrict read. 
                // However, read_column reads raw values. We still need to filter them.
                // The candidates vector effectively replaces the row_range iteration space, 
                // but we also need to pass it to read_column to avoid IO on skipped chunks.
                
                // Optimization: calling read_column with candidates only reads those blocks.
                // Then filter the results.
                
                // Since `apply_filter` logic below filters based on ALL values returned by `read_column`,
                // and `read_column` returns values *corresponding* to inputs if provided...
                // Wait, read_column returns Vec<Value>. If input IDs are [0, 5], it returns [v0, v5].
                // The iterating logic below `values.get(i)` assumes `values` is indexed by row_id from 0??
                // Checking read_column: "Read a single column for all rows or specific row IDs"
                // If row_ids provided, it returns values for ONLY those rows.
                // BUT the filter logic below `values.get(i as usize)` accesses by row index.
                // If `read_column` returns partial vector, we can't index it by row_id directly.
                // The existing logic matches: `let values = self.read_column(col, None)?;` -> full column.
                
                // So I MUST restructure the scan logic to handle partial reads.
                if let Some(cand_ids) = candidates {
                    let values = self.read_column_by_positions(col, &cand_ids)?;
                    // Zip candidates with values
                    let threshold = val.as_f64().unwrap_or(f64::NEG_INFINITY);
                    Ok(cand_ids.into_iter().zip(values.into_iter())
                        .filter(|(_, v): &(_, Value)| {
                             v.as_f64().map(|n| n > threshold).unwrap_or(false)
                        })
                        .map(|(id, _)| id)
                        .collect())
                } else {
                    let values = self.read_column(col, None)?;
                    let threshold = val.as_f64().unwrap_or(f64::NEG_INFINITY);
                    Ok(row_range
                        .filter(|&i| {
                            values
                                .get(i as usize)
                                .and_then(|v| v.as_f64())
                                .map(|n| n > threshold)
                                .unwrap_or(false)
                        })
                        .collect())
                }
            }


            ColumnFilter::Gte(col, val) => {
                // Use sorted index if available
                if self.has_sorted_index(col) {
                    return self.index_range_scan(col, filter);
                }

                // Use MinMax
                let mut candidates = None;
                if let Some(ColumnarIndexType::MinMax) = self.get_index_type(col) {
                    candidates = Some(self.get_candidate_rows_from_minmax(col, filter)?);
                }

                if let Some(cand_ids) = candidates {
                    let values = self.read_column_by_positions(col, &cand_ids)?;
                    let threshold = val.as_f64().unwrap_or(f64::NEG_INFINITY);
                    Ok(cand_ids.into_iter().zip(values.into_iter())
                        .filter(|(_, v): &(_, Value)| {
                             v.as_f64().map(|n| n >= threshold).unwrap_or(false)
                        })
                        .map(|(id, _)| id)
                        .collect())
                } else {
                    let values = self.read_column(col, None)?;
                    let threshold = val.as_f64().unwrap_or(f64::NEG_INFINITY);
                    Ok(row_range
                        .filter(|&i| {
                            values
                                .get(i as usize)
                                .and_then(|v| v.as_f64())
                                .map(|n| n >= threshold)
                                .unwrap_or(false)
                        })
                        .collect())
                }
            }

            ColumnFilter::Lt(col, val) => {
                // Use sorted index if available
                if self.has_sorted_index(col) {
                    return self.index_range_scan(col, filter);
                }
                
                // Use MinMax
                let mut candidates = None;
                if let Some(ColumnarIndexType::MinMax) = self.get_index_type(col) {
                    candidates = Some(self.get_candidate_rows_from_minmax(col, filter)?);
                }

                if let Some(cand_ids) = candidates {
                    let values = self.read_column_by_positions(col, &cand_ids)?;
                    let threshold = val.as_f64().unwrap_or(f64::INFINITY);
                    Ok(cand_ids.into_iter().zip(values.into_iter())
                        .filter(|(_, v): &(_, Value)| {
                             v.as_f64().map(|n| n < threshold).unwrap_or(false)
                        })
                        .map(|(id, _)| id)
                        .collect())
                } else {
                    let values = self.read_column(col, None)?;
                    let threshold = val.as_f64().unwrap_or(f64::INFINITY);
                    Ok(row_range
                        .filter(|&i| {
                            values
                                .get(i as usize)
                                .and_then(|v| v.as_f64())
                                .map(|n| n < threshold)
                                .unwrap_or(false)
                        })
                        .collect())
                }
            }

            ColumnFilter::Lte(col, val) => {
                // Use sorted index if available
                if self.has_sorted_index(col) {
                    return self.index_range_scan(col, filter);
                }
                
                // Use MinMax
                let mut candidates = None;
                if let Some(ColumnarIndexType::MinMax) = self.get_index_type(col) {
                    candidates = Some(self.get_candidate_rows_from_minmax(col, filter)?);
                }

                if let Some(cand_ids) = candidates {
                    let values = self.read_column_by_positions(col, &cand_ids)?;
                    let threshold = val.as_f64().unwrap_or(f64::INFINITY);
                    Ok(cand_ids.into_iter().zip(values.into_iter())
                        .filter(|(_, v): &(_, Value)| {
                             v.as_f64().map(|n| n <= threshold).unwrap_or(false)
                        })
                        .map(|(id, _)| id)
                        .collect())
                } else {
                    let values = self.read_column(col, None)?;
                    let threshold = val.as_f64().unwrap_or(f64::INFINITY);
                    Ok(row_range
                        .filter(|&i| {
                            values
                                .get(i as usize)
                                .and_then(|v| v.as_f64())
                                .map(|n| n <= threshold)
                                .unwrap_or(false)
                        })
                        .collect())
                }
            }

            ColumnFilter::In(col, vals) => {
                // Use index if available - lookup each value and union results
                // Check index type to dispatch correctly
                if let Some(idx_type) = self.get_index_type(col) {
                   match idx_type {
                        ColumnarIndexType::Bitmap | ColumnarIndexType::Hash | ColumnarIndexType::Sorted => {
                            let mut result = Vec::new();
                            for val in vals {
                                if idx_type == ColumnarIndexType::Bitmap {
                                    result.extend(self.index_lookup_bitmap(col, val)?);
                                } else {
                                    result.extend(self.index_lookup_eq(col, val)?);
                                }
                            }
                            result.sort();
                            result.dedup();
                            return Ok(result);
                        }
                        _ => {}
                   }
                }
                
                let values = self.read_column(col, None)?;
                Ok(row_range
                    .filter(|&i| {
                        values
                            .get(i as usize)
                            .map(|v| vals.contains(v))
                            .unwrap_or(false)
                    })
                    .collect())
            }

            ColumnFilter::And(filters) => {
                let mut result: Vec<u64> = row_range.collect();
                for f in filters {
                    let matching = self.apply_filter(f, 0..result.len() as u64)?;
                    let matching_set: std::collections::HashSet<u64> = matching.into_iter().collect();
                    result.retain(|id| matching_set.contains(id));
                }
                Ok(result)
            }

            ColumnFilter::Or(filters) => {
                let mut result_set: std::collections::HashSet<u64> = std::collections::HashSet::new();
                for f in filters {
                    let matching = self.apply_filter(f, row_range.clone())?;
                    result_set.extend(matching);
                }
                let mut result: Vec<u64> = result_set.into_iter().collect();
                result.sort();
                Ok(result)
            }
        }
    }
}

/// Helper to create JSON number from f64
fn json_number(n: f64) -> Value {
    serde_json::Number::from_f64(n)
        .map(Value::Number)
        .unwrap_or_else(|| Value::Number(serde_json::Number::from(n as i64)))
}

/// Compare two JSON values for ordering
fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Number(na), Value::Number(nb)) => {
            let fa = na.as_f64().unwrap_or(0.0);
            let fb = nb.as_f64().unwrap_or(0.0);
            fa.partial_cmp(&fb).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::String(sa), Value::String(sb)) => sa.cmp(sb),
        (Value::Bool(ba), Value::Bool(bb)) => ba.cmp(bb),
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        _ => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_column_type_inference() {
        assert_eq!(ColumnType::infer_from_value(&Value::Bool(true)), ColumnType::Bool);
        assert_eq!(ColumnType::infer_from_value(&serde_json::json!(42)), ColumnType::Int64);
        assert_eq!(ColumnType::infer_from_value(&serde_json::json!(3.14)), ColumnType::Float64);
        assert_eq!(ColumnType::infer_from_value(&serde_json::json!("hello")), ColumnType::String);
        assert_eq!(
            ColumnType::infer_from_value(&serde_json::json!("2024-01-01T00:00:00Z")),
            ColumnType::Timestamp
        );
        assert_eq!(
            ColumnType::infer_from_value(&serde_json::json!({"nested": true})),
            ColumnType::Json
        );
    }

    #[test]
    fn test_aggregate_op_parsing() {
        assert_eq!(AggregateOp::from_str("SUM"), Some(AggregateOp::Sum));
        assert_eq!(AggregateOp::from_str("avg"), Some(AggregateOp::Avg));
        assert_eq!(AggregateOp::from_str("COUNT"), Some(AggregateOp::Count));
        assert_eq!(AggregateOp::from_str("MIN"), Some(AggregateOp::Min));
        assert_eq!(AggregateOp::from_str("MAX"), Some(AggregateOp::Max));
        assert_eq!(AggregateOp::from_str("unknown"), None);
    }

    #[test]
    fn test_lz4_compression_roundtrip() {
        // Test LZ4 compression directly without needing a full ColumnarCollection
        let data = b"Hello, world! This is test data for compression. Repeated text helps compression.";

        // Compress with LZ4
        let compressed = compress_prepend_size(data);

        // Verify compression happened (compressed should be smaller or similar for small data)
        assert!(!compressed.is_empty());

        // Decompress
        let decompressed = decompress_size_prepended(&compressed).unwrap();

        // Verify roundtrip
        assert_eq!(data.to_vec(), decompressed);
    }

    #[test]
    fn test_compression_type_default() {
        assert_eq!(CompressionType::default(), CompressionType::Lz4);
    }
}

mod parsers {
    pub fn parse_interval(s: &str) -> Option<(i64, &str)> {
        let len = s.len();
        if len < 2 {
            return None;
        }
        let (num_part, unit_part) = s.split_at(len - 1);
        if let Ok(num) = num_part.parse::<i64>() {
            Some((num, unit_part))
        } else {
            None
        }
    }
}
