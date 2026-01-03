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
    pub fn insert_rows(&self, rows: Vec<Value>) -> DbResult<usize> {
        if rows.is_empty() {
            return Ok(0);
        }

        let mut meta = self.meta.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.write().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let start_row_id = meta.row_count;
        let mut inserted = 0;

        for (i, row) in rows.iter().enumerate() {
            let row_id = start_row_id + i as u64;

            if let Value::Object(obj) = row {
                // Store each column value separately
                for col_def in &meta.columns {
                    let value = obj.get(&col_def.name).unwrap_or(&Value::Null);
                    let col_key = format!("{}{}:{}", COL_DATA_PREFIX, col_def.name, row_id);

                    let value_bytes = serde_json::to_vec(value)?;
                    let stored_bytes = self.compress_data(&value_bytes, &meta.compression);

                    db_guard
                        .put_cf(cf, col_key.as_bytes(), &stored_bytes)
                        .map_err(|e| DbError::InternalError(e.to_string()))?;
                }

                // Also store full row for reconstruction
                let row_key = format!("{}{}", COL_ROW_PREFIX, row_id);
                let row_bytes = serde_json::to_vec(row)?;
                let stored_row = self.compress_data(&row_bytes, &meta.compression);
                db_guard
                    .put_cf(cf, row_key.as_bytes(), &stored_row)
                    .map_err(|e| DbError::InternalError(e.to_string()))?;

                inserted += 1;
            }
        }

        // Update metadata
        meta.row_count += inserted as u64;
        meta.last_updated_at = chrono::Utc::now().timestamp();

        let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
        let meta_bytes = serde_json::to_vec(&*meta)?;
        db_guard
            .put_cf(cf, meta_key.as_bytes(), &meta_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(inserted)
    }

    /// Read a single column for all rows or specific row IDs
    pub fn read_column(&self, column: &str, row_ids: Option<&[u64]>) -> DbResult<Vec<Value>> {
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let db_guard = self.db.read().map_err(|e| DbError::InternalError(e.to_string()))?;
        let cf = db_guard.cf_handle(&self.cf_name).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        // Validate column exists
        if !meta.columns.iter().any(|c| c.name == column) {
            return Err(DbError::BadRequest(format!("Column '{}' not found", column)));
        }

        let ids: Vec<u64> = match row_ids {
            Some(ids) => ids.to_vec(),
            None => (0..meta.row_count).collect(),
        };

        let mut values = Vec::with_capacity(ids.len());

        for row_id in ids {
            let col_key = format!("{}{}:{}", COL_DATA_PREFIX, column, row_id);

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

    /// Read multiple columns (projection) for all rows or specific row IDs
    pub fn read_columns(&self, columns: &[&str], row_ids: Option<&[u64]>) -> DbResult<Vec<Value>> {
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

        let ids: Vec<u64> = match row_ids {
            Some(ids) => ids.to_vec(),
            None => (0..meta.row_count).collect(),
        };

        let mut results = Vec::with_capacity(ids.len());

        for row_id in ids {
            let mut row_obj = serde_json::Map::new();

            for col in columns {
                let col_key = format!("{}{}:{}", COL_DATA_PREFIX, col, row_id);

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
        let meta = self.meta.read().map_err(|e| DbError::InternalError(e.to_string()))?;

        // Read group columns and aggregation column
        let mut group_data: HashMap<String, Vec<u64>> = HashMap::new();

        for row_id in 0..meta.row_count {
            // Build group key
            let mut group_key_parts = Vec::new();
            for col_def in group_columns {
                let values = self.read_column(col_def.name(), Some(&[row_id]))?;
                if let Some(v) = values.first() {
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
            }
            let group_key = group_key_parts.join("|");

            group_data.entry(group_key).or_default().push(row_id);
        }

        // Compute aggregates for each group
        let mut results = Vec::new();

        for (group_key, row_ids) in group_data {
            let agg_values = self.read_column(agg_column, Some(&row_ids))?;
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

        // Find matching row IDs
        let matching_ids = self.apply_filter(filter, 0..meta.row_count)?;

        // Return projected columns for matching rows
        self.read_columns(columns, Some(&matching_ids))
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


            ColumnFilter::Gte(col, val) => {
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

            ColumnFilter::Lt(col, val) => {
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

            ColumnFilter::Lte(col, val) => {
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

            ColumnFilter::In(col, vals) => {
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
