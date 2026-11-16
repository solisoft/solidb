//! Columnar Storage for Analytics/Reporting Workloads
//!
//! This module provides column-oriented storage optimized for:
//! - Read-heavy workloads (analytics, reporting)
//! - Aggregations (SUM, AVG, COUNT, MIN, MAX)
//! - Column pruning (only read needed columns)
//! - LZ4 compression for efficient storage

use super::columnar_blocks::{
    ColBlock, ColData, Column, RowSet, BLOCK_ROWS, COL_BLOCK_PREFIX, SEAL_ROWS,
};
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
    /// For each indexed column, how many rows its index holds, counted
    /// from the first in id order. An index names rows by their position
    /// in that order and is written once, when it is created: the rows
    /// after these are read as rows (`filter_positions`), and a write that
    /// moves a position (a delete, a row with an older id) lowers the
    /// count to where it landed. A column with no entry is one indexed
    /// before this was kept: nothing is known of its index, which is not
    /// used.
    #[serde(default)]
    pub index_rows: std::collections::HashMap<String, u64>,
    /// How long a row is kept, in seconds, counted from the instant in its
    /// id (a UUIDv7): rows older than that are removed by
    /// [`ColumnarCollection::prune_expired`], which the TTL worker calls.
    /// `None`: rows are kept until they are deleted.
    #[serde(default)]
    pub retention_secs: Option<u64>,
}

/// The storage format new collections are created in.
const BLOCK_FORMAT: u8 = 2;

/// The decoded blocks of a collection, `None` until they are read from
/// disk, and the database they belong to: the entry keeps that address
/// from being used again by another one.
type BlockStore = (std::sync::Weak<DB>, Arc<Kept>);

/// What is kept of one collection's blocks, and when it was last read.
#[derive(Default)]
struct Kept {
    blocks: parking_lot::RwLock<Option<Vec<Arc<ColBlock>>>>,
    /// [`USES`] at the last read.
    used: std::sync::atomic::AtomicU64,
}

/// Counts the reads of decoded blocks, to tell which collection was read
/// longest ago.
static USES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `SOLIDB_COLUMNAR_BLOCKS_MB`: what the decoded blocks of every columnar
/// collection together may take in memory. By default a quarter of the
/// memory the process may use (its cgroup's limit, else the host's), at
/// least 1 GiB. Past it, the collections read longest ago give their
/// blocks back, and read and decode them again at their next query.
fn decoded_budget() -> usize {
    static BUDGET: once_cell::sync::Lazy<usize> = once_cell::sync::Lazy::new(|| {
        std::env::var("SOLIDB_COLUMNAR_BLOCKS_MB")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .map(|mb| mb << 20)
            .unwrap_or_else(|| {
                let memory = super::memory_limit::detected().map_or(4 << 30, |limit| limit.bytes);
                ((memory / 4) as usize).max(1 << 30)
            })
    });
    *BUDGET
}

/// Reads of a whole collection a few blocks at a time
/// ([`ColumnarCollection::for_each_blocks`]) since the process started: for
/// tests, which must know that way was taken.
pub static WINDOWED_READS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Bytes of decoded blocks kept for every columnar collection of the
/// process, as [`ColBlock::bytes`] counts them.
pub fn decoded_blocks_kept() -> usize {
    BLOCK_STORES
        .iter()
        .map(|entry| {
            entry
                .value()
                .1
                .blocks
                .read()
                .as_ref()
                .map_or(0, |blocks| blocks.iter().map(|block| block.bytes()).sum())
        })
        .sum()
}

