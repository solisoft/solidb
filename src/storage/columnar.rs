//! Columnar Storage for Analytics/Reporting Workloads
//!
//! This module provides column-oriented storage optimized for:
//! - Read-heavy workloads (analytics, reporting)
//! - Aggregations (SUM, AVG, COUNT, MIN, MAX)
//! - Column pruning (only read needed columns)
//! - LZ4 compression for efficient storage

use super::columnar_blocks::{ColBlock, RowSet, BLOCK_ROWS, COL_BLOCK_PREFIX, SEAL_ROWS};
use super::keyspace::{Keyspace, KsBatchExt, KsCf, KsDbExt};
use crate::storage::kv::KvBatch;
use fastbloom::BloomFilter;
use lz4_flex::{compress_prepend_size, decompress_size_prepended};

use super::RocksDb as DB;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::error::{DbError, DbResult};

/// Key prefixes for columnar storage
const COL_DATA_PREFIX: &str = "col:"; // col:{column}:{row_id} -> compressed_value
const COL_META_PREFIX: &str = "col_meta:"; // col_meta:{collection} -> ColumnarCollectionMeta
const COL_ROW_PREFIX: &str = "col_row:"; // col_row:{row_id} -> full row for reconstruction
const COL_IDX_PREFIX: &str = "col_idx:"; // col_idx:{column}:{encoded_value} -> [row_ids]
const COL_IDX_META_PREFIX: &str = "col_idx_meta:"; // col_idx_meta:{column} -> ColumnarIndexMeta
const COL_IDX_BITMAP_PREFIX: &str = "col_idx_bmp:"; // col_idx_bmp:{column}:{encoded_value} -> [compressed_bitset]
const COL_IDX_MINMAX_PREFIX: &str = "col_idx_mm:"; // col_idx_mm:{column}:{chunk_id} -> {min, max}
const COL_IDX_BLOOM_PREFIX: &str = "col_idx_blo:"; // col_idx_blo:{column}:{chunk_id} -> [bloom_filter]

const MINMAX_CHUNK_SIZE: u64 = 1000;

/// One writer lock per (RocksDB instance, columnar CF), shared by every handle.
type WriteLockMap = dashmap::DashMap<(usize, String), Arc<parking_lot::Mutex<()>>>;
static COLUMNAR_WRITE_LOCKS: once_cell::sync::Lazy<WriteLockMap> =
    once_cell::sync::Lazy::new(dashmap::DashMap::new);

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
    /// How the rows are stored: 2 is blocks and a tail
    /// (`storage::columnar_blocks`); anything lower is one key per cell, and
    /// is converted when the collection is loaded.
    #[serde(default)]
    pub format: u8,
    /// Rows in the tail (`col_row:`), not yet sealed into a block.
    #[serde(default)]
    pub tail_rows: u64,
}

/// The storage format new collections are created in.
const BLOCK_FORMAT: u8 = 2;

/// The decoded blocks of a collection, `None` until they are read from
/// disk, and the database they belong to: the entry keeps that address
/// from being used again by another one.
type BlockStore = (
    std::sync::Weak<DB>,
    Arc<parking_lot::RwLock<Option<Vec<Arc<ColBlock>>>>>,
);
/// By (database, backing collection), shared by every handle.
static BLOCK_STORES: once_cell::sync::Lazy<dashmap::DashMap<(usize, String), BlockStore>> =
    once_cell::sync::Lazy::new(dashmap::DashMap::new);

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
    #[allow(clippy::should_implement_trait)]
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

impl ColumnFilter {
    /// Parse a filter from JSON: a leaf `{"column": c, "op": o, "value": v}`
    /// (`op` one of EQ, NE, GT, GTE, LT, LTE, IN, or the symbols), or
    /// `{"and": [..]}` / `{"or": [..]}` of filters.
    pub fn from_json(v: &Value) -> DbResult<Self> {
        Self::from_json_at(v, 0)
    }

    fn from_json_at(v: &Value, depth: usize) -> DbResult<Self> {
        const MAX_DEPTH: usize = 16;
        let bad = |m: &str| DbError::BadRequest(m.to_string());
        if depth > MAX_DEPTH {
            return Err(bad("filter is nested too deeply"));
        }
        let obj = v
            .as_object()
            .ok_or_else(|| bad("filter must be an object"))?;
        for (key, make) in [
            (
                "and",
                ColumnFilter::And as fn(Vec<ColumnFilter>) -> ColumnFilter,
            ),
            ("or", ColumnFilter::Or),
        ] {
            if let Some(items) = obj.get(key) {
                let items = items
                    .as_array()
                    .ok_or_else(|| bad("and/or take an array of filters"))?;
                return items
                    .iter()
                    .map(|i| Self::from_json_at(i, depth + 1))
                    .collect::<DbResult<Vec<_>>>()
                    .map(make);
            }
        }
        let column = obj
            .get("column")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("filter needs a string 'column'"))?
            .to_string();
        let op = obj
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("filter needs a string 'op'"))?;
        let value = obj.get("value").cloned().unwrap_or(Value::Null);
        match op.to_uppercase().as_str() {
            "EQ" | "=" | "==" => Ok(Self::Eq(column, value)),
            "NE" | "!=" | "<>" => Ok(Self::Ne(column, value)),
            "GT" | ">" => Ok(Self::Gt(column, value)),
            "GTE" | ">=" => Ok(Self::Gte(column, value)),
            "LT" | "<" => Ok(Self::Lt(column, value)),
            "LTE" | "<=" => Ok(Self::Lte(column, value)),
            "IN" => match value {
                Value::Array(arr) => Ok(Self::In(column, arr)),
                _ => Err(bad("IN operator requires an array value")),
            },
            other => Err(DbError::BadRequest(format!(
                "Unknown filter operator: {}. Supported: EQ, NE, GT, GTE, LT, LTE, IN",
                other
            ))),
        }
    }
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
    Sorted, // Supports equality + range queries (binary-comparable keys)
    Hash,   // Equality-only (faster for high-cardinality)
    Bitmap, // Low cardinality, fast boolean ops, compressed
    MinMax, // Range pruning for high cardinality/time-series
    Bloom,  // Probabilistic existence check per chunk
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
    db: Arc<DB>,
    /// Name of the backing collection, `db:_columnar_{name}`.
    cf_name: String,
    /// Where the backing collection keeps its keys.
    ks: Keyspace,
    meta: Arc<RwLock<ColumnarCollectionMeta>>,
}

impl Clone for ColumnarCollection {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            db: self.db.clone(),
            cf_name: self.cf_name.clone(),
            ks: self.ks.clone(),
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
        db: Arc<DB>,
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
            format: BLOCK_FORMAT,
            tail_rows: 0,
        };

        // Store metadata (lock-free, RocksDB is thread-safe)
        // Scope the CF handle so its borrow of `db` ends before `db` is moved
        let ks =
            super::collection_registry::keyspace_of(&db, &cf_name, |_| false).ok_or_else(|| {
                DbError::CollectionNotFound(format!("Columnar CF '{}' not found", cf_name))
            })?;
        {
            let cf = ks.handle(&db).ok_or_else(|| {
                DbError::CollectionNotFound(format!("Columnar CF '{}' not found", cf_name))
            })?;

            let meta_key = format!("{}{}", COL_META_PREFIX, name);
            let meta_bytes = serde_json::to_vec(&meta)?;
            db.put_ks(&cf, meta_key.as_bytes(), &meta_bytes)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        let created = Self {
            name,
            db,
            cf_name,
            ks,
            meta: Arc::new(RwLock::new(meta)),
        };
        // Nothing of an earlier collection of that name is kept.
        *created.store().write() = Some(Vec::new());
        Ok(created)
    }

    /// Load an existing columnar collection
    pub fn load(name: String, db_name: &str, db: Arc<DB>) -> DbResult<Self> {
        let cf_name = format!("{}:_columnar_{}", db_name, name);

        // Lock-free read - RocksDB is thread-safe
        // Scope the CF handle so its borrow of `db` ends before `db` is moved
        let ks =
            super::collection_registry::keyspace_of(&db, &cf_name, |_| false).ok_or_else(|| {
                DbError::CollectionNotFound(format!("Columnar collection '{}' not found", name))
            })?;
        let meta_bytes = {
            let cf = ks.handle(&db).ok_or_else(|| {
                DbError::CollectionNotFound(format!("Columnar collection '{}' not found", name))
            })?;

            let meta_key = format!("{}{}", COL_META_PREFIX, name);
            db.get_ks(&cf, meta_key.as_bytes())
                .map_err(|e| DbError::InternalError(e.to_string()))?
                .ok_or_else(|| {
                    DbError::CollectionNotFound(format!(
                        "Columnar collection metadata '{}' not found",
                        name
                    ))
                })?
        };

        let meta = serde_json::from_slice::<ColumnarCollectionMeta>(&meta_bytes)?;

        let format = meta.format;
        let loaded = Self {
            name,
            db,
            cf_name,
            ks,
            meta: Arc::new(RwLock::new(meta)),
        };
        if format < BLOCK_FORMAT {
            loaded.migrate()?;
        }
        Ok(loaded)
    }

    /// Lock shared by every handle of this columnar collection. The per-handle
    /// `meta` lock does not serialise writers: `load` builds independent
    /// handles, and two concurrent `insert_rows` lost `row_count` increments
    /// and each other's index appends (read-modify-write of the same keys).
    fn shared_write_lock(&self) -> Arc<parking_lot::Mutex<()>> {
        COLUMNAR_WRITE_LOCKS
            .entry((Arc::as_ptr(&self.db) as usize, self.cf_name.clone()))
            .or_default()
            .clone()
    }

    /// Reload metadata from disk; another handle may have written it since
    /// this one was loaded. Call under the shared write lock.
    fn refresh_meta(&self, db: &DB, cf: &KsCf, meta: &mut ColumnarCollectionMeta) {
        let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
        if let Ok(Some(bytes)) = db.get_ks(cf, meta_key.as_bytes()) {
            if let Ok(disk_meta) = serde_json::from_slice::<ColumnarCollectionMeta>(&bytes) {
                *meta = disk_meta;
            }
        }
    }

    fn stage_meta(
        &self,
        cf: &KsCf,
        batch: &mut KvBatch,
        meta: &ColumnarCollectionMeta,
    ) -> DbResult<()> {
        let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
        batch.put_ks(cf, meta_key.as_bytes(), serde_json::to_vec(meta)?);
        Ok(())
    }

    /// The decoded blocks of this collection, shared by every handle.
    fn store(&self) -> Arc<parking_lot::RwLock<Option<Vec<Arc<ColBlock>>>>> {
        let key = (Arc::as_ptr(&self.db) as usize, self.cf_name.clone());
        if let Some(found) = BLOCK_STORES.get(&key) {
            return found.1.clone();
        }
        BLOCK_STORES.retain(|_, (db, _)| db.strong_count() > 0);
        BLOCK_STORES
            .entry(key)
            .or_insert_with(|| (Arc::downgrade(&self.db), Arc::default()))
            .1
            .clone()
    }

    fn not_found(&self) -> DbError {
        DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
    }

    /// The blocks, in the order of their keys: from memory, or read once.
    fn blocks(&self, cf: &KsCf, compression: &CompressionType) -> DbResult<Vec<Arc<ColBlock>>> {
        let store = self.store();
        if let Some(blocks) = store.read().as_ref() {
            return Ok(blocks.clone());
        }
        let mut held = store.write();
        if let Some(blocks) = held.as_ref() {
            return Ok(blocks.clone());
        }
        let prefix = COL_BLOCK_PREFIX.as_bytes();
        let mut blocks = Vec::new();
        for (key, bytes) in self.db.prefix_iterator_ks(cf, prefix).flatten() {
            if !key.starts_with(prefix) {
                break;
            }
            let block = self
                .decompress_data(&bytes, compression)
                .ok()
                .and_then(|plain| ColBlock::decode(&plain))
                .ok_or_else(|| {
                    DbError::InternalError(format!(
                        "columnar block '{}' of '{}' cannot be read",
                        String::from_utf8_lossy(&key),
                        self.name
                    ))
                })?;
            blocks.push(Arc::new(block));
        }
        *held = Some(blocks.clone());
        Ok(blocks)
    }

    /// The rows of the tail, ascending by id.
    fn tail(
        &self,
        cf: &KsCf,
        compression: &CompressionType,
    ) -> DbResult<Vec<(String, serde_json::Map<String, Value>)>> {
        let prefix = COL_ROW_PREFIX.as_bytes();
        let mut rows = Vec::new();
        for (key, bytes) in self.db.prefix_iterator_ks(cf, prefix).flatten() {
            if !key.starts_with(prefix) {
                break;
            }
            let Some(uuid) = std::str::from_utf8(&key[prefix.len()..]).ok() else {
                continue;
            };
            let plain = self.decompress_data(&bytes, compression)?;
            // MessagePack, which holds a float exactly; rows written
            // through 2.10.4 are JSON text, which starts with `{` — to
            // MessagePack a small integer, not a map.
            let row = match rmp_serde::from_slice::<Value>(&plain) {
                Ok(Value::Object(row)) => Some(row),
                _ => match serde_json::from_slice::<Value>(&plain) {
                    Ok(Value::Object(row)) => Some(row),
                    _ => None,
                },
            };
            if let Some(row) = row {
                rows.push((uuid.to_string(), row));
            }
        }
        Ok(rows)
    }

    /// The rows as a query reads them: the blocks, then the tail.
    pub(crate) fn row_set(&self) -> DbResult<RowSet> {
        let compression = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?
            .compression
            .clone();
        self.row_set_with(&compression)
    }

    fn row_set_with(&self, compression: &CompressionType) -> DbResult<RowSet> {
        let cf = self.ks.handle(&self.db).ok_or_else(|| self.not_found())?;
        Ok(RowSet::new(
            self.blocks(&cf, compression)?,
            self.tail(&cf, compression)?,
        ))
    }

    /// Stage `block` into `batch`.
    fn stage_block(
        &self,
        cf: &KsCf,
        batch: &mut KvBatch,
        compression: &CompressionType,
        block: &ColBlock,
    ) -> DbResult<()> {
        let bytes = block
            .encode()
            .ok_or_else(|| DbError::InternalError("columnar block: encoding".to_string()))?;
        let key = format!("{}{}", COL_BLOCK_PREFIX, block.key);
        batch.put_ks(cf, key.as_bytes(), self.compress_data(&bytes, compression));
        Ok(())
    }

    /// Stage new rows (ascending by id) into `batch`: as blocks when there
    /// are enough of them, alone or with the tail, else in the tail. Returns
    /// the blocks sealed, for the store once the batch is written.
    fn stage_rows(
        &self,
        cf: &KsCf,
        batch: &mut KvBatch,
        meta: &mut ColumnarCollectionMeta,
        mut rows: Vec<(String, serde_json::Map<String, Value>)>,
    ) -> DbResult<Vec<ColBlock>> {
        let names: Vec<String> = meta.columns.iter().map(|c| c.name.clone()).collect();
        meta.row_count += rows.len() as u64;
        meta.last_updated_at = chrono::Utc::now().timestamp();
        if rows.len() < SEAL_ROWS && meta.tail_rows as usize + rows.len() < SEAL_ROWS {
            for (uuid, row) in &rows {
                let key = format!("{}{}", COL_ROW_PREFIX, uuid);
                let bytes = rmp_serde::to_vec(row)
                    .map_err(|e| DbError::InternalError(format!("columnar row: {e}")))?;
                batch.put_ks(
                    cf,
                    key.as_bytes(),
                    self.compress_data(&bytes, &meta.compression),
                );
            }
            meta.tail_rows += rows.len() as u64;
            return Ok(Vec::new());
        }
        // Enough for a block: with the tail, when these rows are few.
        if rows.len() < SEAL_ROWS {
            let tail = self.tail(cf, &meta.compression)?;
            for (uuid, _) in &tail {
                batch.delete_ks(cf, format!("{}{}", COL_ROW_PREFIX, uuid).as_bytes());
            }
            rows.extend(tail);
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            meta.tail_rows = 0;
        }
        let mut sealed = Vec::new();
        for chunk in rows.chunks(BLOCK_ROWS) {
            let block = ColBlock::seal(&names, chunk);
            self.stage_block(cf, batch, &meta.compression, &block)?;
            sealed.push(block);
        }
        Ok(sealed)
    }

    /// Make the store hold `sealed` too, once their batch is written.
    fn keep_blocks(&self, sealed: Vec<ColBlock>) {
        if sealed.is_empty() {
            return;
        }
        if let Some(blocks) = self.store().write().as_mut() {
            blocks.extend(sealed.into_iter().map(Arc::new));
            blocks.sort_by(|a, b| a.key.cmp(&b.key));
        }
    }

    /// Convert a collection stored one key per cell: its rows (`col_row:`
    /// holds each in full) are sealed into blocks, and the cells deleted.
    /// A conversion interrupted part-way starts again from the rows left.
    fn migrate(&self) -> DbResult<()> {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        self.refresh_meta(db, &cf, &mut meta);
        if meta.format >= BLOCK_FORMAT {
            return Ok(());
        }
        let names: Vec<String> = meta.columns.iter().map(|c| c.name.clone()).collect();
        let rows = self.tail(&cf, &meta.compression)?;
        for chunk in rows.chunks(BLOCK_ROWS) {
            let mut batch = KvBatch::default();
            self.stage_block(
                &cf,
                &mut batch,
                &meta.compression,
                &ColBlock::seal(&names, chunk),
            )?;
            for (uuid, _) in chunk {
                batch.delete_ks(&cf, format!("{}{}", COL_ROW_PREFIX, uuid).as_bytes());
            }
            db.write(batch)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }
        // `col;` follows every `col:` key and precedes `col_…`.
        let mut batch = KvBatch::default();
        batch.delete_range_ks(&cf, COL_DATA_PREFIX.as_bytes(), b"col;".as_slice());
        let mut new_meta = meta.clone();
        new_meta.format = BLOCK_FORMAT;
        new_meta.tail_rows = 0;
        self.stage_meta(&cf, &mut batch, &new_meta)?;
        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
        *self.store().write() = None;
        tracing::info!(
            "columnar collection '{}': {} rows converted to blocks",
            self.name,
            rows.len()
        );
        Ok(())
    }

    /// Insert rows into columnar storage
    /// Returns a vector of UUIDs for the inserted rows (for replication)
    ///
    /// The rows and the updated metadata land in one KvBatch, so a failure
    /// part-way leaves nothing half-written.
    pub fn insert_rows(&self, rows: Vec<Value>) -> DbResult<Vec<String>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        // Anything but an object is passed over.
        let objects: Vec<serde_json::Map<String, Value>> = rows
            .into_iter()
            .filter_map(|row| match row {
                Value::Object(obj) => Some(obj),
                _ => None,
            })
            .collect();
        // Enough rows for blocks of their own: their columns are built
        // here, before the lock every writer of the collection waits for.
        // The schema's columns do not change once the collection exists.
        let columns = (objects.len() >= SEAL_ROWS).then(|| {
            let names: Vec<String> = self
                .meta
                .read()
                .map(|meta| meta.columns.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default();
            objects
                .chunks(BLOCK_ROWS)
                .map(|chunk| (chunk.len(), ColBlock::of_rows(&names, chunk)))
                .collect::<Vec<_>>()
        });

        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        self.refresh_meta(db, &cf, &mut meta);

        // A UUID v7 (time-ordered) for each row, under the lock: the ids of
        // one insert follow those of the one before it.
        let inserted_ids: Vec<String> =
            objects.iter().map(|_| uuid7::uuid7().to_string()).collect();

        let mut batch = KvBatch::default();
        let mut new_meta = meta.clone();
        let sealed = match columns {
            Some(columns) => {
                let mut ids = inserted_ids.iter().cloned();
                let mut sealed = Vec::with_capacity(columns.len());
                for (rows, block) in columns {
                    let block = block.identified(ids.by_ref().take(rows).collect());
                    self.stage_block(&cf, &mut batch, &new_meta.compression, &block)?;
                    sealed.push(block);
                }
                new_meta.row_count += objects.len() as u64;
                new_meta.last_updated_at = chrono::Utc::now().timestamp();
                sealed
            }
            None => {
                let fresh = inserted_ids.iter().cloned().zip(objects).collect();
                self.stage_rows(&cf, &mut batch, &mut new_meta, fresh)?
            }
        };
        self.stage_meta(&cf, &mut batch, &new_meta)?;

        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
        self.keep_blocks(sealed);

        Ok(inserted_ids)
    }

    /// Insert rows under the ids their origin gave them (a full sync sends
    /// a collection this way). A row whose id is already here is passed
    /// over, so the same rows may arrive twice. Returns how many went in.
    ///
    /// Unlike [`Self::insert_row_with_id`], enough rows are sealed as
    /// blocks at once.
    pub fn insert_rows_with_ids(
        &self,
        rows: Vec<(String, serde_json::Map<String, Value>)>,
    ) -> DbResult<usize> {
        if rows.is_empty() {
            return Ok(0);
        }
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        self.refresh_meta(db, &cf, &mut meta);

        let mut fresh = {
            let here = self.row_set_with(&meta.compression)?;
            rows.into_iter()
                .filter(|(id, _)| here.find(id).is_none())
                .collect::<Vec<_>>()
        };
        // `stage_rows` takes them ascending, each id once.
        fresh.sort_by(|a, b| a.0.cmp(&b.0));
        fresh.dedup_by(|a, b| a.0 == b.0);
        if fresh.is_empty() {
            return Ok(0);
        }
        let inserted = fresh.len();

        let mut batch = KvBatch::default();
        let mut new_meta = meta.clone();
        let sealed = self.stage_rows(&cf, &mut batch, &mut new_meta, fresh)?;
        self.stage_meta(&cf, &mut batch, &new_meta)?;
        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
        self.keep_blocks(sealed);
        Ok(inserted)
    }

    /// Insert a row with a specific UUID (for replication)
    /// This is idempotent - if the row already exists, it's skipped
    pub fn insert_row_with_id(&self, row_uuid: &str, row: Value) -> DbResult<bool> {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        self.refresh_meta(db, &cf, &mut meta);

        // Check if row already exists (idempotency for replication)
        if self
            .row_set_with(&meta.compression)?
            .find(row_uuid)
            .is_some()
        {
            return Ok(false); // Already exists, skip
        }

        let Value::Object(obj) = row else {
            return Err(DbError::BadRequest("Row must be a JSON object".to_string()));
        };
        let mut batch = KvBatch::default();
        let mut new_meta = meta.clone();
        let sealed = self.stage_rows(
            &cf,
            &mut batch,
            &mut new_meta,
            vec![(row_uuid.to_string(), obj)],
        )?;
        self.stage_meta(&cf, &mut batch, &new_meta)?;

        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
        self.keep_blocks(sealed);

        Ok(true)
    }

    /// Delete a row by UUID
    pub fn delete_row(&self, row_uuid: &str) -> DbResult<bool> {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        self.refresh_meta(db, &cf, &mut meta);

        let rows = self.row_set_with(&meta.compression)?;
        let Some((at, row)) = rows.find(row_uuid) else {
            return Ok(false); // Doesn't exist
        };

        let mut batch = KvBatch::default();
        let mut new_meta = meta.clone();
        // In the tail: its key goes. In a block: the block is written again
        // without it, under the same key.
        let rewritten = match rows.blocks.get(at) {
            None => {
                batch.delete_ks(&cf, format!("{}{}", COL_ROW_PREFIX, row_uuid).as_bytes());
                new_meta.tail_rows = new_meta.tail_rows.saturating_sub(1);
                None
            }
            Some(block) => {
                let smaller = block.without(row);
                if smaller.len() == 0 {
                    let key = format!("{}{}", COL_BLOCK_PREFIX, block.key);
                    batch.delete_ks(&cf, key.as_bytes());
                } else {
                    self.stage_block(&cf, &mut batch, &meta.compression, &smaller)?;
                }
                Some(smaller)
            }
        };

        // Update metadata
        new_meta.row_count = new_meta.row_count.saturating_sub(1);
        new_meta.last_updated_at = chrono::Utc::now().timestamp();
        self.stage_meta(&cf, &mut batch, &new_meta)?;

        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
        if let (Some(smaller), Some(blocks)) = (rewritten, self.store().write().as_mut()) {
            blocks.retain(|b| b.key != smaller.key);
            if smaller.len() > 0 {
                blocks.push(Arc::new(smaller));
                blocks.sort_by(|a, b| a.key.cmp(&b.key));
            }
        }

        Ok(true)
    }

    /// List all row UUIDs in the collection
    pub fn list_row_uuids(&self) -> DbResult<Vec<String>> {
        let rows = self.row_set()?;
        Ok(rows
            .positions()
            .map(|at| rows.uuid(at).to_string())
            .collect())
    }

    fn check_column(&self, column: &str) -> DbResult<()> {
        let meta = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        if !meta.columns.iter().any(|c| c.name == column) {
            return Err(DbError::BadRequest(format!(
                "Column '{}' not found",
                column
            )));
        }
        Ok(())
    }

    /// Read a single column for all rows or specific row IDs
    pub fn read_column(&self, column: &str, row_uuids: Option<&[String]>) -> DbResult<Vec<Value>> {
        self.check_column(column)?;
        let rows = self.row_set()?;
        Ok(match row_uuids {
            None => rows.positions().map(|at| rows.value(at, column)).collect(),
            Some(ids) => ids
                .iter()
                .map(|id| {
                    rows.find(id)
                        .map_or(Value::Null, |at| rows.value(at, column))
                })
                .collect(),
        })
    }

    /// Read multiple columns (projection) for all rows or specific row UUIDs
    pub fn read_columns(
        &self,
        columns: &[&str],
        row_uuids: Option<&[String]>,
    ) -> DbResult<Vec<Value>> {
        for column in columns {
            self.check_column(column)?;
        }
        let rows = self.row_set()?;
        let object = |at: Option<(usize, usize)>| {
            Value::Object(
                columns
                    .iter()
                    .map(|column| {
                        let value = at.map_or(Value::Null, |at| rows.value(at, column));
                        (column.to_string(), value)
                    })
                    .collect(),
            )
        };
        Ok(match row_uuids {
            None => rows.positions().map(|at| object(Some(at))).collect(),
            Some(ids) => ids.iter().map(|id| object(rows.find(id))).collect(),
        })
    }

    /// Get total row count
    pub fn count(&self) -> usize {
        self.meta.read().map(|m| m.row_count as usize).unwrap_or(0)
    }

    /// Read a single column value for a specific row UUID
    fn read_column_value(&self, column: &str, row_uuid: &str) -> DbResult<Value> {
        let rows = self.row_set()?;
        Ok(rows
            .find(row_uuid)
            .map_or(Value::Null, |at| rows.value(at, column)))
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

    /// Aggregate a column with the specified operation: over its arrays,
    /// and the rows of the tail. Count counts rows, nulls included; the
    /// others take numbers and return floats.
    pub fn aggregate(&self, column: &str, op: AggregateOp) -> DbResult<Value> {
        let rows = self.row_set()?;
        let numbers = || {
            let in_blocks = rows.blocks.iter().flat_map(move |block| {
                let held = block.column(column);
                (0..block.len()).filter_map(move |row| held.and_then(|c| c.number(row)))
            });
            let in_tail = rows
                .tail
                .iter()
                .filter_map(move |(_, row)| row.get(column).and_then(Value::as_f64));
            in_blocks.chain(in_tail)
        };
        Ok(match op {
            AggregateOp::Count => Value::Number(rows.len().into()),
            AggregateOp::Sum => json_number(numbers().sum()),
            AggregateOp::Avg => {
                let (sum, count) = numbers().fold((0.0, 0u64), |(s, n), x| (s + x, n + 1));
                if count == 0 {
                    Value::Null
                } else {
                    json_number(sum / count as f64)
                }
            }
            AggregateOp::Min => numbers()
                .reduce(|a, b| if b < a { b } else { a })
                .map_or(Value::Null, json_number),
            AggregateOp::Max => numbers()
                .reduce(|a, b| if b > a { b } else { a })
                .map_or(Value::Null, json_number),
            AggregateOp::CountDistinct => {
                let distinct: std::collections::HashSet<String> = rows
                    .positions()
                    .map(|at| rows.value(at, column).to_string())
                    .collect();
                Value::Number(distinct.len().into())
            }
        })
    }

    /// Group by with aggregation
    /// Group by with aggregation
    pub fn group_by(
        &self,
        group_columns: &[GroupByColumn],
        agg_column: &str,
        op: AggregateOp,
    ) -> DbResult<Vec<Value>> {
        self.group_by_rows(group_columns, agg_column, op, None)
    }

    /// [`Self::group_by`] restricted to the rows matching `filter`.
    pub fn group_by_where(
        &self,
        filter: &ColumnFilter,
        group_columns: &[GroupByColumn],
        agg_column: &str,
        op: AggregateOp,
    ) -> DbResult<Vec<Value>> {
        let rows = self.filtered_row_uuids(filter)?;
        self.group_by_rows(group_columns, agg_column, op, Some(rows))
    }

    /// [`Self::aggregate`] restricted to the rows matching `filter`.
    pub fn aggregate_where(
        &self,
        filter: &ColumnFilter,
        column: &str,
        op: AggregateOp,
    ) -> DbResult<Value> {
        let rows = self.filtered_row_uuids(filter)?;
        let values = self.read_column(column, Some(&rows))?;
        Ok(self.compute_aggregate(&values, op))
    }

    fn filtered_row_uuids(&self, filter: &ColumnFilter) -> DbResult<Vec<String>> {
        let row_count = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?
            .row_count;
        let positions = self.apply_filter(filter, 0..row_count)?;
        self.positions_to_uuids(&positions)
    }

    fn group_by_rows(
        &self,
        group_columns: &[GroupByColumn],
        agg_column: &str,
        op: AggregateOp,
        rows: Option<Vec<String>>,
    ) -> DbResult<Vec<Value>> {
        let all_uuids = match rows {
            Some(rows) => rows,
            None => self.list_row_uuids()?,
        };

        // Read group columns and aggregation column
        let mut group_data: HashMap<String, Vec<String>> = HashMap::new();

        for row_uuid in &all_uuids {
            // Build group key
            let mut group_key_parts = Vec::new();
            for col_def in group_columns {
                let v = self.read_column_value(col_def.name(), row_uuid)?;
                match col_def {
                    GroupByColumn::Simple(_) => {
                        // `Value::to_string` JSON-encodes, so a string column
                        // yielded `"\"a\""` — the quotes then survived into the
                        // result because the key is split and re-wrapped in
                        // `Value::String` below. Use the string's contents.
                        group_key_parts.push(match &v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        });
                    }
                    GroupByColumn::TimeBucket(_, interval) => {
                        // Try to parse timestamp and bucket it
                        let s = v.as_str().unwrap_or("");
                        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                            // Simple bucketing logic (naive implementation for POC)
                            // Parse interval
                            let (num, unit) = parsers::parse_interval(interval).unwrap_or((1, "h")); // Default to 1h if invalid

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

            group_data
                .entry(group_key)
                .or_default()
                .push(row_uuid.clone());
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
                            result_obj.insert(
                                col_def.name().to_string(),
                                Value::Number(
                                    serde_json::Number::from_f64(n)
                                        .unwrap_or(serde_json::Number::from(0)),
                                ),
                            );
                        } else {
                            result_obj.insert(
                                col_def.name().to_string(),
                                Value::String(part.to_string()),
                            );
                        }
                    } else {
                        result_obj
                            .insert(col_def.name().to_string(), Value::String(part.to_string()));
                    }
                }
            }

            result_obj.insert("_agg".to_string(), agg_result);
            results.push(Value::Object(result_obj));
        }

        Ok(results)
    }

    /// Scan with filter and column projection
    pub fn scan_filtered(&self, filter: &ColumnFilter, columns: &[&str]) -> DbResult<Vec<Value>> {
        let meta = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        // Find matching row indices (positions)
        let matching_positions = self.apply_filter(filter, 0..meta.row_count)?;

        // Convert positions to UUIDs
        let matching_uuids = self.positions_to_uuids(&matching_positions)?;

        // Return projected columns for matching rows
        self.read_columns(columns, Some(&matching_uuids))
    }

    /// Get collection statistics
    pub fn stats(&self) -> DbResult<ColumnarStats> {
        let meta = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let mut compressed_size = 0u64;
        let mut uncompressed_size = 0u64;

        // What the blocks and the tail take, stored and decompressed.
        for prefix in [COL_BLOCK_PREFIX, COL_ROW_PREFIX] {
            let prefix = prefix.as_bytes();
            for (key, value) in db.prefix_iterator_ks(&cf, prefix).flatten() {
                if !key.starts_with(prefix) {
                    break;
                }
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
        let meta = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        Ok(meta.clone())
    }

    /// Truncate all data from the collection (preserves schema)
    pub fn truncate(&self) -> DbResult<()> {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;
        self.refresh_meta(db, &cf, &mut meta);

        // Row data, full rows and index entries. Each scan stops at the end
        // of its prefix: `col:` sorts right before `col_idx…`/`col_meta:`,
        // and an unbounded walk used to delete the schema along with the data.
        let mut batch = KvBatch::default();
        for prefix in [
            COL_DATA_PREFIX,
            COL_BLOCK_PREFIX,
            COL_ROW_PREFIX,
            COL_IDX_PREFIX,
            COL_IDX_BITMAP_PREFIX,
            COL_IDX_MINMAX_PREFIX,
            COL_IDX_BLOOM_PREFIX,
        ] {
            let prefix = prefix.as_bytes();
            for (key, _) in db.prefix_iterator_ks(&cf, prefix).flatten() {
                if !key.starts_with(prefix) {
                    break;
                }
                batch.delete_ks(&cf, &key);
            }
        }

        // Reset row count, saved under the key `load` reads (this used to
        // write a bare `meta` key nothing read back).
        let mut new_meta = meta.clone();
        new_meta.row_count = 0;
        new_meta.tail_rows = 0;
        new_meta.last_updated_at = chrono::Utc::now().timestamp();
        self.stage_meta(&cf, &mut batch, &new_meta)?;

        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to truncate: {}", e)))?;
        *meta = new_meta;
        *self.store().write() = Some(Vec::new());

        Ok(())
    }

    /// Drop the entire columnar collection (removes everything including schema)
    pub fn drop(&self) -> DbResult<()> {
        // Shared layout: a record and a range, no column family to drop.
        if let Some(ks) =
            super::collection_registry::get(&self.db, &self.cf_name).and_then(|r| r.ks)
        {
            let _catalog = super::collection_registry::catalog_lock();
            return super::collection_registry::drop_shared(&self.db, &self.cf_name, ks);
        }
        // MultiThreaded mode: drop_cf takes &self and synchronizes internally
        super::cf_ops::timed(|| self.db.drop_cf(&self.cf_name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop CF: {}", e)))?;

        // The column family was created through `Database::create_collection`,
        // so it has a registry entry. Dropping it here without deregistering
        // would leave the collection listed forever, with no column family
        // behind it.
        if let Err(e) = super::collection_registry::forget(&self.db, &self.cf_name) {
            tracing::warn!(
                "Failed to deregister columnar collection '{}': {}",
                self.cf_name,
                e
            );
        }

        Ok(())
    }

    // ==================== Index Methods ====================

    /// Create an index on a column
    pub fn create_index(&self, column: &str, index_type: ColumnarIndexType) -> DbResult<()> {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;
        self.refresh_meta(db, &cf, &mut meta);

        // Validate column exists and check if already indexed
        let col_idx = meta
            .columns
            .iter()
            .position(|c| c.name == column)
            .ok_or_else(|| DbError::BadRequest(format!("Column '{}' not found", column)))?;

        if meta.columns[col_idx].indexed {
            return Err(DbError::BadRequest(format!(
                "Column '{}' already has an index",
                column
            )));
        }

        // Copy values we need before building index
        let compression = meta.compression.clone();

        // Build index from the existing rows, by their position.
        let values: Vec<Value> = {
            let rows = self.row_set_with(&compression)?;
            rows.positions().map(|at| rows.value(at, column)).collect()
        };
        for (row_id, value) in values.iter().enumerate() {
            match index_type {
                ColumnarIndexType::Bitmap => {
                    let idx_key = self.encode_bitmap_key(column, value);
                    self.append_row_to_bitmap_index(
                        db,
                        &cf,
                        &idx_key,
                        row_id as u64,
                        &compression,
                    )?;
                }
                ColumnarIndexType::MinMax => {
                    self.update_minmax_index(db, &cf, column, value, row_id as u64)?;
                }
                ColumnarIndexType::Bloom => {
                    self.update_bloom_index(db, &cf, column, value, row_id as u64)?;
                }
                _ => {
                    let idx_key = self.encode_index_key(column, value);
                    self.append_row_to_index(db, &cf, &idx_key, row_id as u64)?;
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
        db.put_ks(&cf, idx_meta_key.as_bytes(), &idx_meta_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        // Mark column as indexed
        meta.columns[col_idx].indexed = true;
        meta.columns[col_idx].index_type = Some(index_type.clone());

        // Update collection metadata
        let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
        let meta_bytes = serde_json::to_vec(&*meta)?;
        db.put_ks(&cf, meta_key.as_bytes(), &meta_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// Drop an index from a column
    pub fn drop_index(&self, column: &str) -> DbResult<()> {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;
        self.refresh_meta(db, &cf, &mut meta);

        // Validate column exists and has index
        let col_idx = meta
            .columns
            .iter()
            .position(|c| c.name == column)
            .ok_or_else(|| DbError::BadRequest(format!("Column '{}' not found", column)))?;

        if !meta.columns[col_idx].indexed {
            return Err(DbError::BadRequest(format!(
                "Column '{}' has no index",
                column
            )));
        }

        // Delete all index entries for this column
        let prefix = format!("{}{}:", COL_IDX_PREFIX, column);
        let iter = db.prefix_iterator_ks(&cf, prefix.as_bytes());
        for (key, _) in iter.flatten() {
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            db.delete_ks(&cf, &key)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        // Delete index metadata
        let idx_meta_key = format!("{}{}", COL_IDX_META_PREFIX, column);
        db.delete_ks(&cf, idx_meta_key.as_bytes())
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        // Mark column as not indexed
        meta.columns[col_idx].indexed = false;

        // Update collection metadata
        let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
        let meta_bytes = serde_json::to_vec(&*meta)?;
        db.put_ks(&cf, meta_key.as_bytes(), &meta_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// List all indexes on this collection
    pub fn list_indexes(&self) -> DbResult<Vec<ColumnarIndexMeta>> {
        let meta = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let mut indexes = Vec::new();
        for col_def in &meta.columns {
            if col_def.indexed {
                let idx_meta_key = format!("{}{}", COL_IDX_META_PREFIX, col_def.name);
                if let Ok(Some(bytes)) = db.get_ks(&cf, idx_meta_key.as_bytes()) {
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
        self.meta
            .read()
            .map(|m| m.columns.iter().any(|c| c.name == column && c.indexed))
            .unwrap_or(false)
    }

    /// Check if a column has a sorted index (for range queries)
    pub fn has_sorted_index(&self, column: &str) -> bool {
        if !self.has_index(column) {
            return false;
        }
        // Check index type
        if let Some(cf) = self.ks.handle(&self.db) {
            let idx_meta_key = format!("{}{}", COL_IDX_META_PREFIX, column);
            if let Ok(Some(bytes)) = self.db.get_ks(&cf, idx_meta_key.as_bytes()) {
                if let Ok(idx_meta) = serde_json::from_slice::<ColumnarIndexMeta>(&bytes) {
                    return idx_meta.index_type == ColumnarIndexType::Sorted;
                }
            }
        }
        false
    }

    /// Get index type/info for a column
    pub fn get_index_type(&self, column: &str) -> Option<ColumnarIndexType> {
        self.meta.read().ok().and_then(|m| {
            m.columns
                .iter()
                .find(|c| c.name == column && c.indexed)
                .and_then(|c| c.index_type.clone())
        })
    }

    /// Lookup rows using Bitmap index
    fn index_lookup_bitmap(&self, column: &str, value: &Value) -> DbResult<Vec<u64>> {
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let idx_key = self.encode_bitmap_key(column, value);
        let mut row_ids = Vec::new();

        if let Ok(Some(bytes)) = db.get_ks(&cf, idx_key.as_bytes()) {
            if let Ok(bitmap) = self.decompress_data(&bytes, &CompressionType::Lz4) {
                for (byte_idx, &byte) in bitmap.iter().enumerate() {
                    if byte == 0 {
                        continue;
                    }
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
    fn get_candidate_rows_from_minmax(
        &self,
        column: &str,
        filter: &ColumnFilter,
    ) -> DbResult<Vec<u64>> {
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let meta = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let row_count = meta.row_count;
        let chunk_count = row_count.div_ceil(MINMAX_CHUNK_SIZE);

        let mut candidate_ids = Vec::new();

        for chunk_id in 0..chunk_count {
            let idx_key = self.encode_minmax_key(column, chunk_id);
            if let Ok(Some(bytes)) = db.get_ks(&cf, idx_key.as_bytes()) {
                if let Ok(chunk) = serde_json::from_slice::<MinMaxChunk>(&bytes) {
                    let mut matches = false;
                    match filter {
                        ColumnFilter::Gt(_, val) => {
                            matches =
                                compare_values(&chunk.max, val) == std::cmp::Ordering::Greater;
                        }
                        ColumnFilter::Gte(_, val) => {
                            matches = compare_values(&chunk.max, val) != std::cmp::Ordering::Less;
                        }
                        ColumnFilter::Lt(_, val) => {
                            matches = compare_values(&chunk.min, val) == std::cmp::Ordering::Less;
                        }
                        ColumnFilter::Lte(_, val) => {
                            matches =
                                compare_values(&chunk.min, val) != std::cmp::Ordering::Greater;
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

    /// Get candidate chunks/rows from Bloom index
    fn get_candidate_rows_from_bloom(&self, column: &str, value: &Value) -> DbResult<Vec<u64>> {
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let meta = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let row_count = meta.row_count;
        let chunk_count = row_count.div_ceil(MINMAX_CHUNK_SIZE);

        let mut candidate_ids = Vec::new();
        let val_str = value.to_string(); // Use consistent stringification

        for chunk_id in 0..chunk_count {
            let idx_key = self.encode_bloom_key(column, chunk_id);
            // If bloom filter missing, assume match (safe default)
            let mut matches = true;

            if let Ok(Some(bytes)) = db.get_ks(&cf, idx_key.as_bytes()) {
                if let Ok(filter) = serde_json::from_slice::<BloomFilter>(&bytes) {
                    matches = filter.contains(&val_str);
                }
            }

            if matches {
                let start = chunk_id * MINMAX_CHUNK_SIZE;
                let end = (start + MINMAX_CHUNK_SIZE).min(row_count);
                candidate_ids.extend(start..end);
            }
        }

        Ok(candidate_ids)
    }

    /// Lookup rows matching a value using index (O(1) for equality)
    fn index_lookup_eq(&self, column: &str, value: &Value) -> DbResult<Vec<u64>> {
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let idx_key = self.encode_index_key(column, value);
        match db.get_ks(&cf, idx_key.as_bytes()) {
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
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;

        let prefix = format!("{}{}:", COL_IDX_PREFIX, column);
        let iter = db.prefix_iterator_ks(&cf, prefix.as_bytes());

        let mut result = Vec::new();

        for (key, val_bytes) in iter.flatten() {
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

    /// Encode key for Bloom index chunk
    fn encode_bloom_key(&self, column: &str, chunk_id: u64) -> String {
        format!("{}{}:{}", COL_IDX_BLOOM_PREFIX, column, chunk_id)
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
    fn append_row_to_index(&self, db: &DB, cf: &KsCf, idx_key: &str, row_id: u64) -> DbResult<()> {
        let mut row_ids: Vec<u64> = match db.get_ks(cf, idx_key.as_bytes()) {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes)?,
            Ok(None) => Vec::new(),
            Err(e) => return Err(DbError::InternalError(e.to_string())),
        };

        row_ids.push(row_id);

        let row_ids_bytes = serde_json::to_vec(&row_ids)?;
        db.put_ks(cf, idx_key.as_bytes(), &row_ids_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// Update Bitmap index with a new row
    fn append_row_to_bitmap_index(
        &self,
        db: &DB,
        cf: &KsCf,
        idx_key: &str,
        row_id: u64,
        _compression: &CompressionType,
    ) -> DbResult<()> {
        let bmp_compression = CompressionType::Lz4;
        let mut bitmap = match db.get_ks(cf, idx_key.as_bytes()) {
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
        db.put_ks(cf, idx_key.as_bytes(), &compressed)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// Update Bloom index with a new row
    fn update_bloom_index(
        &self,
        db: &DB,
        cf: &KsCf,
        column: &str,
        value: &Value,
        row_id: u64,
    ) -> DbResult<()> {
        let chunk_id = row_id / MINMAX_CHUNK_SIZE;
        let idx_key = self.encode_bloom_key(column, chunk_id);

        let mut filter: BloomFilter = match db.get_ks(cf, idx_key.as_bytes()) {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes)?,
            Ok(None) => BloomFilter::with_num_bits(10_000).expected_items(1000),
            Err(e) => return Err(DbError::InternalError(e.to_string())),
        };

        filter.insert(&value.to_string());

        let filter_bytes = serde_json::to_vec(&filter)?;
        db.put_ks(cf, idx_key.as_bytes(), &filter_bytes)
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        Ok(())
    }

    /// Update MinMax index with a new row
    fn update_minmax_index(
        &self,
        db: &DB,
        cf: &KsCf,
        column: &str,
        value: &Value,
        row_id: u64,
    ) -> DbResult<()> {
        let chunk_id = row_id / MINMAX_CHUNK_SIZE;
        let idx_key = self.encode_minmax_key(column, chunk_id);

        let mut chunk: MinMaxChunk = match db.get_ks(cf, idx_key.as_bytes()) {
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
        db.put_ks(cf, idx_key.as_bytes(), &chunk_bytes)
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
                let sum: f64 = values.iter().filter_map(|v| v.as_f64()).sum();
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

            AggregateOp::Min => values
                .iter()
                .filter_map(|v| v.as_f64())
                .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(json_number)
                .unwrap_or(Value::Null),

            AggregateOp::Max => values
                .iter()
                .filter_map(|v| v.as_f64())
                .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
                .map(json_number)
                .unwrap_or(Value::Null),

            AggregateOp::CountDistinct => {
                let distinct: std::collections::HashSet<String> =
                    values.iter().map(|v| v.to_string()).collect();
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
                        ColumnarIndexType::Sorted | ColumnarIndexType::Hash => {
                            return self.index_lookup_eq(col, val)
                        }
                        ColumnarIndexType::Bloom => {
                            // Bloom filter can give false positives, so it provides candidates.
                            // We still need to verify with actual data.
                            let candidate_ids = self.get_candidate_rows_from_bloom(col, val)?;
                            let values = self.read_column_by_positions(col, &candidate_ids)?;
                            return Ok(candidate_ids
                                .into_iter()
                                .zip(values)
                                .filter(|(_, v)| v == val)
                                .map(|(id, _)| id)
                                .collect());
                        }
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
                    Ok(cand_ids
                        .into_iter()
                        .zip(values)
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
                    Ok(cand_ids
                        .into_iter()
                        .zip(values)
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
                    Ok(cand_ids
                        .into_iter()
                        .zip(values)
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
                    Ok(cand_ids
                        .into_iter()
                        .zip(values)
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
                        ColumnarIndexType::Bitmap
                        | ColumnarIndexType::Hash
                        | ColumnarIndexType::Sorted => {
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
                        ColumnarIndexType::Bloom => {
                            let mut candidates = Vec::new();
                            for val in vals {
                                candidates.extend(self.get_candidate_rows_from_bloom(col, val)?);
                            }
                            candidates.sort();
                            candidates.dedup();

                            // Verify candidates
                            let values = self.read_column_by_positions(col, &candidates)?;
                            return Ok(candidates
                                .into_iter()
                                .zip(values)
                                .filter(|(_, v)| vals.contains(v)) // O(N*M) where M is vals len. vals usually small.
                                .map(|(id, _)| id)
                                .collect());
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
                    let matching_set: std::collections::HashSet<u64> =
                        matching.into_iter().collect();
                    result.retain(|id| matching_set.contains(id));
                }
                Ok(result)
            }

            ColumnFilter::Or(filters) => {
                let mut result_set: std::collections::HashSet<u64> =
                    std::collections::HashSet::new();
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
        assert_eq!(
            ColumnType::infer_from_value(&Value::Bool(true)),
            ColumnType::Bool
        );
        assert_eq!(
            ColumnType::infer_from_value(&serde_json::json!(42)),
            ColumnType::Int64
        );
        assert_eq!(
            ColumnType::infer_from_value(&serde_json::json!(3.5)),
            ColumnType::Float64
        );
        assert_eq!(
            ColumnType::infer_from_value(&serde_json::json!("hello")),
            ColumnType::String
        );
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
        let data =
            b"Hello, world! This is test data for compression. Repeated text helps compression.";

        // Compress with LZ4
        let compressed = compress_prepend_size(data);

        // Verify compression happened (compressed should be smaller or similar for small data)
        assert!(!compressed.is_empty());

        // Decompress
        let decompressed = decompress_size_prepended(&compressed).unwrap();

        // Verify roundtrip
        assert_eq!(data.to_vec(), decompressed);
    }

    /// A collection stored one key per cell, as 2.10.4 and earlier wrote
    /// it, is converted when it is loaded: the same rows in the same
    /// order, in a block, and no cell left.
    #[test]
    fn a_collection_stored_by_cell_is_converted_when_loaded() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = crate::storage::StorageEngine::new(tmp.path()).unwrap();
        engine.create_database("_system".to_string()).unwrap();
        let db = engine.get_database("_system").unwrap();
        db.create_collection("_columnar_old".to_string(), None)
            .unwrap();
        let columns = ["host", "cpu"]
            .iter()
            .map(|name| ColumnDef {
                name: name.to_string(),
                data_type: ColumnType::Json,
                nullable: true,
                indexed: false,
                index_type: None,
            })
            .collect();
        let col = ColumnarCollection::new(
            "old".to_string(),
            "_system",
            db.db_arc(),
            columns,
            CompressionType::Lz4,
        )
        .unwrap();
        let rows: Vec<Value> = (0..300)
            .map(|i| serde_json::json!({"host": format!("h{}", i % 7), "cpu": i as f64 / 8.0}))
            .collect();
        let ids = col.insert_rows(rows.clone()).unwrap();

        // Back to the old layout: the cells beside the rows, and a format
        // the metadata does not name.
        {
            let cf = col.ks.handle(&col.db).unwrap();
            let mut batch = KvBatch::default();
            for (id, row) in ids.iter().zip(&rows) {
                for name in ["host", "cpu"] {
                    let key = format!("{}{}:{}", COL_DATA_PREFIX, name, id);
                    let bytes = serde_json::to_vec(&row[name]).unwrap();
                    batch.put_ks(
                        &cf,
                        key.as_bytes(),
                        col.compress_data(&bytes, &CompressionType::Lz4),
                    );
                }
            }
            let mut meta = col.metadata().unwrap();
            meta.format = 0;
            col.stage_meta(&cf, &mut batch, &meta).unwrap();
            col.db.write(batch).unwrap();
        }
        drop(col);

        let col = ColumnarCollection::load("old".to_string(), "_system", db.db_arc()).unwrap();
        let meta = col.metadata().unwrap();
        assert_eq!(
            (meta.format, meta.tail_rows, meta.row_count),
            (BLOCK_FORMAT, 0, 300)
        );
        assert_eq!(col.read_columns(&["host", "cpu"], None).unwrap(), rows);
        assert_eq!(col.list_row_uuids().unwrap(), ids);
        let cf = col.ks.handle(&col.db).unwrap();
        let keys_under = |prefix: &str| {
            col.db
                .prefix_iterator_ks(&cf, prefix.as_bytes())
                .flatten()
                .take_while(|(key, _)| key.starts_with(prefix.as_bytes()))
                .count()
        };
        assert_eq!(keys_under(COL_DATA_PREFIX), 0);
        assert_eq!(keys_under(COL_ROW_PREFIX), 0);
        assert_eq!(keys_under(COL_BLOCK_PREFIX), 1);
        // New rows go on after the converted ones.
        col.insert_rows(vec![serde_json::json!({"host": "new", "cpu": 1})])
            .unwrap();
        assert_eq!(col.count(), 301);
        assert_eq!(
            col.read_column("host", None).unwrap().last(),
            Some(&serde_json::json!("new"))
        );
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