/// Hold the decoded blocks kept for all collections to the budget: the
/// collections read longest ago stop keeping theirs, this one included if
/// it alone is past it (the query that asked still has them).
fn keep_within_budget() {
    let budget = decoded_budget();
    let mut held: Vec<(u64, usize, Arc<Kept>)> = BLOCK_STORES
        .iter()
        .filter_map(|entry| {
            let kept = entry.value().1.clone();
            let bytes: usize = kept
                .blocks
                .read()
                .as_ref()?
                .iter()
                .map(|block| block.bytes())
                .sum();
            (bytes > 0).then(|| {
                (
                    kept.used.load(std::sync::atomic::Ordering::Relaxed),
                    bytes,
                    kept,
                )
            })
        })
        .collect();
    let mut total: usize = held.iter().map(|(_, bytes, _)| bytes).sum();
    if total <= budget {
        return;
    }
    held.sort_by_key(|(used, ..)| *used);
    for (_, bytes, kept) in held {
        if total <= budget {
            break;
        }
        *kept.blocks.write() = None;
        total -= bytes;
    }
}
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
    /// The column a comparison is about; `None` for `And` / `Or`.
    pub(crate) fn column(&self) -> Option<&str> {
        match self {
            ColumnFilter::Eq(column, _)
            | ColumnFilter::Ne(column, _)
            | ColumnFilter::Gt(column, _)
            | ColumnFilter::Gte(column, _)
            | ColumnFilter::Lt(column, _)
            | ColumnFilter::Lte(column, _)
            | ColumnFilter::In(column, _) => Some(column),
            ColumnFilter::And(_) | ColumnFilter::Or(_) => None,
        }
    }

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
            index_rows: Default::default(),
            retention_secs: None,
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
        *created.store().blocks.write() = Some(Vec::new());
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
    fn store(&self) -> Arc<Kept> {
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

    /// Whether a read of the whole collection should take its blocks a few
    /// at a time ([`Self::for_each_blocks`]) rather than all at once: they
    /// are not in memory, and by the count of rows they would not fit the
    /// budget of what is kept.
    pub(crate) fn larger_than_kept(&self) -> bool {
        // What a row of a few columns takes decoded, at the least.
        const ROW: usize = 48;
        self.store().blocks.read().is_none() && self.count().saturating_mul(ROW) > decoded_budget()
    }

    /// The blocks in the order of their keys, `at_once` at a time, read
    /// and decoded for `f` and not kept: what a collection larger than the
    /// budget is read with. `only`, when given, names the blocks wanted by
    /// their keys; the others are not decoded.
    pub(crate) fn for_each_blocks(
        &self,
        at_once: usize,
        only: Option<&std::collections::HashSet<String>>,
        mut f: impl FnMut(Vec<Arc<ColBlock>>) -> DbResult<()>,
    ) -> DbResult<()> {
        let compression = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?
            .compression
            .clone();
        let cf = self.ks.handle(&self.db).ok_or_else(|| self.not_found())?;
        if only.is_none() {
            WINDOWED_READS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        let prefix = COL_BLOCK_PREFIX.as_bytes();
        let mut blocks = Vec::with_capacity(at_once.max(1));
        for (key, bytes) in self.db.prefix_iterator_ks(&cf, prefix).flatten() {
            if !key.starts_with(prefix) {
                break;
            }
            if let Some(only) = only {
                let wanted = std::str::from_utf8(&key[prefix.len()..])
                    .is_ok_and(|block| only.contains(block));
                if !wanted {
                    continue;
                }
            }
            let block = self
                .decompress_data(&bytes, &compression)
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
            if blocks.len() >= at_once.max(1) {
                f(std::mem::take(&mut blocks))?;
            }
        }
        if !blocks.is_empty() {
            f(blocks)?;
        }
        Ok(())
    }

    /// The rows of the tail, ascending by id, as a query reads them.
    pub(crate) fn tail_rows(&self) -> DbResult<Vec<(String, serde_json::Map<String, Value>)>> {
        let compression = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?
            .compression
            .clone();
        let cf = self.ks.handle(&self.db).ok_or_else(|| self.not_found())?;
        self.tail(&cf, &compression)
    }

    /// The blocks, in the order of their keys: from memory, or read once.
    fn blocks(&self, cf: &KsCf, compression: &CompressionType) -> DbResult<Vec<Arc<ColBlock>>> {
        let store = self.store();
        store.used.store(
            USES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1,
            std::sync::atomic::Ordering::Relaxed,
        );
        if let Some(blocks) = store.blocks.read().as_ref() {
            return Ok(blocks.clone());
        }
        let mut held = store.blocks.write();
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
        drop(held);
        keep_within_budget();
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

    /// Before a write that puts a row of id `id` in, or takes it out: no
    /// index may answer for a position from that row's on, which the write
    /// moves (`ColumnarCollectionMeta::index_rows`). Rows appended after
    /// every other change nothing. Reads the rows only when an index holds
    /// some.
    fn cap_indexes(
        &self,
        current: &ColumnarCollectionMeta,
        new_meta: &mut ColumnarCollectionMeta,
        id: &str,
    ) -> DbResult<()> {
        if !new_meta.index_rows.values().any(|rows| *rows > 0) {
            return Ok(());
        }
        let rows = self.row_set_with(&current.compression)?;
        // Ids ascend along the positions.
        let (mut low, mut high) = (0, rows.len());
        while low < high {
            let middle = low + (high - low) / 2;
            if rows.uuid(rows.locate(middle)).as_ref() < id {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        for held in new_meta.index_rows.values_mut() {
            *held = (*held).min(low as u64);
        }
        Ok(())
    }

    /// How many rows the index on `column` answers for.
    fn index_rows(&self, column: &str) -> u64 {
        self.meta
            .read()
            .ok()
            .and_then(|meta| meta.index_rows.get(column).copied())
            .unwrap_or(0)
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
        if let Some(blocks) = self.store().blocks.write().as_mut() {
            // (Not twice a block that a load, started after this write
            // reached the disk, already read.)
            blocks.retain(|held| !sealed.iter().any(|new| new.key == held.key));
            blocks.extend(sealed.into_iter().map(Arc::new));
            blocks.sort_by(|a, b| a.key.cmp(&b.key));
        }
        keep_within_budget();
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
        *self.store().blocks.write() = None;
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
        // Every row against the schema, before anything is written: one
        // that does not fit refuses the whole insert.
        let schema: Vec<ColumnDef> = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?
            .columns
            .clone();
        let objects: Vec<serde_json::Map<String, Value>> = rows
            .into_iter()
            .enumerate()
            .map(|(at, row)| conform(&schema, at, row))
            .collect::<DbResult<_>>()?;
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
        if let Some(first) = inserted_ids.first() {
            self.cap_indexes(&meta, &mut new_meta, first)?;
        }
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
        self.cap_indexes(&meta, &mut new_meta, &fresh[0].0)?;
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
        self.cap_indexes(&meta, &mut new_meta, row_uuid)?;
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
        Ok(!self.delete_rows(&[row_uuid.to_string()])?.is_empty())
    }

    /// Delete the rows of these ids, in one batch. Returns the ids that
    /// were there, in the order given.
    ///
    /// A block that loses rows is written again once, whatever their
    /// number.
    pub fn delete_rows(&self, ids: &[String]) -> DbResult<Vec<String>> {
        if ids.is_empty() {
            return Ok(Vec::new());
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

        let rows = self.row_set_with(&meta.compression)?;
        // Where each row is: by block, or in the tail.
        let mut in_blocks: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
        let mut in_tail = 0u64;
        let mut seen = std::collections::HashSet::new();
        let mut deleted = Vec::new();
        let mut batch = KvBatch::default();
        for id in ids {
            if !seen.insert(id.as_str()) {
                continue;
            }
            let Some((at, row)) = rows.find(id) else {
                continue;
            };
            if at < rows.blocks.len() {
                in_blocks.entry(at).or_default().push(row);
            } else {
                batch.delete_ks(&cf, format!("{}{}", COL_ROW_PREFIX, id).as_bytes());
                in_tail += 1;
            }
            deleted.push(id.clone());
        }
        let Some(first) = deleted.iter().min() else {
            return Ok(deleted);
        };

        let mut new_meta = meta.clone();
        self.cap_indexes(&meta, &mut new_meta, first)?;
        new_meta.tail_rows = new_meta.tail_rows.saturating_sub(in_tail);

        // Each block written again without its rows: under its key when a
        // single row goes, sealed anew from the rows left otherwise.
        let names: Vec<String> = meta.columns.iter().map(|c| c.name.clone()).collect();
        let mut rewritten: Vec<(String, Option<ColBlock>)> = Vec::new();
        for (at, gone) in in_blocks {
            let block = &rows.blocks[at];
            let old_key = format!("{}{}", COL_BLOCK_PREFIX, block.key);
            let smaller = if gone.len() >= block.len() {
                None
            } else if let [row] = gone[..] {
                Some(block.without(row))
            } else {
                let gone: std::collections::HashSet<usize> = gone.into_iter().collect();
                let kept: Vec<(String, serde_json::Map<String, Value>)> = (0..block.len())
                    .filter(|row| !gone.contains(row))
                    .map(|row| (block.uuids.text(row), block.row(row)))
                    .collect();
                Some(ColBlock::seal(&names, &kept))
            };
            match &smaller {
                Some(smaller) => {
                    if smaller.key != block.key {
                        batch.delete_ks(&cf, old_key.as_bytes());
                    }
                    self.stage_block(&cf, &mut batch, &meta.compression, smaller)?;
                }
                None => batch.delete_ks(&cf, old_key.as_bytes()),
            }
            rewritten.push((block.key.clone(), smaller));
        }

        new_meta.row_count = new_meta.row_count.saturating_sub(deleted.len() as u64);
        new_meta.last_updated_at = chrono::Utc::now().timestamp();
        self.stage_meta(&cf, &mut batch, &new_meta)?;

        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
        if !rewritten.is_empty() {
            if let Some(blocks) = self.store().blocks.write().as_mut() {
                for (old_key, smaller) in rewritten {
                    blocks.retain(|b| b.key != old_key);
                    if let Some(smaller) = smaller {
                        blocks.push(Arc::new(smaller));
                    }
                }
                blocks.sort_by(|a, b| a.key.cmp(&b.key));
            }
        }

        Ok(deleted)
    }

    /// Merge the blocks that are smaller than they could be: runs of
    /// neighbouring blocks that fit together in one block of
    /// [`BLOCK_ROWS`] rows are sealed again as one. Returns how many blocks
    /// there were and how many there are.
    ///
    /// An insert of 4,096 rows or more is sealed as its own block, so a
    /// loader sending batches of 5,000 leaves thirteen blocks where one
    /// fits, and a delete leaves a block with the rows it kept. The rows
    /// and their order are what they were: positions do not move, so the
    /// indexes answer as before. Each run is merged under the collection's
    /// write lock, which is given back between two runs.
    pub fn compact(&self) -> DbResult<(usize, usize)> {
        let before = self.row_set()?.blocks.len();
        while self.merge_one_run()? {}
        Ok((before, self.row_set()?.blocks.len()))
    }

    /// Merge the first run of blocks that fit in one. `false` when there
    /// is none left.
    fn merge_one_run(&self) -> DbResult<bool> {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let meta = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?
            .clone();
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        let blocks = self.blocks(&cf, &meta.compression)?;

        // The first run of two blocks or more that fit together, each run
        // taking as many neighbours as it can.
        let mut run: Option<std::ops::Range<usize>> = None;
        let mut start = 0;
        while start < blocks.len() {
            let mut end = start;
            let mut rows = 0;
            while end < blocks.len() && rows + blocks[end].len() <= BLOCK_ROWS {
                rows += blocks[end].len();
                end += 1;
            }
            if end - start >= 2 {
                run = Some(start..end);
                break;
            }
            start = end.max(start + 1);
        }
        let Some(run) = run else {
            return Ok(false);
        };

        let names: Vec<String> = meta.columns.iter().map(|c| c.name.clone()).collect();
        let merged_from = &blocks[run];
        let mut rows: Vec<(String, serde_json::Map<String, Value>)> = merged_from
            .iter()
            .flat_map(|block| {
                (0..block.len()).map(move |row| (block.uuids.text(row), block.row(row)))
            })
            .collect();
        // Blocks follow each other by their first id; a row that came by
        // replication may sit in a later block under an older id.
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        let merged = ColBlock::seal(&names, &rows);

        let mut batch = KvBatch::default();
        for block in merged_from {
            batch.delete_ks(&cf, format!("{}{}", COL_BLOCK_PREFIX, block.key).as_bytes());
        }
        self.stage_block(&cf, &mut batch, &meta.compression, &merged)?;
        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let gone: Vec<String> = merged_from.iter().map(|b| b.key.clone()).collect();
        if let Some(held) = self.store().blocks.write().as_mut() {
            held.retain(|b| !gone.contains(&b.key));
            held.push(Arc::new(merged));
            held.sort_by(|a, b| a.key.cmp(&b.key));
        }
        keep_within_budget();
        Ok(true)
    }

    /// Add a column to the schema. The rows already stored hold nothing
    /// for it and read as null, so the column is nullable; nothing is
    /// written but the schema: a block sealed before has no array of that
    /// name, which is how a null column of it reads.
    pub fn add_column(&self, column: ColumnDef) -> DbResult<()> {
        if column.name.is_empty() || column.name.starts_with('_') || column.name.contains(':') {
            return Err(DbError::BadRequest(format!(
                "'{}' is not a column name: it is not empty, does not start with '_' and holds no ':'",
                column.name
            )));
        }
        if !column.nullable {
            return Err(DbError::BadRequest(format!(
                "Column '{}' is added to rows that hold nothing for it: it must be nullable",
                column.name
            )));
        }
        self.change_schema(|meta| {
            if meta.columns.iter().any(|c| c.name == column.name) {
                return Err(DbError::ConflictError(format!(
                    "Column '{}' already exists",
                    column.name
                )));
            }
            meta.columns.push(ColumnDef {
                indexed: false,
                index_type: None,
                ..column
            });
            Ok(None)
        })
    }

    /// Remove a column from the schema and its values from every block and
    /// tail row, which are written again: a column added later under the
    /// same name must not find them. Refused for the last column, and for
    /// a column with an index (drop the index first).
    pub fn drop_column(&self, name: &str) -> DbResult<()> {
        self.change_schema(|meta| {
            let at = Self::column_to_change(meta, name)?;
            if meta.columns.len() == 1 {
                return Err(DbError::BadRequest(format!(
                    "'{}' is the only column: drop the collection instead",
                    name
                )));
            }
            meta.columns.remove(at);
            Ok(Some((name.to_string(), None)))
        })
    }

    /// Rename a column, in the schema and in every block and tail row,
    /// which are written again (a block names its columns). Refused for a
    /// column with an index (drop the index first).
    pub fn rename_column(&self, from: &str, to: &str) -> DbResult<()> {
        if to.is_empty() || to.starts_with('_') || to.contains(':') {
            return Err(DbError::BadRequest(format!(
                "'{}' is not a column name: it is not empty, does not start with '_' and holds no ':'",
                to
            )));
        }
        self.change_schema(|meta| {
            let at = Self::column_to_change(meta, from)?;
            if meta.columns.iter().any(|c| c.name == to) {
                return Err(DbError::ConflictError(format!(
                    "Column '{}' already exists",
                    to
                )));
            }
            meta.columns[at].name = to.to_string();
            Ok(Some((from.to_string(), Some(to.to_string()))))
        })
    }

    /// The position of a column that may be dropped or renamed.
    fn column_to_change(meta: &ColumnarCollectionMeta, name: &str) -> DbResult<usize> {
        let at = meta
            .columns
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| DbError::BadRequest(format!("Column '{}' not found", name)))?;
        if meta.columns[at].indexed {
            return Err(DbError::ConflictError(format!(
                "Column '{}' has an index: drop the index first",
                name
            )));
        }
        Ok(at)
    }

    /// Change the schema under the collection's write lock. `change` edits
    /// the columns and says which stored column, if any, is to be removed
    /// (`None`) or renamed (`Some(new name)`) in the blocks and the tail,
    /// which are then written again in the batch that stores the schema.
    fn change_schema(
        &self,
        change: impl FnOnce(&mut ColumnarCollectionMeta) -> DbResult<Option<(String, Option<String>)>>,
    ) -> DbResult<()> {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        self.refresh_meta(db, &cf, &mut meta);

        let mut new_meta = meta.clone();
        let stored = change(&mut new_meta)?;
        new_meta.last_updated_at = chrono::Utc::now().timestamp();
        let mut batch = KvBatch::default();
        if let Some((column, renamed)) = &stored {
            let prefix = COL_BLOCK_PREFIX.as_bytes();
            for (key, bytes) in db.prefix_iterator_ks(&cf, prefix).flatten() {
                if !key.starts_with(prefix) {
                    break;
                }
                let mut block = self
                    .decompress_data(&bytes, &meta.compression)
                    .ok()
                    .and_then(|plain| ColBlock::decode(&plain))
                    .ok_or_else(|| {
                        DbError::InternalError(format!(
                            "columnar block '{}' of '{}' cannot be read",
                            String::from_utf8_lossy(&key),
                            self.name
                        ))
                    })?;
                let Some(at) = block.names.iter().position(|n| n == column) else {
                    continue;
                };
                match renamed {
                    Some(to) => block.names[at] = to.clone(),
                    None => {
                        block.names.remove(at);
                        block.cols.remove(at);
                    }
                }
                self.stage_block(&cf, &mut batch, &meta.compression, &block)?;
            }
            for (id, mut row) in self.tail(&cf, &meta.compression)? {
                let Some(value) = row.remove(column) else {
                    continue;
                };
                if let Some(to) = renamed {
                    row.insert(to.clone(), value);
                }
                let bytes = rmp_serde::to_vec(&row)
                    .map_err(|e| DbError::InternalError(format!("columnar row: {e}")))?;
                batch.put_ks(
                    &cf,
                    format!("{}{}", COL_ROW_PREFIX, id).as_bytes(),
                    self.compress_data(&bytes, &meta.compression),
                );
            }
        }
        self.stage_meta(&cf, &mut batch, &new_meta)?;
        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
        if stored.is_some() {
            // The decoded blocks hold the column as it was: read again.
            *self.store().blocks.write() = None;
        }
        Ok(())
    }

    /// Set how long rows are kept (`None`: until they are deleted). Nothing
    /// is removed here: the next [`Self::prune_expired`] does it.
    pub fn set_retention(&self, seconds: Option<u64>) -> DbResult<()> {
        if seconds == Some(0) {
            return Err(DbError::BadRequest(
                "A retention is one second or more; leave it out to keep every row".to_string(),
            ));
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
        let mut new_meta = meta.clone();
        new_meta.retention_secs = seconds;
        let mut batch = KvBatch::default();
        self.stage_meta(&cf, &mut batch, &new_meta)?;
        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
        Ok(())
    }

    /// Remove the rows older than the collection's retention. Returns how
    /// many went; 0 at once for a collection that keeps its rows.
    pub fn prune_expired(&self) -> DbResult<u64> {
        let retention = self
            .meta
            .read()
            .map_err(|e| DbError::InternalError(e.to_string()))?
            .retention_secs;
        let Some(seconds) = retention else {
            return Ok(0);
        };
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.prune_before(now.saturating_sub(seconds.saturating_mul(1000)))
    }

    /// Remove the rows whose id was made before `cutoff_ms` (milliseconds
    /// since the epoch, as a UUIDv7 holds it). Returns how many went.
    ///
    /// Rows are in the order of their ids, so the old ones are the first: a
    /// block whose rows are all older goes as one key, with nothing read or
    /// written again, and at most one block, the one the cutoff falls in,
    /// is sealed again from the rows it keeps. A row whose id is not a
    /// UUIDv7 of that time is compared as the text it is.
    pub fn prune_before(&self, cutoff_ms: u64) -> DbResult<u64> {
        // The lowest id of that instant: every id below it is older.
        let cutoff = uuid::Uuid::from_u128((cutoff_ms as u128) << 80).to_string();
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
        let names: Vec<String> = meta.columns.iter().map(|c| c.name.clone()).collect();
        let mut batch = KvBatch::default();
        let mut removed = 0u64;
        let mut first: Option<String> = None;
        let mut rewritten: Vec<(String, Option<ColBlock>)> = Vec::new();
        for block in &rows.blocks {
            if block.key.as_str() >= cutoff.as_str() {
                continue;
            }
            let len = block.len();
            // Rows of a block are in id order: the old ones are a prefix.
            let (mut low, mut high) = (0, len);
            while low < high {
                let mid = (low + high) / 2;
                if block.uuids.text(mid).as_str() < cutoff.as_str() {
                    low = mid + 1;
                } else {
                    high = mid;
                }
            }
            let old = low;
            if old == 0 {
                continue;
            }
            let old_key = format!("{}{}", COL_BLOCK_PREFIX, block.key);
            batch.delete_ks(&cf, old_key.as_bytes());
            let smaller = if old == len {
                None
            } else {
                let kept: Vec<(String, serde_json::Map<String, Value>)> = (old..len)
                    .map(|row| (block.uuids.text(row), block.row(row)))
                    .collect();
                let smaller = ColBlock::seal(&names, &kept);
                self.stage_block(&cf, &mut batch, &meta.compression, &smaller)?;
                Some(smaller)
            };
            removed += old as u64;
            if first.as_ref().is_none_or(|f| block.key < *f) {
                first = Some(block.key.clone());
            }
            rewritten.push((block.key.clone(), smaller));
        }
        let mut from_tail = 0u64;
        for (id, _) in &rows.tail {
            if id.as_str() < cutoff.as_str() {
                batch.delete_ks(&cf, format!("{}{}", COL_ROW_PREFIX, id).as_bytes());
                from_tail += 1;
                if first.as_ref().is_none_or(|f| id < f) {
                    first = Some(id.clone());
                }
            }
        }
        removed += from_tail;
        let Some(first) = first else {
            return Ok(0);
        };

        let mut new_meta = meta.clone();
        // The rows that stay all move down: no index answers for them any
        // more, until it is created again.
        self.cap_indexes(&meta, &mut new_meta, &first)?;
        new_meta.tail_rows = new_meta.tail_rows.saturating_sub(from_tail);
        new_meta.row_count = new_meta.row_count.saturating_sub(removed);
        new_meta.last_updated_at = chrono::Utc::now().timestamp();
        self.stage_meta(&cf, &mut batch, &new_meta)?;
        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
        if !rewritten.is_empty() {
            if let Some(blocks) = self.store().blocks.write().as_mut() {
                for (old_key, smaller) in rewritten {
                    blocks.retain(|b| b.key != old_key);
                    if let Some(smaller) = smaller {
                        blocks.push(Arc::new(smaller));
                    }
                }
                blocks.sort_by(|a, b| a.key.cmp(&b.key));
            }
        }
        Ok(removed)
    }

    /// Delete the rows `filter` keeps. Returns their ids.
    pub fn delete_where(&self, filter: &ColumnFilter) -> DbResult<Vec<String>> {
        let ids = self.filtered_row_uuids(filter)?;
        self.delete_rows(&ids)
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

    /// The first `limit` rows in id order (all of them without a limit),
    /// with these columns. Stops at the limit: nothing is built past it.
    pub fn read_columns_limited(
        &self,
        columns: &[&str],
        limit: Option<usize>,
    ) -> DbResult<Vec<Value>> {
        for column in columns {
            self.check_column(column)?;
        }
        let rows = self.row_set()?;
        Ok(rows
            .positions()
            .take(limit.unwrap_or(usize::MAX))
            .map(|at| {
                Value::Object(
                    columns
                        .iter()
                        .map(|column| (column.to_string(), rows.value(at, column)))
                        .collect(),
                )
            })
            .collect())
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
        let rows = self.row_set()?;
        let positions = self.filter_positions(&rows, filter, 0..rows.len() as u64)?;
        Ok(positions
            .into_iter()
            .map(|position| rows.uuid(rows.locate(position as usize)).to_string())
            .collect())
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

    /// The rows `filter` keeps, with these columns, in the order of their
    /// ids. The rows are read once, for the filter and for what is
    /// returned.
    pub fn scan_filtered(&self, filter: &ColumnFilter, columns: &[&str]) -> DbResult<Vec<Value>> {
        for column in columns {
            self.check_column(column)?;
        }
        let rows = self.row_set()?;
        let positions = self.filter_positions(&rows, filter, 0..rows.len() as u64)?;
        Ok(positions
            .into_iter()
            .map(|position| {
                let at = rows.locate(position as usize);
                Value::Object(
                    columns
                        .iter()
                        .map(|column| (column.to_string(), rows.value(at, column)))
                        .collect(),
                )
            })
            .collect())
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
        // The index entries went with the rows.
        for held in new_meta.index_rows.values_mut() {
            *held = 0;
        }
        new_meta.last_updated_at = chrono::Utc::now().timestamp();
        self.stage_meta(&cf, &mut batch, &new_meta)?;

        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to truncate: {}", e)))?;
        *meta = new_meta;
        *self.store().blocks.write() = Some(Vec::new());

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
        // The entries are built in memory and written once, with the
        // schema, in one batch. They used to be written a row at a time,
        // each row reading the list of its value back, adding itself and
        // writing the list again: time quadratic in the rows (0.8 s for
        // 20,000 rows, 11.7 s for 80,000), and an index half written when
        // one of those writes failed.
        let mut batch = KvBatch::default();
        match index_type {
            ColumnarIndexType::Bitmap => {
                let mut bitmaps: HashMap<String, Vec<u8>> = HashMap::new();
                for (row_id, value) in values.iter().enumerate() {
                    let bitmap = bitmaps
                        .entry(self.encode_bitmap_key(column, value))
                        .or_default();
                    let byte = row_id / 8;
                    if byte >= bitmap.len() {
                        bitmap.resize(byte + 1, 0);
                    }
                    bitmap[byte] |= 1 << (row_id % 8);
                }
                for (key, bitmap) in bitmaps {
                    batch.put_ks(
                        &cf,
                        key.as_bytes(),
                        self.compress_data(&bitmap, &CompressionType::Lz4),
                    );
                }
            }
            ColumnarIndexType::MinMax => {
                for (chunk_id, chunk) in values.chunks(MINMAX_CHUNK_SIZE as usize).enumerate() {
                    let Some(first) = chunk.first() else {
                        continue;
                    };
                    let mut summary = MinMaxChunk {
                        min: first.clone(),
                        max: first.clone(),
                        count: 0,
                    };
                    for value in chunk {
                        if compare_values(value, &summary.min) == std::cmp::Ordering::Less {
                            summary.min = value.clone();
                        }
                        if compare_values(value, &summary.max) == std::cmp::Ordering::Greater {
                            summary.max = value.clone();
                        }
                        summary.count += 1;
                    }
                    batch.put_ks(
                        &cf,
                        self.encode_minmax_key(column, chunk_id as u64).as_bytes(),
                        serde_json::to_vec(&summary)?,
                    );
                }
            }
            ColumnarIndexType::Bloom => {
                for (chunk_id, chunk) in values.chunks(MINMAX_CHUNK_SIZE as usize).enumerate() {
                    let mut filter = BloomFilter::with_num_bits(10_000).expected_items(1000);
                    for value in chunk {
                        filter.insert(&value.to_string());
                    }
                    batch.put_ks(
                        &cf,
                        self.encode_bloom_key(column, chunk_id as u64).as_bytes(),
                        serde_json::to_vec(&filter)?,
                    );
                }
            }
            _ => {
                let mut lists: HashMap<String, Vec<u64>> = HashMap::new();
                for (row_id, value) in values.iter().enumerate() {
                    lists
                        .entry(self.encode_index_key(column, value))
                        .or_default()
                        .push(row_id as u64);
                }
                for (key, rows) in lists {
                    batch.put_ks(&cf, key.as_bytes(), serde_json::to_vec(&rows)?);
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
        batch.put_ks(&cf, idx_meta_key.as_bytes(), serde_json::to_vec(&idx_meta)?);

        // The schema, with the entries.
        let mut new_meta = meta.clone();
        new_meta.columns[col_idx].indexed = true;
        new_meta.columns[col_idx].index_type = Some(index_type.clone());
        new_meta
            .index_rows
            .insert(column.to_string(), values.len() as u64);
        let meta_key = format!("{}{}", COL_META_PREFIX, self.name);
        batch.put_ks(&cf, meta_key.as_bytes(), serde_json::to_vec(&new_meta)?);
        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;

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
        meta.columns[col_idx].index_type = None;
        meta.index_rows.remove(column);

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

    /// The positions, among `row_range`, of the rows `filter` keeps,
    /// ascending.
    ///
    /// A comparison is answered from the arrays of the blocks
    /// (`scan_leaf`), which skips a block whose smallest and largest value
    /// rule it out, or from an index on the column where one answers it
    /// (`index_for`). An index answers only for the rows it holds
    /// (`ColumnarCollectionMeta::index_rows`): those after them are
    /// compared as rows. It used to answer for all of them, and a row
    /// inserted after an index was created was never found (card #901).
    ///
    /// Each comparison used to build its whole column as JSON values, and
    /// the rows kept were then found again by id, one block after another
    /// (card #893).
    fn filter_positions(
        &self,
        rows: &RowSet,
        filter: &ColumnFilter,
        row_range: std::ops::Range<u64>,
    ) -> DbResult<Vec<u64>> {
        let row_range = row_range.start..row_range.end.min(rows.len() as u64);
        let span = row_range.end.saturating_sub(row_range.start) as usize;
        let column = match filter {
            ColumnFilter::And(filters) => {
                // Each side over the whole range: it used to be asked only
                // for as many positions as the sides before it had kept,
                // and a row past them was lost.
                //
                // The comparisons the arrays answer go block by block: a
                // block one of them rules out is passed over for all of
                // them, which is what a range bounded on both sides needs
                // (`t >= a AND t < b` reads the blocks that hold the range,
                // not every row above `a`). The others, an index's or a
                // nested filter, are asked once.
                let by_block = |side: &&ColumnFilter| {
                    side.column().is_some() && self.index_answering(side).is_none()
                };
                let (scanned, whole): (Vec<&ColumnFilter>, Vec<&ColumnFilter>) =
                    filters.iter().partition(by_block);
                let mut kept: Option<Vec<u64>> = None;
                if !scanned.is_empty() {
                    let mut found = Vec::new();
                    for (block, part) in parts(rows, &row_range) {
                        let ruled_out = block.is_some_and(|block| {
                            scanned.iter().any(|side| {
                                side.column()
                                    .and_then(|column| block.column(column))
                                    .is_some_and(|values| block_rules_out(values, side))
                            })
                        });
                        if ruled_out {
                            continue;
                        }
                        let mut here: Option<Vec<u64>> = None;
                        for side in &scanned {
                            let positions = self.filter_positions(rows, side, part.clone())?;
                            let both = match here {
                                None => positions,
                                Some(here) => in_both(here, &positions),
                            };
                            let none = both.is_empty();
                            here = Some(both);
                            if none {
                                break;
                            }
                        }
                        found.extend(here.unwrap_or_default());
                    }
                    kept = Some(found);
                }
                for side in whole {
                    let positions = self.filter_positions(rows, side, row_range.clone())?;
                    kept = Some(match kept {
                        None => positions,
                        Some(kept) => in_both(kept, &positions),
                    });
                }
                return Ok(kept.unwrap_or_else(|| row_range.collect()));
            }
            ColumnFilter::Or(filters) => {
                let mut held = vec![false; span];
                for side in filters {
                    for position in self.filter_positions(rows, side, row_range.clone())? {
                        held[(position - row_range.start) as usize] = true;
                    }
                }
                return Ok(row_range
                    .clone()
                    .filter(|p| held[(p - row_range.start) as usize])
                    .collect());
            }
            ColumnFilter::Eq(column, _)
            | ColumnFilter::Ne(column, _)
            | ColumnFilter::Gt(column, _)
            | ColumnFilter::Gte(column, _)
            | ColumnFilter::Lt(column, _)
            | ColumnFilter::Lte(column, _)
            | ColumnFilter::In(column, _) => column,
        };
        self.check_column(column)?;
        let Some(index) = self.index_for(filter, column) else {
            return Ok(scan_leaf(rows, filter, column, row_range));
        };
        let held = self.index_rows(column).min(row_range.end);
        if held <= row_range.start {
            return Ok(scan_leaf(rows, filter, column, row_range));
        }
        let mut positions = self.index_positions(rows, filter, column, index)?;
        positions.retain(|position| (row_range.start..held).contains(position));
        if !positions.windows(2).all(|pair| pair[0] < pair[1]) {
            positions.sort_unstable();
            positions.dedup();
        }
        if held < row_range.end {
            positions.extend(scan_leaf(rows, filter, column, held..row_range.end));
        }
        Ok(positions)
    }

    /// The kind of the index that answers the comparison `filter` for the
    /// rows it holds, if one does: what EXPLAIN reports.
    pub(crate) fn index_answering(&self, filter: &ColumnFilter) -> Option<ColumnarIndexType> {
        let column = filter.column()?;
        self.index_for(filter, column)
            .filter(|_| self.index_rows(column) > 0)
    }

    /// The index on `column` that answers the comparison `filter`, if one
    /// does.
    ///
    /// An equality (or `IN`) is one entry of a hash, sorted or bitmap
    /// index, or the chunks a bloom index does not rule out. A range
    /// against a number is never an index's: the arrays answer it in one
    /// pass and skip the blocks out of range, where a sorted index reads
    /// one entry per distinct value (0.5 s against 0.2 s for a hundred
    /// rows out of a million, card #891), and a minmax index only knows
    /// what each block already does.
    fn index_for(&self, filter: &ColumnFilter, column: &str) -> Option<ColumnarIndexType> {
        let index = self.get_index_type(column)?;
        let answers = match filter {
            ColumnFilter::Eq(..) | ColumnFilter::In(..) => index != ColumnarIndexType::MinMax,
            ColumnFilter::Gt(_, bound)
            | ColumnFilter::Gte(_, bound)
            | ColumnFilter::Lt(_, bound)
            | ColumnFilter::Lte(_, bound) => {
                !bound.is_number() && index == ColumnarIndexType::Sorted
            }
            ColumnFilter::Ne(..) | ColumnFilter::And(_) | ColumnFilter::Or(_) => false,
        };
        answers.then_some(index)
    }

    /// The positions `index` gives for the comparison `filter`, among the
    /// rows it holds.
    fn index_positions(
        &self,
        rows: &RowSet,
        filter: &ColumnFilter,
        column: &str,
        index: ColumnarIndexType,
    ) -> DbResult<Vec<u64>> {
        let lookup = |value: &Value| match index {
            ColumnarIndexType::Bitmap => self.index_lookup_bitmap(column, value),
            // May name a chunk that does not hold the value: checked below.
            ColumnarIndexType::Bloom => self.get_candidate_rows_from_bloom(column, value),
            _ => self.index_lookup_eq(column, value),
        };
        let mut positions = match filter {
            ColumnFilter::Eq(_, value) => lookup(value)?,
            ColumnFilter::In(_, values) => {
                let mut found = Vec::new();
                for value in values {
                    found.extend(lookup(value)?);
                }
                found.sort_unstable();
                found.dedup();
                found
            }
            _ => return self.index_range_scan(column, filter),
        };
        if index == ColumnarIndexType::Bloom {
            positions.retain(|position| {
                (*position as usize) < rows.len()
                    && leaf_keeps(filter, &rows.value(rows.locate(*position as usize), column))
            });
        }
        Ok(positions)
    }
}

/// Whether the comparison `filter` keeps a row holding `value`.
fn leaf_keeps(filter: &ColumnFilter, value: &Value) -> bool {
    use std::cmp::Ordering::{Equal, Greater, Less};
    match filter {
        ColumnFilter::Eq(_, wanted) => value == wanted,
        ColumnFilter::Ne(_, wanted) => value != wanted,
        ColumnFilter::In(_, wanted) => wanted.contains(value),
        ColumnFilter::Gt(_, bound) => against(value, bound, f64::NEG_INFINITY) == Some(Greater),
        ColumnFilter::Gte(_, bound) => {
            matches!(
                against(value, bound, f64::NEG_INFINITY),
                Some(Greater | Equal)
            )
        }
        ColumnFilter::Lt(_, bound) => against(value, bound, f64::INFINITY) == Some(Less),
        ColumnFilter::Lte(_, bound) => {
            matches!(against(value, bound, f64::INFINITY), Some(Less | Equal))
        }
        ColumnFilter::And(_) | ColumnFilter::Or(_) => false,
    }
}

/// A number against the bound of a range; `None` for anything but a number,
/// which no range keeps. Two integers are compared as integers (as floats,
/// those past 2^53 are equal to their neighbours). A bound that is not a
/// number counts as `otherwise`.
fn against(value: &Value, bound: &Value, otherwise: f64) -> Option<std::cmp::Ordering> {
    let Value::Number(number) = value else {
        return None;
    };
    if let (Some(a), Some(b)) = (number.as_i64(), bound.as_i64()) {
        return Some(a.cmp(&b));
    }
    number
        .as_f64()?
        .partial_cmp(&bound.as_f64().unwrap_or(otherwise))
}

/// The positions of `kept` that `other` holds too; both ascending.
fn in_both(mut kept: Vec<u64>, other: &[u64]) -> Vec<u64> {
    let mut at = 0;
    kept.retain(|position| {
        while at < other.len() && other[at] < *position {
            at += 1;
        }
        at < other.len() && other[at] == *position
    });
    kept
}

/// `range` cut where the rows change block: each block that holds some of
/// it, with its share, then the tail's (`None`). The whole range as one
/// part when the rows are not the blocks one after another.
fn parts<'r>(
    rows: &'r RowSet,
    range: &std::ops::Range<u64>,
) -> Vec<(Option<&'r ColBlock>, std::ops::Range<u64>)> {
    if !rows.in_order() {
        return vec![(None, range.clone())];
    }
    let mut parts = Vec::new();
    for at in 0..=rows.blocks.len() {
        let block = rows.blocks.get(at).map(|block| block.as_ref());
        let start = rows.start(at) as u64;
        let rows_held = block.map_or(rows.tail.len(), |block| block.len()) as u64;
        let from = range.start.max(start);
        let to = range.end.min(start + rows_held);
        if from < to {
            parts.push((block, from..to));
        }
    }
    parts
}

/// Whether the smallest and largest value of a block's column say that
/// the comparison `filter` keeps none of its rows. Only a column of
/// integers knows them; `false` is always a safe answer.
fn block_rules_out(column: &Column, filter: &ColumnFilter) -> bool {
    let Some((low, high)) = column.int_range() else {
        return false;
    };
    let keeps = |value: i64| leaf_keeps(filter, &Value::from(value));
    match filter {
        // What keeps neither end keeps nothing between them.
        ColumnFilter::Gt(_, bound)
        | ColumnFilter::Gte(_, bound)
        | ColumnFilter::Lt(_, bound)
        | ColumnFilter::Lte(_, bound) => bound.as_i64().is_some() && !keeps(low) && !keeps(high),
        ColumnFilter::Eq(_, wanted) => wanted.as_i64().is_some_and(|w| w < low || high < w),
        _ => false,
    }
}

/// The positions, among `range`, of the rows the comparison `filter` on
/// `column` keeps, ascending, read from the arrays of the blocks and from
/// the tail.
fn scan_leaf(
    rows: &RowSet,
    filter: &ColumnFilter,
    column: &str,
    range: std::ops::Range<u64>,
) -> Vec<u64> {
    let range = range.start..range.end.min(rows.len() as u64);
    let mut kept = Vec::new();
    if !rows.in_order() {
        // A row with an older id sits among the blocks (replication):
        // position by position.
        kept.extend(range.filter(|position| {
            leaf_keeps(filter, &rows.value(rows.locate(*position as usize), column))
        }));
        return kept;
    }
    for (b, block) in rows.blocks.iter().enumerate() {
        let start = rows.start(b) as u64;
        let from = range.start.max(start);
        let to = range.end.min(start + block.len() as u64);
        if from >= to {
            continue;
        }
        match block.column(column) {
            Some(values) => {
                let inside = (from - start) as usize..(to - start) as usize;
                scan_block(values, filter, inside, start, &mut kept)
            }
            // Sealed before the column was added: null for every row.
            None => {
                if leaf_keeps(filter, &Value::Null) {
                    kept.extend(from..to)
                }
            }
        }
    }
    let start = rows.start(rows.blocks.len()) as u64;
    for (at, (_, row)) in rows.tail.iter().enumerate() {
        let position = start + at as u64;
        if range.contains(&position) && leaf_keeps(filter, row.get(column).unwrap_or(&Value::Null))
        {
            kept.push(position);
        }
    }
    kept
}

/// The rows of one block, among `inside`, that `filter` keeps, pushed as
/// positions (`base` is the block's first).
///
/// Integers, floats and strings are compared in their arrays, and a block
/// of integers is passed over, or taken whole, when its smallest and
/// largest value decide for every row. Anything else is compared value by
/// value, as `leaf_keeps` does. Both must keep the same rows.
fn scan_block(
    column: &Column,
    filter: &ColumnFilter,
    inside: std::ops::Range<usize>,
    base: u64,
    kept: &mut Vec<u64>,
) {
    use std::cmp::Ordering::{self, Greater, Less};
    /// What a range keeps of the order between a value and its bound.
    type Keeps = fn(Ordering) -> bool;
    let null = |row: usize| column.nulls.as_ref().is_some_and(|nulls| nulls[row]);
    // What a range keeps of the order between a value and its bound.
    let range: Option<(&Value, Keeps)> = match filter {
        ColumnFilter::Gt(_, bound) => Some((bound, |o| o == Greater)),
        ColumnFilter::Gte(_, bound) => Some((bound, |o| o != Less)),
        ColumnFilter::Lt(_, bound) => Some((bound, |o| o == Less)),
        ColumnFilter::Lte(_, bound) => Some((bound, |o| o != Greater)),
        _ => None,
    };
    // The bounds of a block of integers hold a placeholder for each null,
    // which only widens them: a block is never passed over wrongly, and
    // one with nulls is never taken whole.
    let mut ints = |values: &[i64], keeps: &dyn Fn(i64) -> bool| {
        if let Some((low, high)) = column.int_range() {
            match (keeps(low), keeps(high)) {
                (false, false) => return,
                (true, true) if column.nulls.is_none() => {
                    kept.extend(inside.clone().map(|row| base + row as u64));
                    return;
                }
                _ => {}
            }
        }
        kept.extend(
            inside
                .clone()
                .filter(|&row| !null(row) && keeps(values[row]))
                .map(|row| base + row as u64),
        );
    };
    match (&column.data, filter, range) {
        (_, ColumnFilter::Eq(_, Value::Null), _) => {
            if let Some(nulls) = &column.nulls {
                kept.extend(
                    inside
                        .filter(|&row| nulls[row])
                        .map(|row| base + row as u64),
                );
            }
        }
        // What keeps a value of a range, or the one value of an equality,
        // keeps every value between two it keeps: the bounds decide.
        (ColData::Int(values), _, Some((bound, keeps))) if bound.as_i64().is_some() => {
            let bound = bound.as_i64().unwrap_or(0);
            ints(values, &|value| keeps(value.cmp(&bound)));
        }
        (ColData::Int(values), ColumnFilter::Eq(_, wanted), _) if wanted.as_i64().is_some() => {
            let wanted = wanted.as_i64().unwrap_or(0);
            if column
                .int_range()
                .is_some_and(|(low, high)| wanted < low || high < wanted)
            {
                return;
            }
            kept.extend(
                inside
                    .filter(|&row| !null(row) && values[row] == wanted)
                    .map(|row| base + row as u64),
            );
        }
        (ColData::Float(values), _, Some((bound, keeps))) if bound.is_number() => {
            let bound = bound.as_f64().unwrap_or(0.0);
            kept.extend(
                inside
                    .filter(|&row| !null(row) && values[row].partial_cmp(&bound).is_some_and(keeps))
                    .map(|row| base + row as u64),
            );
        }
        (ColData::Str { dict, .. }, ColumnFilter::Eq(_, Value::String(wanted)), _) => {
            let Ok(code) = dict.binary_search_by(|held| held.as_str().cmp(wanted)) else {
                return;
            };
            // The rows of that string, ascending; a null row holds a
            // placeholder code.
            let rows = column.rows_of(code as u32);
            let from = rows.partition_point(|row| (*row as usize) < inside.start);
            kept.extend(
                rows[from..]
                    .iter()
                    .map(|row| *row as usize)
                    .take_while(|row| *row < inside.end)
                    .filter(|&row| !null(row))
                    .map(|row| base + row as u64),
            );
        }
        _ => kept.extend(
            inside
                .filter(|&row| leaf_keeps(filter, &column.value(row)))
                .map(|row| base + row as u64),
        ),
    }
}

/// Helper to create JSON number from f64
/// A row as the schema takes it, or why it does not fit (`at` is its place
/// in the insert, from 0).
///
/// Rows used to be stored whatever they held: a string in an `int64`
/// column, a value missing from a column that is not nullable, a column
/// the schema does not have, and a row that was not an object was passed
/// over without a word. A column holding values of several kinds is also
/// stored as JSON text, which no aggregation reads as an array.
///
/// Only for rows a client inserts: a replicated row was accepted by its
/// origin and is taken as it is.
fn conform(
    schema: &[ColumnDef],
    at: usize,
    row: Value,
) -> DbResult<serde_json::Map<String, Value>> {
    let refuse = |why: String| DbError::BadRequest(format!("row {at}: {why}"));
    let kind = |value: &Value| match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(n) if n.is_f64() => "a float",
        Value::Number(_) => "an integer",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    };
    let Value::Object(mut row) = row else {
        return Err(refuse(format!("must be an object, got {}", kind(&row))));
    };
    if let Some(unknown) = row
        .keys()
        .find(|key| !schema.iter().any(|column| column.name == **key))
    {
        return Err(refuse(format!("unknown column '{unknown}'")));
    }
    for column in schema {
        let name = &column.name;
        let Some(value) = row.get_mut(name).filter(|value| !value.is_null()) else {
            if column.nullable {
                continue;
            }
            return Err(refuse(format!(
                "column '{name}' is not nullable and has no value"
            )));
        };
        let (fits, expected) = match column.data_type {
            ColumnType::Int64 => (value.is_i64(), "int64"),
            ColumnType::Float64 => {
                // An integer is a float here: `12` beside `12.5` would
                // otherwise make a column of two kinds.
                let float = value.as_f64().and_then(serde_json::Number::from_f64);
                if let (true, Some(float)) = (value.is_number(), float) {
                    *value = Value::Number(float);
                }
                (value.is_f64(), "float64")
            }
            ColumnType::String => (value.is_string(), "string"),
            ColumnType::Bool => (value.is_boolean(), "bool"),
            // ISO 8601 text, or a number of (milli)seconds.
            ColumnType::Timestamp => (
                value.is_string() || value.is_i64(),
                "timestamp (a string or an integer)",
            ),
            ColumnType::Json => (true, "json"),
        };
        if !fits {
            return Err(refuse(format!(
                "column '{name}' is {expected}, got {}",
                kind(value)
            )));
        }
    }
    Ok(row)
}

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
