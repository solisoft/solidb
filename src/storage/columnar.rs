//! Columnar Storage for Analytics/Reporting Workloads
//!
//! This module provides column-oriented storage optimized for:
//! - Read-heavy workloads (analytics, reporting)
//! - Aggregations (SUM, AVG, COUNT, MIN, MAX)
//! - Column pruning (only read needed columns)
//! - LZ4 compression for efficient storage

use super::columnar_blocks::{
    Bounds, ColBlock, ColData, ColStats, Column, RowSet, BLOCK_ROWS, COL_BLOCK_PREFIX,
    DECODED_MORE, SEAL_ROWS,
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
                                                    // What a `minmax` index stored, through 2.11: nothing writes it any more,
                                                    // and whatever is left under it is deleted (`retire_minmax`, drop, truncate).
const COL_IDX_MINMAX_PREFIX: &str = "col_idx_mm:";
const COL_IDX_BLOOM_PREFIX: &str = "col_idx_blo:"; // col_idx_blo:{column}:{chunk_id} -> [bloom_filter]

/// The rows one bloom filter of a bloom index answers for.
const BLOOM_CHUNK_ROWS: u64 = 1000;

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
    /// For each indexed column, how many rows were removed from the front
    /// of the collection since its index was written: the index names a
    /// row by the position it had then, which is its position now plus
    /// this. A delete that takes exactly the first rows (a retention, a
    /// delete of everything before a date) leaves every other row behind
    /// the same neighbours, so the index keeps answering for them.
    #[serde(default)]
    pub index_base: std::collections::HashMap<String, u64>,
    /// The indexes being built in the background, by column: written when
    /// a build is accepted and removed by the write that stores the index,
    /// so a build a restart interrupted is started again
    /// ([`ColumnarCollection::resume_index_builds`]).
    #[serde(default)]
    pub index_builds: std::collections::HashMap<String, PendingBuild>,
    /// How long a row is kept, in seconds, counted from the instant in its
    /// id (a UUIDv7): rows older than that are removed by
    /// [`ColumnarCollection::prune_expired`], which the TTL worker calls.
    /// `None`: rows are kept until they are deleted.
    #[serde(default)]
    pub retention_secs: Option<u64>,
}

/// An index build that was accepted and has not finished.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingBuild {
    pub id: String,
    pub index_type: ColumnarIndexType,
    pub started_at: i64,
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
    /// Counts the writes that moved a row's position (a delete, a row
    /// with an older id, a truncate): an index build that read the rows
    /// before one of them starts again.
    moved: std::sync::atomic::AtomicU64,
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

/// Blocks a `COLLECT` folded from the arrays had to read as rows instead
/// (a column of several kinds, a comparison with another kind of value)
/// since the process started: for tests, which must know the arrays
/// answered.
pub static BLOCKS_FOLDED_AS_ROWS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

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

/// A block read from disk decodes its ids and its columns as they are
/// asked for, after the budget was looked at: once a read is over, and
/// before the next one, what it decoded is counted.
fn budget_after_read() {
    if DECODED_MORE.swap(false, std::sync::atomic::Ordering::Relaxed) {
        keep_within_budget();
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
    Bloom,  // Probabilistic existence check per chunk
}

/// Metadata for a columnar index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnarIndexMeta {
    pub column: String,
    pub index_type: ColumnarIndexType,
    pub created_at: i64,
    /// What its entries take, keys and values, as they were written.
    #[serde(default)]
    pub size_bytes: u64,
}

/// An index being built in the background, or one that was since this
/// process started.
pub struct IndexBuild {
    pub id: String,
    pub column: String,
    pub index_type: ColumnarIndexType,
    pub started_at: i64,
    rows_done: std::sync::atomic::AtomicU64,
    rows_total: std::sync::atomic::AtomicU64,
    state: parking_lot::Mutex<BuildState>,
    /// Set when the database closes: the build stops, and is started
    /// again by the next process.
    cancel: std::sync::atomic::AtomicBool,
}

/// Where a background build is.
#[derive(Debug, Clone, PartialEq)]
pub enum BuildState {
    Building,
    Done,
    Failed(String),
}

impl IndexBuild {
    fn new(id: String, column: &str, index_type: ColumnarIndexType, started_at: i64) -> Self {
        IndexBuild {
            id,
            column: column.to_string(),
            index_type,
            started_at,
            rows_done: Default::default(),
            rows_total: Default::default(),
            state: parking_lot::Mutex::new(BuildState::Building),
            cancel: Default::default(),
        }
    }

    pub fn state(&self) -> BuildState {
        self.state.lock().clone()
    }

    /// Rows read so far, and how many there are to read.
    pub fn progress(&self) -> (u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (self.rows_done.load(Relaxed), self.rows_total.load(Relaxed))
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// The background builds of every collection, by database, collection and
/// column, with the database they belong to (as [`BLOCK_STORES`]).
type BuildEntry = (std::sync::Weak<DB>, Arc<IndexBuild>);
static INDEX_BUILDS: once_cell::sync::Lazy<dashmap::DashMap<(usize, String, String), BuildEntry>> =
    once_cell::sync::Lazy::new(dashmap::DashMap::new);

/// The threads building them, by database: joined before it closes.
static INDEX_THREADS: once_cell::sync::Lazy<
    dashmap::DashMap<usize, Vec<std::thread::JoinHandle<()>>>,
> = once_cell::sync::Lazy::new(dashmap::DashMap::new);

/// Stop the index builds of `db` and wait for their threads: called when
/// the last handle on the engine goes, before the database is closed, as
/// the sealing thread is. A build stopped here left its mark in the
/// collection's metadata and is started again by the next process.
pub(crate) fn stop_index_builds(db: &Arc<DB>) {
    let address = Arc::as_ptr(db) as usize;
    for entry in INDEX_BUILDS.iter() {
        if entry.key().0 == address {
            entry
                .value()
                .1
                .cancel
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    if let Some((_, threads)) = INDEX_THREADS.remove(&address) {
        for thread in threads {
            let _ = thread.join();
        }
    }
    INDEX_BUILDS.retain(|key, _| key.0 != address);
}

/// The positions of an index entry as they are stored: a marker, their
/// count, then each one as its distance from the one before, all as
/// LEB128 integers. They used to be a JSON array (`[0,500,1000,…]`), about
/// eight bytes a row: an index took four times the rows it was built on.
const POSITIONS_PACKED: u8 = 1;

fn push_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn encode_positions(positions: &[u64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + positions.len() * 2);
    out.push(POSITIONS_PACKED);
    push_varint(&mut out, positions.len() as u64);
    let mut last = 0;
    for position in positions {
        push_varint(&mut out, position - last);
        last = *position;
    }
    out
}

/// The positions of an entry, ascending: packed, or the JSON array an
/// index written through 2.12.0 holds.
fn decode_positions(bytes: &[u8]) -> DbResult<Vec<u64>> {
    if bytes.first() != Some(&POSITIONS_PACKED) {
        return Ok(serde_json::from_slice(bytes)?);
    }
    let corrupt = || DbError::InternalError("a columnar index entry cannot be read".to_string());
    let mut at = 1;
    let mut next = || -> DbResult<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *bytes.get(at).ok_or_else(corrupt)?;
            at += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Ok(value);
            }
        }
        Err(corrupt())
    };
    let count = next()?;
    let mut positions = Vec::with_capacity((count as usize).min(bytes.len()));
    let mut last = 0u64;
    for _ in 0..count {
        last = last.checked_add(next()?).ok_or_else(corrupt)?;
        positions.push(last);
    }
    Ok(positions)
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
            index_base: Default::default(),
            index_builds: Default::default(),
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

        let meta_bytes = retire_minmax(&db, &ks, &cf_name, &name, meta_bytes)?;
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
        loaded.resume_index_builds();
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
                .and_then(ColBlock::decode)
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
        budget_after_read();
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
                .and_then(ColBlock::decode)
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
        Ok(
            RowSet::new(self.blocks(&cf, compression)?, self.tail(&cf, compression)?)
                .then(budget_after_read),
        )
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

    /// Before a write that removes exactly the first `removed` rows: every
    /// row left keeps its neighbours and moves down by as many, so an
    /// index goes on answering for the rows it held, from that base
    /// (`ColumnarCollectionMeta::index_base`). An index used to be capped
    /// at the first row removed, here the first of all: after a retention
    /// or a delete of the oldest rows no index answered for anything.
    fn shift_indexes(new_meta: &mut ColumnarCollectionMeta, removed: u64) {
        for (column, held) in new_meta.index_rows.iter_mut() {
            if *held <= removed {
                *held = 0;
                new_meta.index_base.remove(column);
            } else {
                *held -= removed;
                *new_meta.index_base.entry(column.clone()).or_default() += removed;
            }
        }
    }

    /// A write moved a row's position: a build that read the rows before
    /// it starts again.
    fn note_moved(&self) {
        self.store()
            .moved
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    /// How many rows were removed from the front since the index on
    /// `column` was written.
    fn index_base(&self, column: &str) -> u64 {
        self.meta
            .read()
            .ok()
            .and_then(|meta| meta.index_base.get(column).copied())
            .unwrap_or(0)
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
            .encode(matches!(compression, CompressionType::Lz4))
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
        self.note_moved();
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
        self.note_moved();
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
        self.delete_found(|rows| {
            let mut seen = std::collections::HashSet::new();
            Ok(ids
                .iter()
                .filter(|id| seen.insert(id.as_str()))
                .filter_map(|id| rows.find(id))
                .collect())
        })
    }

    /// Delete the rows `filter` keeps. Returns their ids.
    ///
    /// The rows are found and removed under the lock writers share, by
    /// where they are: their ids used to be listed first, and each one
    /// then looked for again among the blocks (9.5 s for 1.8 million
    /// rows, most of it looking).
    pub fn delete_where(&self, filter: &ColumnFilter) -> DbResult<Vec<String>> {
        self.delete_found(|rows| {
            Ok(self
                .filter_positions(rows, filter, 0..rows.len() as u64)?
                .into_iter()
                .map(|position| rows.locate(position as usize))
                .collect())
        })
    }

    /// Delete the rows `find` names, each by its block and its row there
    /// (the tail is the block after the last), without one twice. Returns
    /// their ids in that order.
    fn delete_found(
        &self,
        find: impl FnOnce(&RowSet) -> DbResult<Vec<(usize, usize)>>,
    ) -> DbResult<Vec<String>> {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        {
            let mut meta = self
                .meta
                .write()
                .map_err(|e| DbError::InternalError(e.to_string()))?;
            self.refresh_meta(db, &cf, &mut meta);
        }
        // Found before the schema is locked for writing: a filter reads
        // it. No other writer can come between the two.
        let rows = self.row_set()?;
        let found = find(&rows)?;
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        // Where each row is: by block, or in the tail.
        let mut in_blocks: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
        let mut in_tail = 0u64;
        let mut deleted = Vec::with_capacity(found.len());
        let mut batch = KvBatch::default();
        // Whether the rows that go are exactly the first ones.
        let from_the_front = rows.in_order() && {
            let mut positions: Vec<usize> = found
                .iter()
                .map(|(at, row)| rows.start(*at) + row)
                .collect();
            positions.sort_unstable();
            positions.iter().enumerate().all(|(at, held)| at == *held)
        };
        for (at, row) in found {
            let id = rows.uuid((at, row)).into_owned();
            if at < rows.blocks.len() {
                in_blocks.entry(at).or_default().push(row);
            } else {
                batch.delete_ks(&cf, format!("{}{}", COL_ROW_PREFIX, id).as_bytes());
                in_tail += 1;
            }
            deleted.push(id);
        }
        let Some(first) = deleted.iter().min() else {
            return Ok(deleted);
        };

        let mut new_meta = meta.clone();
        if from_the_front {
            Self::shift_indexes(&mut new_meta, deleted.len() as u64);
        } else {
            self.cap_indexes(&meta, &mut new_meta, first)?;
        }
        self.note_moved();
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
                    .map(|row| (block.uuids().text(row), block.row(row)))
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
                (0..block.len()).map(move |row| (block.uuids().text(row), block.row(row)))
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
                    .and_then(ColBlock::decode)
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
                    Some(to) => block.rename_column(at, to),
                    None => block.remove_column(at),
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
                if block.uuids().text(mid).as_str() < cutoff.as_str() {
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
                    .map(|row| (block.uuids().text(row), block.row(row)))
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
        // The rows that stay all move down by as many: an index answers
        // for them from that base. Out of id order (a replicated row with
        // an older id sits among the blocks), no index answers any more.
        if rows.in_order() {
            Self::shift_indexes(&mut new_meta, removed);
        } else {
            self.cap_indexes(&meta, &mut new_meta, &first)?;
        }
        self.note_moved();
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

    /// Whether the schema says `column` holds instants: its comparisons
    /// are then those of `Instants`, not of the values as written.
    fn is_timestamp(&self, column: &str) -> bool {
        self.meta.read().is_ok_and(|meta| {
            meta.columns
                .iter()
                .any(|c| c.name == column && c.data_type == ColumnType::Timestamp)
        })
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

    /// Aggregate a column with the specified operation, over every row.
    /// Count counts rows, nulls included; the others take numbers and
    /// return floats.
    pub fn aggregate(&self, column: &str, op: AggregateOp) -> DbResult<Value> {
        self.aggregate_rows(column, op, None)
    }

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
        self.group_by_rows(group_columns, agg_column, op, Some(filter))
    }

    /// [`Self::aggregate`] restricted to the rows matching `filter`.
    pub fn aggregate_where(
        &self,
        filter: &ColumnFilter,
        column: &str,
        op: AggregateOp,
    ) -> DbResult<Value> {
        self.aggregate_rows(column, op, Some(filter))
    }

    fn aggregate_rows(
        &self,
        column: &str,
        op: AggregateOp,
        filter: Option<&ColumnFilter>,
    ) -> DbResult<Value> {
        self.check_column(column)?;
        let rows = self.row_set()?;
        let positions = match filter {
            Some(filter) => Some(self.filter_positions(&rows, filter, 0..rows.len() as u64)?),
            None => None,
        };
        let mut all = Folded::default();
        each_row(&rows, positions.as_deref(), &[column], |cells| {
            all.add(&cells[0], op)
        });
        Ok(all.result(op))
    }

    /// One pass over the rows `filter` keeps (all of them without one),
    /// read from the arrays of their blocks: each row is added to the
    /// group of its key.
    ///
    /// The rows used to be listed by id, then each one found again by id
    /// in a set of rows built for it alone, once per group column, and the
    /// column aggregated read once more per group: two minutes for 500
    /// groups of 3.65 million rows, which this does in a fraction of a
    /// second. The result is the same: a group is the text of its keys
    /// joined by `|`, a key that reads as a number comes back as a float.
    fn group_by_rows(
        &self,
        group_columns: &[GroupByColumn],
        agg_column: &str,
        op: AggregateOp,
        filter: Option<&ColumnFilter>,
    ) -> DbResult<Vec<Value>> {
        self.check_column(agg_column)?;
        for column in group_columns {
            self.check_column(column.name())?;
        }
        let rows = self.row_set()?;
        let positions = match filter {
            Some(filter) => Some(self.filter_positions(&rows, filter, 0..rows.len() as u64)?),
            None => None,
        };
        let mut columns: Vec<&str> = group_columns.iter().map(|c| c.name()).collect();
        columns.push(agg_column);

        // The groups in the order their first row comes, found by the
        // values of their keys.
        let mut found: HashMap<Vec<KeyPart>, usize> = HashMap::new();
        let mut groups: Vec<(Vec<KeyPart>, Folded)> = Vec::new();
        let mut key: Vec<KeyPart> = Vec::with_capacity(group_columns.len());
        each_row(&rows, positions.as_deref(), &columns, |cells| {
            key.clear();
            for (cell, column) in cells.iter().zip(group_columns) {
                key.push(match column {
                    GroupByColumn::Simple(_) => cell.key_part(),
                    GroupByColumn::TimeBucket(_, interval) => {
                        KeyPart::Text(time_bucket(&cell.value(), interval))
                    }
                });
            }
            let at = match found.get(&key) {
                Some(at) => *at,
                None => {
                    found.insert(key.clone(), groups.len());
                    groups.push((key.clone(), Folded::default()));
                    groups.len() - 1
                }
            };
            groups[at].1.add(&cells[group_columns.len()], op);
        });

        // Two keys of different kinds with the same text (`1` and `"1"`)
        // are one group, as they always were.
        let mut by_text: HashMap<String, usize> = HashMap::new();
        let mut merged: Vec<(String, Folded)> = Vec::new();
        for (key, folded) in groups {
            let text = key.iter().map(KeyPart::text).collect::<Vec<_>>().join("|");
            match by_text.get(&text) {
                Some(at) => merged[*at].1.merge(folded),
                None => {
                    by_text.insert(text.clone(), merged.len());
                    merged.push((text, folded));
                }
            }
        }

        let mut results = Vec::with_capacity(merged.len());
        for (group_key, folded) in merged {
            let mut result_obj = serde_json::Map::new();
            let key_parts: Vec<&str> = group_key.split('|').collect();
            for (i, col_def) in group_columns.iter().enumerate() {
                let Some(part) = key_parts.get(i) else {
                    continue;
                };
                let number = match col_def {
                    GroupByColumn::TimeBucket(_, _) => None,
                    _ => part.parse::<f64>().ok(),
                };
                let value = match number {
                    Some(n) => Value::Number(
                        serde_json::Number::from_f64(n).unwrap_or(serde_json::Number::from(0)),
                    ),
                    None => Value::String(part.to_string()),
                };
                result_obj.insert(col_def.name().to_string(), value);
            }
            result_obj.insert("_agg".to_string(), folded.result(op));
            results.push(Value::Object(result_obj));
        }
        Ok(results)
    }

    /// How many rows `filter` keeps: no row is built, and where the
    /// arrays answer every comparison the positions of one block at a time
    /// are counted, not gathered (two million of them took most of the
    /// time of counting them).
    pub fn count_filtered(&self, filter: &ColumnFilter) -> DbResult<u64> {
        let rows = self.row_set()?;
        let range = 0..rows.len() as u64;
        let sides: Vec<&ColumnFilter> = match filter {
            ColumnFilter::And(sides) => sides.iter().collect(),
            leaf => vec![leaf],
        };
        let from_arrays = sides
            .iter()
            .all(|side| side.column().is_some() && self.index_answering(side).is_none());
        if !from_arrays {
            return Ok(self.filter_positions(&rows, filter, range)?.len() as u64);
        }
        for side in &sides {
            if let Some(column) = side.column() {
                self.check_column(column)?;
            }
        }
        let mut count = 0;
        self.by_block(&rows, &sides, &range, |here| count += here.len() as u64)?;
        Ok(count)
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
        new_meta.index_base.clear();
        self.note_moved();
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

    /// Create an index on a column, and return once it is built.
    pub fn create_index(&self, column: &str, index_type: ColumnarIndexType) -> DbResult<()> {
        self.build_index(column, index_type, None, false)
    }

    /// Build the index of a column again from the rows as they are, in
    /// place of the one it has: what brings back under the index the rows
    /// inserted since it was built.
    pub fn rebuild_index(&self, column: &str, index_type: ColumnarIndexType) -> DbResult<()> {
        self.build_index(column, index_type, None, true)
    }

    /// Start building an index in the background and return at once: the
    /// build is followed with [`Self::index_build`]. `replace` builds it
    /// in place of the index the column has; without it a column that has
    /// one is refused. A column whose index is already being built gets
    /// that build back.
    ///
    /// The build is written down in the metadata before the thread
    /// starts, so that a process that stops before it ends starts it
    /// again ([`Self::resume_index_builds`]).
    pub fn start_index_build(
        &self,
        column: &str,
        index_type: ColumnarIndexType,
        replace: bool,
    ) -> DbResult<Arc<IndexBuild>> {
        self.accept_index_build(column, index_type, replace, false)
    }

    fn accept_index_build(
        &self,
        column: &str,
        index_type: ColumnarIndexType,
        replace: bool,
        mark_only: bool,
    ) -> DbResult<Arc<IndexBuild>> {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self
            .meta
            .write()
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        self.refresh_meta(db, &cf, &mut meta);
        if let Some(running) = self.index_build(column) {
            if running.state() == BuildState::Building && meta.index_builds.contains_key(column) {
                if running.index_type == index_type {
                    return Ok(running);
                }
                return Err(DbError::BadRequest(format!(
                    "An index on column '{}' is being built",
                    column
                )));
            }
        }
        let held = meta
            .columns
            .iter()
            .find(|c| c.name == column)
            .ok_or_else(|| DbError::BadRequest(format!("Column '{}' not found", column)))?;
        if held.indexed && !replace {
            return Err(DbError::BadRequest(format!(
                "Column '{}' already has an index",
                column
            )));
        }
        let pending = PendingBuild {
            id: uuid7::uuid7().to_string(),
            index_type,
            started_at: chrono::Utc::now().timestamp(),
        };
        let mut new_meta = meta.clone();
        new_meta
            .index_builds
            .insert(column.to_string(), pending.clone());
        let mut batch = KvBatch::default();
        self.stage_meta(&cf, &mut batch, &new_meta)?;
        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
        let rows = meta.row_count;
        drop(meta);
        if mark_only {
            let build = IndexBuild::new(pending.id, column, pending.index_type, pending.started_at);
            return Ok(Arc::new(build));
        }
        Ok(self.spawn_build(column, pending, rows))
    }

    /// Accept a build and write it down, without starting it: what a
    /// process that stops right then leaves behind. For tests of the
    /// resumption.
    #[doc(hidden)]
    pub fn mark_index_build(
        &self,
        column: &str,
        index_type: ColumnarIndexType,
    ) -> DbResult<Arc<IndexBuild>> {
        self.accept_index_build(column, index_type, false, true)
    }

    /// The background build of the index on `column`, running or ended
    /// since this process started.
    pub fn index_build(&self, column: &str) -> Option<Arc<IndexBuild>> {
        let key = (
            Arc::as_ptr(&self.db) as usize,
            self.cf_name.clone(),
            column.to_string(),
        );
        INDEX_BUILDS
            .get(&key)
            .filter(|entry| entry.0.strong_count() > 0)
            .map(|entry| entry.1.clone())
    }

    /// The background builds of this collection.
    pub fn index_builds(&self) -> Vec<Arc<IndexBuild>> {
        let address = Arc::as_ptr(&self.db) as usize;
        let mut builds: Vec<Arc<IndexBuild>> = INDEX_BUILDS
            .iter()
            .filter(|entry| entry.key().0 == address && entry.key().1 == self.cf_name)
            .filter(|entry| entry.value().0.strong_count() > 0)
            .map(|entry| entry.value().1.clone())
            .collect();
        builds.sort_by(|a, b| a.column.cmp(&b.column));
        builds
    }

    /// Start again the builds the metadata names and no thread of this
    /// process runs: those a restart interrupted. Called when a collection
    /// is loaded; nothing is read for a collection with no build pending.
    pub fn resume_index_builds(&self) {
        let pending: Vec<(String, PendingBuild)> = match self.meta.read() {
            Ok(meta) if !meta.index_builds.is_empty() => meta
                .index_builds
                .iter()
                .map(|(column, build)| (column.clone(), build.clone()))
                .collect(),
            _ => return,
        };
        let rows = self.count() as u64;
        for (column, build) in pending {
            if self
                .index_build(&column)
                .is_none_or(|known| known.id != build.id)
            {
                tracing::info!(
                    "columnar '{}': resuming the build of the index on '{}'",
                    self.name,
                    column
                );
                self.spawn_build(&column, build, rows);
            }
        }
    }

    /// Register the build and start its thread, unless one of that id is
    /// already registered (two requests loading the collection at once).
    fn spawn_build(&self, column: &str, pending: PendingBuild, rows: u64) -> Arc<IndexBuild> {
        let address = Arc::as_ptr(&self.db) as usize;
        let key = (address, self.cf_name.clone(), column.to_string());
        let build = Arc::new(IndexBuild::new(
            pending.id.clone(),
            column,
            pending.index_type.clone(),
            pending.started_at,
        ));
        build
            .rows_total
            .store(rows, std::sync::atomic::Ordering::Relaxed);
        {
            INDEX_BUILDS.retain(|_, (db, _)| db.strong_count() > 0);
            let mut entry = INDEX_BUILDS
                .entry(key)
                .or_insert_with(|| (Arc::downgrade(&self.db), build.clone()));
            if entry.1.id == pending.id && !Arc::ptr_eq(&entry.1, &build) {
                return entry.1.clone();
            }
            *entry = (Arc::downgrade(&self.db), build.clone());
        }
        let collection = self.clone();
        let running = build.clone();
        let spawned = std::thread::Builder::new()
            .name("solidb-colidx".to_string())
            .spawn(move || {
                let column = running.column.clone();
                let built = collection.build_index(
                    &column,
                    running.index_type.clone(),
                    Some(&running),
                    true,
                );
                let state = match built {
                    Ok(()) => BuildState::Done,
                    // Stopped with the database: the mark stays, and the
                    // next process builds it.
                    Err(_) if running.cancelled() => BuildState::Failed("stopped".to_string()),
                    Err(error) => {
                        collection.forget_build(&column, &running.id);
                        tracing::warn!(
                            "columnar '{}': the index on '{}' was not built: {}",
                            collection.name,
                            column,
                            error
                        );
                        BuildState::Failed(error.to_string())
                    }
                };
                *running.state.lock() = state;
            });
        match spawned {
            Ok(thread) => {
                let mut threads = INDEX_THREADS.entry(address).or_default();
                threads.retain(|thread| !thread.is_finished());
                threads.push(thread);
            }
            Err(error) => {
                self.forget_build(column, &build.id);
                *build.state.lock() = BuildState::Failed(error.to_string());
            }
        }
        build
    }

    /// Take the mark of the build `id` out of the metadata: it failed, and
    /// must not be started again.
    fn forget_build(&self, column: &str, id: &str) {
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let Ok(mut meta) = self.meta.write() else {
            return;
        };
        let Some(cf) = self.ks.handle(&self.db) else {
            return;
        };
        self.refresh_meta(&self.db, &cf, &mut meta);
        if meta.index_builds.get(column).is_some_and(|b| b.id == id) {
            meta.index_builds.remove(column);
            let mut batch = KvBatch::default();
            if self.stage_meta(&cf, &mut batch, &meta).is_ok() {
                let _ = self.db.write(batch);
            }
        }
    }

    /// Build the index on `column` and store it.
    ///
    /// The rows are read and the entries built without the lock writers
    /// share: it is held to take the rows as they stand, and again to
    /// write. Rows appended meanwhile are simply after those the index
    /// holds. A write that moved a row meanwhile (a delete, a row with an
    /// older id) made the positions read wrong: the build starts again,
    /// and holds the lock throughout at its third attempt. It used to
    /// hold it from the first row to the write, and every insert waited.
    ///
    /// `build` is the background build this runs for: its progress is
    /// kept, it stops when the database closes, and it gives up when its
    /// mark is no longer in the metadata (the index was dropped, or
    /// another build took its place).
    fn build_index(
        &self,
        column: &str,
        index_type: ColumnarIndexType,
        build: Option<&IndexBuild>,
        replace: bool,
    ) -> DbResult<()> {
        use std::sync::atomic::Ordering::Relaxed;
        const ATTEMPTS: usize = 3;
        let shared = self.shared_write_lock();
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("Columnar CF '{}' not found", self.cf_name))
        })?;
        let stopped = || DbError::InternalError("the index build was stopped".to_string());
        // The column as the metadata has it, or why it cannot be indexed.
        let check = |meta: &ColumnarCollectionMeta| -> DbResult<usize> {
            let at = meta
                .columns
                .iter()
                .position(|c| c.name == column)
                .ok_or_else(|| DbError::BadRequest(format!("Column '{}' not found", column)))?;
            if meta.columns[at].indexed && !replace {
                return Err(DbError::BadRequest(format!(
                    "Column '{}' already has an index",
                    column
                )));
            }
            if let Some(build) = build {
                if meta
                    .index_builds
                    .get(column)
                    .is_none_or(|b| b.id != build.id)
                {
                    return Err(DbError::BadRequest(format!(
                        "The build of the index on '{}' was called off",
                        column
                    )));
                }
            }
            Ok(at)
        };
        for attempt in 1..=ATTEMPTS {
            let mut guard = Some(shared.lock());
            let (rows, moved) = {
                let mut meta = self
                    .meta
                    .write()
                    .map_err(|e| DbError::InternalError(e.to_string()))?;
                self.refresh_meta(db, &cf, &mut meta);
                check(&meta)?;
                (
                    self.row_set_with(&meta.compression)?,
                    self.store().moved.load(Relaxed),
                )
            };
            if attempt < ATTEMPTS {
                guard = None;
            }
            let total = rows.len() as u64;
            if let Some(build) = build {
                build.rows_total.store(total, Relaxed);
                build.rows_done.store(0, Relaxed);
            }

            // The entries, built in memory: they are written once, with
            // the schema, in one batch. They used to be written a row at a
            // time, each row reading the list of its value back, adding
            // itself and writing the list again: time quadratic in the
            // rows, and an index half written when a write failed.
            let mut entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
            let mut lists: HashMap<String, Vec<u64>> = HashMap::new();
            let mut chunk: Vec<String> = Vec::new();
            let mut position = 0u64;
            let mut cancelled = false;
            each_row(&rows, None, &[column], |cells| {
                if cancelled {
                    return;
                }
                let value = cells[0].value();
                match index_type {
                    ColumnarIndexType::Bitmap => lists
                        .entry(self.encode_bitmap_key(column, &value))
                        .or_default()
                        .push(position),
                    ColumnarIndexType::Bloom => {
                        chunk.push(value.to_string());
                        if chunk.len() as u64 == BLOOM_CHUNK_ROWS {
                            let at = position / BLOOM_CHUNK_ROWS;
                            entries.push(self.bloom_entry(column, at, &chunk));
                            chunk.clear();
                        }
                    }
                    _ => lists
                        .entry(self.encode_index_key(column, &value))
                        .or_default()
                        .push(position),
                }
                position += 1;
                if position.is_multiple_of(65_536) {
                    if let Some(build) = build {
                        build.rows_done.store(position, Relaxed);
                        cancelled = build.cancelled();
                    }
                }
            });
            if cancelled {
                return Err(stopped());
            }
            if !chunk.is_empty() {
                let at = (position - 1) / BLOOM_CHUNK_ROWS;
                entries.push(self.bloom_entry(column, at, &chunk));
            }
            for (key, positions) in lists {
                let stored = match index_type {
                    ColumnarIndexType::Bitmap => {
                        let last = positions.last().copied().unwrap_or(0) as usize;
                        let mut bitmap = vec![0u8; last / 8 + 1];
                        for position in positions {
                            bitmap[position as usize / 8] |= 1 << (position % 8);
                        }
                        self.compress_data(&bitmap, &CompressionType::Lz4)
                    }
                    _ => encode_positions(&positions),
                };
                entries.push((key.into_bytes(), stored));
            }
            drop(rows);
            if let Some(build) = build {
                build.rows_done.store(total, Relaxed);
            }

            // Written under the lock, if the positions read still stand.
            let _guard = guard.unwrap_or_else(|| shared.lock());
            let mut meta = self
                .meta
                .write()
                .map_err(|e| DbError::InternalError(e.to_string()))?;
            self.refresh_meta(db, &cf, &mut meta);
            let col_idx = check(&meta)?;
            if build.is_some_and(|build| build.cancelled()) {
                return Err(stopped());
            }
            if self.store().moved.load(Relaxed) != moved {
                continue;
            }
            let mut batch = KvBatch::default();
            if meta.columns[col_idx].indexed {
                self.stage_index_removal(&cf, &mut batch, column);
            }
            let size_bytes = entries
                .iter()
                .map(|(key, value)| (key.len() + value.len()) as u64)
                .sum();
            for (key, value) in entries {
                batch.put_ks(&cf, &key, value);
            }
            let idx_meta = ColumnarIndexMeta {
                column: column.to_string(),
                index_type: index_type.clone(),
                created_at: chrono::Utc::now().timestamp(),
                size_bytes,
            };
            let idx_meta_key = format!("{}{}", COL_IDX_META_PREFIX, column);
            batch.put_ks(&cf, idx_meta_key.as_bytes(), serde_json::to_vec(&idx_meta)?);

            // The schema, with the entries.
            let mut new_meta = meta.clone();
            new_meta.columns[col_idx].indexed = true;
            new_meta.columns[col_idx].index_type = Some(index_type.clone());
            new_meta.index_rows.insert(column.to_string(), total);
            new_meta.index_base.remove(column);
            new_meta.index_builds.remove(column);
            self.stage_meta(&cf, &mut batch, &new_meta)?;
            db.write(batch)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
            *meta = new_meta;
            return Ok(());
        }
        Err(DbError::InternalError(
            "the index build was overtaken by writes".to_string(),
        ))
    }

    /// One entry of a bloom index: what the rows of chunk `chunk` hold.
    fn bloom_entry(&self, column: &str, chunk: u64, values: &[String]) -> (Vec<u8>, Vec<u8>) {
        let mut filter = BloomFilter::with_num_bits(10_000).expected_items(1000);
        for value in values {
            filter.insert(value);
        }
        (
            self.encode_bloom_key(column, chunk).into_bytes(),
            serde_json::to_vec(&filter).unwrap_or_default(),
        )
    }

    /// Stage the removal of the column's entries, whatever kind of index
    /// wrote them, and of its metadata. Only those of a sorted or hash
    /// index used to go: a bitmap or a bloom index left its entries
    /// behind, and one created again on the column answered from the
    /// entry of a value no row holds any more.
    fn stage_index_removal(&self, cf: &KsCf, batch: &mut KvBatch, column: &str) {
        for kind in [
            COL_IDX_PREFIX,
            COL_IDX_BITMAP_PREFIX,
            COL_IDX_MINMAX_PREFIX,
            COL_IDX_BLOOM_PREFIX,
        ] {
            let prefix = format!("{}{}:", kind, column);
            let iter = self.db.prefix_iterator_ks(cf, prefix.as_bytes());
            for (key, _) in iter.flatten() {
                if !key.starts_with(prefix.as_bytes()) {
                    break;
                }
                batch.delete_ks(cf, &key);
            }
        }
        let idx_meta_key = format!("{}{}", COL_IDX_META_PREFIX, column);
        batch.delete_ks(cf, idx_meta_key.as_bytes());
    }

    /// Drop an index from a column, and call off a build of it that is
    /// still running.
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

        let col_idx = meta
            .columns
            .iter()
            .position(|c| c.name == column)
            .ok_or_else(|| DbError::BadRequest(format!("Column '{}' not found", column)))?;
        let building = meta.index_builds.contains_key(column);
        if !meta.columns[col_idx].indexed && !building {
            return Err(DbError::BadRequest(format!(
                "Column '{}' has no index",
                column
            )));
        }

        // The entries, the index's metadata and the schema in one batch:
        // they used to be deleted one write at a time, and a failure
        // part-way left an index with some of its entries.
        let mut batch = KvBatch::default();
        self.stage_index_removal(&cf, &mut batch, column);
        let mut new_meta = meta.clone();
        new_meta.columns[col_idx].indexed = false;
        new_meta.columns[col_idx].index_type = None;
        new_meta.index_rows.remove(column);
        new_meta.index_base.remove(column);
        new_meta.index_builds.remove(column);
        self.stage_meta(&cf, &mut batch, &new_meta)?;
        db.write(batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        *meta = new_meta;
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

    /// How many rows the index on `column` answers for: those it was
    /// built on that are still where it knows them. The rows after them
    /// are compared in their blocks.
    pub fn index_coverage(&self, column: &str) -> u64 {
        self.index_rows(column)
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
        // Over the rows the index was written for, in their positions of
        // then.
        let row_count = meta.index_rows.get(column).copied().unwrap_or(0)
            + meta.index_base.get(column).copied().unwrap_or(0);
        let chunk_count = row_count.div_ceil(BLOOM_CHUNK_ROWS);

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
                let start = chunk_id * BLOOM_CHUNK_ROWS;
                let end = (start + BLOOM_CHUNK_ROWS).min(row_count);
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
            Ok(Some(bytes)) => decode_positions(&bytes),
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
                        result.extend(decode_positions(&val_bytes)?);
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
                    self.by_block(rows, &scanned, &row_range, |here| found.extend(here))?;
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
        if self.is_timestamp(column) {
            let filter = Instants::of(filter)?;
            return Ok(scan_instants(rows, &filter, column, row_range));
        }
        let Some(index) = self.index_for(filter, column) else {
            return Ok(scan_leaf(rows, filter, column, row_range));
        };
        let held = self.index_rows(column).min(row_range.end);
        if held <= row_range.start {
            return Ok(scan_leaf(rows, filter, column, row_range));
        }
        // The index names a row by the position it had when it was
        // written: its position now, plus the rows removed from the front
        // since.
        let base = self.index_base(column);
        let mut positions: Vec<u64> = self
            .index_positions(filter, column, index.clone())?
            .into_iter()
            .filter_map(|position| position.checked_sub(base))
            .filter(|position| (row_range.start..held).contains(position))
            .collect();
        if index == ColumnarIndexType::Bloom {
            // It may name a chunk that does not hold the value.
            positions.retain(|position| {
                leaf_keeps(filter, &rows.value(rows.locate(*position as usize), column))
            });
        }
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

    /// The rows every comparison of `sides` keeps, among `row_range`, a
    /// block at a time and then the tail: `each` is given the positions
    /// found in one, ascending. A block one comparison rules out from what
    /// the block says of its column is passed over for all of them.
    fn by_block(
        &self,
        rows: &RowSet,
        sides: &[&ColumnFilter],
        row_range: &std::ops::Range<u64>,
        mut each: impl FnMut(Vec<u64>),
    ) -> DbResult<()> {
        for (block, part) in parts(rows, row_range) {
            let ruled_out = block.is_some_and(|block| {
                sides.iter().any(|side| {
                    side.column().is_some_and(|column| {
                        block.stats(column).is_some_and(|stats| {
                            if self.is_timestamp(column) {
                                instant_ends(stats.bounds, side) == Some((false, false))
                            } else {
                                block_rules_out(stats, side)
                            }
                        })
                    })
                })
            });
            if ruled_out {
                continue;
            }
            let mut here: Option<Vec<u64>> = None;
            for side in sides {
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
            each(here.unwrap_or_default());
        }
        Ok(())
    }

    /// The index on `column` that answers the comparison `filter`, if one
    /// does.
    ///
    /// An equality (or `IN`) is one entry of a hash, sorted or bitmap
    /// index, or the chunks a bloom index does not rule out. A range
    /// against a number is never an index's: the arrays answer it in one
    /// pass and skip the blocks out of range, where a sorted index reads
    /// one entry per distinct value (0.5 s against 0.2 s for a hundred
    /// rows out of a million, card #891). There used to be a `minmax`
    /// index for ranges; it knew only what each block already does, and
    /// was removed.
    ///
    /// None answers for a `timestamp` column: an index holds the values as
    /// they were written, and two writings of one instant are two entries.
    fn index_for(&self, filter: &ColumnFilter, column: &str) -> Option<ColumnarIndexType> {
        if self.is_timestamp(column) {
            return None;
        }
        let index = self.get_index_type(column)?;
        let answers = match filter {
            ColumnFilter::Eq(..) | ColumnFilter::In(..) => true,
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

    /// The positions `index` gives for the comparison `filter`, as they
    /// were when it was written. A bloom index may name rows that do not
    /// hold the value: the caller checks them.
    fn index_positions(
        &self,
        filter: &ColumnFilter,
        column: &str,
        index: ColumnarIndexType,
    ) -> DbResult<Vec<u64>> {
        let lookup = |value: &Value| match index {
            ColumnarIndexType::Bitmap => self.index_lookup_bitmap(column, value),
            ColumnarIndexType::Bloom => self.get_candidate_rows_from_bloom(column, value),
            _ => self.index_lookup_eq(column, value),
        };
        match filter {
            ColumnFilter::Eq(_, value) => lookup(value),
            ColumnFilter::In(_, values) => {
                let mut found = Vec::new();
                for value in values {
                    found.extend(lookup(value)?);
                }
                found.sort_unstable();
                found.dedup();
                Ok(found)
            }
            _ => self.index_range_scan(column, filter),
        }
    }
}

/// The stored metadata of a collection without its `minmax` indexes.
///
/// That kind of index (min and max per 1,000 rows, for ranges) was removed
/// after 2.11: every block carries its own bounds, and a range was no
/// longer answered from it. A collection that still has one loses it here,
/// the first time it is loaded: its entries are deleted and its column is
/// left with no index, which is how it has answered since. Without this
/// the metadata would not be read at all, the kind being unknown.
fn retire_minmax(
    db: &DB,
    ks: &Keyspace,
    cf_name: &str,
    name: &str,
    meta_bytes: Vec<u8>,
) -> DbResult<Vec<u8>> {
    const KIND: &[u8] = b"\"minmax\"";
    if !meta_bytes.windows(KIND.len()).any(|at| at == KIND) {
        return Ok(meta_bytes);
    }
    let lock = COLUMNAR_WRITE_LOCKS
        .entry((db as *const DB as usize, cf_name.to_string()))
        .or_default()
        .clone();
    let _held = lock.lock();
    let cf = ks.handle(db).ok_or_else(|| {
        DbError::CollectionNotFound(format!("Columnar collection '{}' not found", name))
    })?;
    // As another handle may have left it meanwhile.
    let meta_key = format!("{}{}", COL_META_PREFIX, name);
    let stored = db
        .get_ks(&cf, meta_key.as_bytes())
        .map_err(|e| DbError::InternalError(e.to_string()))?
        .unwrap_or(meta_bytes);
    let mut meta: Value = serde_json::from_slice(&stored)?;
    let mut retired = Vec::new();
    if let Some(columns) = meta.get_mut("columns").and_then(Value::as_array_mut) {
        for column in columns {
            if column.get("index_type").and_then(Value::as_str) != Some("minmax") {
                continue;
            }
            column["indexed"] = Value::Bool(false);
            column["index_type"] = Value::Null;
            if let Some(name) = column.get("name").and_then(Value::as_str) {
                retired.push(name.to_string());
            }
        }
    }
    if retired.is_empty() {
        return Ok(stored);
    }
    let mut batch = KvBatch::default();
    for column in &retired {
        if let Some(held) = meta.get_mut("index_rows").and_then(Value::as_object_mut) {
            held.remove(column);
        }
        batch.delete_ks(&cf, format!("{}{}", COL_IDX_META_PREFIX, column).as_bytes());
        let prefix = format!("{}{}:", COL_IDX_MINMAX_PREFIX, column);
        for (key, _) in db.prefix_iterator_ks(&cf, prefix.as_bytes()).flatten() {
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            batch.delete_ks(&cf, &key);
        }
    }
    let bytes = serde_json::to_vec(&meta)?;
    batch.put_ks(&cf, meta_key.as_bytes(), &bytes);
    db.write(batch)
        .map_err(|e| DbError::InternalError(e.to_string()))?;
    tracing::info!(
        "columnar collection '{}': minmax index removed from {} (the kind no longer exists; ranges are answered from the blocks' bounds)",
        name,
        retired.join(", ")
    );
    Ok(bytes)
}

/// The value of one column for one row, as an aggregation reads it:
/// where it is in its block's array, or the value itself (the tail, and
/// rows out of id order).
enum Cell<'a> {
    In(&'a Column, usize),
    Is(&'a Value),
    Null,
}

impl Cell<'_> {
    fn value(&self) -> Value {
        match self {
            Cell::In(column, row) => column.value(*row),
            Cell::Is(value) => (*value).clone(),
            Cell::Null => Value::Null,
        }
    }

    /// As `Value::as_f64` of the value.
    fn number(&self) -> Option<f64> {
        match self {
            Cell::In(column, row) => column.number(*row),
            Cell::Is(value) => value.as_f64(),
            Cell::Null => None,
        }
    }

    /// The value as a key of a group. An integer stays one; anything else
    /// is the text the groups have always been told apart by: a string's
    /// contents, the JSON text of any other value.
    fn key_part(&self) -> KeyPart {
        if let Cell::In(column, row) = self {
            let null = column.nulls.as_ref().is_some_and(|nulls| nulls[*row]);
            if let (ColData::Int(values), false) = (&column.data, null) {
                return KeyPart::Int(values[*row]);
            }
        }
        match self.value() {
            Value::Number(number) if number.is_i64() => {
                KeyPart::Int(number.as_i64().unwrap_or_default())
            }
            Value::String(text) => KeyPart::Text(text),
            other => KeyPart::Text(other.to_string()),
        }
    }
}

/// One key of a group.
#[derive(Clone, PartialEq, Eq, Hash)]
enum KeyPart {
    Int(i64),
    Text(String),
}

impl KeyPart {
    fn text(&self) -> String {
        match self {
            KeyPart::Int(value) => value.to_string(),
            KeyPart::Text(text) => text.clone(),
        }
    }
}

/// What an aggregation keeps of the rows of one group.
#[derive(Default)]
struct Folded {
    /// Rows, nulls included.
    rows: u64,
    /// Rows holding a number, and their sum in the order of the rows.
    numbers: u64,
    sum: f64,
    min: Option<f64>,
    max: Option<f64>,
    /// The JSON text of each value seen, for COUNT_DISTINCT only.
    distinct: std::collections::HashSet<String>,
}

impl Folded {
    fn add(&mut self, cell: &Cell<'_>, op: AggregateOp) {
        self.rows += 1;
        match op {
            AggregateOp::Count => {}
            AggregateOp::CountDistinct => {
                self.distinct.insert(cell.value().to_string());
            }
            _ => {
                if let Some(number) = cell.number() {
                    self.numbers += 1;
                    self.sum += number;
                    self.min = Some(self.min.map_or(number, |min| min.min(number)));
                    self.max = Some(self.max.map_or(number, |max| max.max(number)));
                }
            }
        }
    }

    fn merge(&mut self, other: Folded) {
        self.rows += other.rows;
        self.numbers += other.numbers;
        self.sum += other.sum;
        self.min = match (self.min, other.min) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.max = match (self.max, other.max) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        self.distinct.extend(other.distinct);
    }

    fn result(&self, op: AggregateOp) -> Value {
        match op {
            AggregateOp::Count => Value::Number(self.rows.into()),
            AggregateOp::Sum => json_number(self.sum),
            AggregateOp::Avg if self.numbers == 0 => Value::Null,
            AggregateOp::Avg => json_number(self.sum / self.numbers as f64),
            AggregateOp::Min => self.min.map_or(Value::Null, json_number),
            AggregateOp::Max => self.max.map_or(Value::Null, json_number),
            AggregateOp::CountDistinct => Value::Number(self.distinct.len().into()),
        }
    }
}

/// `each` for every row at `positions` (ascending; every row without
/// them), in that order, with its cells for `columns`. The columns of a
/// block are looked up once for all its rows.
fn each_row<'r>(
    rows: &'r RowSet,
    positions: Option<&[u64]>,
    columns: &[&str],
    mut each: impl FnMut(&[Cell<'r>]),
) {
    let mut cells: Vec<Cell<'r>> = Vec::with_capacity(columns.len());
    if !rows.in_order() {
        let mut one = |position: usize| {
            let at = rows.locate(position);
            cells.clear();
            for column in columns {
                cells.push(match rows.blocks.get(at.0) {
                    Some(block) => block
                        .column(column)
                        .map_or(Cell::Null, |held| Cell::In(held, at.1)),
                    None => rows.tail[at.1].1.get(*column).map_or(Cell::Null, Cell::Is),
                });
            }
            each(&cells);
        };
        match positions {
            Some(positions) => positions.iter().for_each(|p| one(*p as usize)),
            None => (0..rows.len()).for_each(one),
        }
        return;
    }
    // Where the positions not yet read start.
    let mut next = 0;
    for part in 0..=rows.blocks.len() {
        let start = rows.start(part);
        let block = rows.blocks.get(part);
        let held = block.map_or(rows.tail.len(), |block| block.len());
        let here: Option<&[u64]> = positions.map(|positions| {
            let from = next;
            while next < positions.len() && (positions[next] as usize) < start + held {
                next += 1;
            }
            &positions[from..next]
        });
        if held == 0 || here.is_some_and(|here| here.is_empty()) {
            continue;
        }
        let in_block: Option<Vec<Option<&'r Column>>> =
            block.map(|block| columns.iter().map(|column| block.column(column)).collect());
        let mut one = |row: usize| {
            cells.clear();
            match &in_block {
                Some(held) => cells.extend(
                    held.iter()
                        .map(|column| column.map_or(Cell::Null, |column| Cell::In(column, row))),
                ),
                None => cells.extend(
                    columns
                        .iter()
                        .map(|column| rows.tail[row].1.get(*column).map_or(Cell::Null, Cell::Is)),
                ),
            }
            each(&cells);
        };
        match here {
            Some(here) => here.iter().for_each(|p| one(*p as usize - start)),
            None => (0..held).for_each(one),
        }
    }
}

/// The key of a row in a `TimeBucket` group: the start of the bucket of
/// an RFC 3339 instant, as RFC 3339 text; the JSON text of any other
/// value.
fn time_bucket(value: &Value, interval: &str) -> String {
    let Ok(instant) = chrono::DateTime::parse_from_rfc3339(value.as_str().unwrap_or("")) else {
        return value.to_string();
    };
    let (num, unit) = parsers::parse_interval(interval).unwrap_or((1, "h"));
    let bucket_size = match unit {
        "s" => num,
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        _ => num,
    };
    let bucketed = (instant.timestamp() / bucket_size) * bucket_size;
    match chrono::DateTime::from_timestamp(bucketed, 0) {
        Some(start) => start.to_rfc3339(),
        None => value.to_string(),
    }
}

/// The instant a value of a `timestamp` column names, in microseconds
/// since the epoch; `None` when it names none (null, a text that is not a
/// date).
///
/// Read as the SDBQL date functions read a date (`parse_date_str`,
/// `number_to_epoch_ms`): ISO 8601 text, with `T` or a space, a date
/// alone, any fraction of a second, an offset or none (then UTC); a number
/// is milliseconds, or seconds below `SECONDS_EPOCH_THRESHOLD`.
fn instant(value: &Value) -> Option<i64> {
    use crate::sdbql::executor::utils::{number_to_epoch_ms, parse_date_str};
    match value {
        Value::Number(number) => number_to_epoch_ms(number).ok()?.checked_mul(1000),
        Value::String(text) => Some(parse_date_str(text)?.to_utc().timestamp_micros()),
        _ => None,
    }
}

/// A comparison on a `timestamp` column, with the instants its constants
/// name read once.
///
/// The column takes a string or an integer, so one instant has many
/// writings (`2026-01-01T00:00:00Z`, `2026-01-01 01:00:00+01:00`,
/// `1767225600`). Compared as the values they are, an equality found only
/// the writing that was inserted, and a range, which compares numbers
/// only, found no row at all.
struct Instants<'f> {
    filter: &'f ColumnFilter,
    /// The instant of each constant, in the filter's order.
    at: Vec<Option<i64>>,
}

impl<'f> Instants<'f> {
    /// A range against something that is not an instant is refused: it
    /// would silently keep nothing.
    fn of(filter: &'f ColumnFilter) -> DbResult<Self> {
        let at: Vec<Option<i64>> = match filter {
            ColumnFilter::Eq(_, value) | ColumnFilter::Ne(_, value) => vec![instant(value)],
            ColumnFilter::In(_, values) => values.iter().map(instant).collect(),
            ColumnFilter::Gt(column, bound)
            | ColumnFilter::Gte(column, bound)
            | ColumnFilter::Lt(column, bound)
            | ColumnFilter::Lte(column, bound) => {
                let Some(at) = instant(bound) else {
                    return Err(DbError::BadRequest(format!(
                        "Column '{column}' is a timestamp: {bound} is not one \
                         (ISO 8601 text, or a number of seconds or milliseconds)"
                    )));
                };
                vec![Some(at)]
            }
            ColumnFilter::And(_) | ColumnFilter::Or(_) => Vec::new(),
        };
        Ok(Self { filter, at })
    }

    /// Whether the comparison keeps a row holding `value`. Where either
    /// side names no instant, an equality is that of the values (which is
    /// what finds a null) and a range keeps nothing.
    fn keeps(&self, value: &Value) -> bool {
        use std::cmp::Ordering::{Equal, Greater, Less};
        let held = instant(value);
        let same = |wanted: &Value, at: Option<i64>| match (held, at) {
            (Some(held), Some(at)) => held == at,
            _ => value == wanted,
        };
        let order = || Some(held?.cmp(&self.at.first().copied().flatten()?));
        match self.filter {
            ColumnFilter::Eq(_, wanted) => same(wanted, self.at[0]),
            ColumnFilter::Ne(_, wanted) => !same(wanted, self.at[0]),
            ColumnFilter::In(_, wanted) => wanted
                .iter()
                .zip(&self.at)
                .any(|(wanted, at)| same(wanted, *at)),
            ColumnFilter::Gt(..) => order() == Some(Greater),
            ColumnFilter::Gte(..) => matches!(order(), Some(Greater | Equal)),
            ColumnFilter::Lt(..) => order() == Some(Less),
            ColumnFilter::Lte(..) => matches!(order(), Some(Less | Equal)),
            ColumnFilter::And(_) | ColumnFilter::Or(_) => false,
        }
    }
}

/// Whether a range on a `timestamp` column keeps the smallest and the
/// largest integer of a block: `None` when that does not decide for those
/// between them. It does only when both are read in the same unit: an
/// integer is seconds below `SECONDS_EPOCH_THRESHOLD` and milliseconds
/// from it, so the instants follow the integers on each side of it and
/// not across.
fn instant_ends(bounds: Bounds, filter: &ColumnFilter) -> Option<(bool, bool)> {
    if !matches!(
        filter,
        ColumnFilter::Gt(..) | ColumnFilter::Gte(..) | ColumnFilter::Lt(..) | ColumnFilter::Lte(..)
    ) {
        return None;
    }
    let Bounds::Int(low, high) = bounds else {
        return None;
    };
    let threshold = crate::sdbql::executor::utils::SECONDS_EPOCH_THRESHOLD as i64;
    let unit = |value: i64| (value <= -threshold) as i8 - (value >= threshold) as i8;
    if unit(low) != unit(high) {
        return None;
    }
    let filter = Instants::of(filter).ok()?;
    Some((
        filter.keeps(&Value::from(low)),
        filter.keeps(&Value::from(high)),
    ))
}

/// `scan_leaf` for a `timestamp` column: the positions, among `range`, of
/// the rows whose instant the comparison keeps, ascending. A block of
/// strings reads each distinct string once.
fn scan_instants(
    rows: &RowSet,
    filter: &Instants,
    column: &str,
    range: std::ops::Range<u64>,
) -> Vec<u64> {
    let range = range.start..range.end.min(rows.len() as u64);
    let mut kept = Vec::new();
    if !rows.in_order() {
        kept.extend(
            range.filter(|position| {
                filter.keeps(&rows.value(rows.locate(*position as usize), column))
            }),
        );
        return kept;
    }
    for (b, block) in rows.blocks.iter().enumerate() {
        let start = rows.start(b) as u64;
        let from = range.start.max(start);
        let to = range.end.min(start + block.len() as u64);
        if from >= to {
            continue;
        }
        let Some(stats) = block.stats(column) else {
            // Sealed before the column was added: null for every row.
            if filter.keeps(&Value::Null) {
                kept.extend(from..to)
            }
            continue;
        };
        match instant_ends(stats.bounds, filter.filter) {
            Some((false, false)) => continue,
            Some((true, true)) if !stats.nulls => {
                kept.extend(from..to);
                continue;
            }
            _ => {}
        }
        let Some(values) = block.column(column) else {
            continue;
        };
        let inside = (from - start) as usize..(to - start) as usize;
        let null = |row: usize| values.nulls.as_ref().is_some_and(|nulls| nulls[row]);
        match &values.data {
            ColData::Str { dict, codes } => {
                let null_kept = filter.keeps(&Value::Null);
                let of_string: Vec<bool> = dict
                    .iter()
                    .map(|text| filter.keeps(&Value::String(text.to_string())))
                    .collect();
                kept.extend(
                    inside
                        .filter(|&row| {
                            if null(row) {
                                null_kept
                            } else {
                                of_string[codes[row] as usize]
                            }
                        })
                        .map(|row| start + row as u64),
                );
            }
            _ => kept.extend(
                inside
                    .filter(|&row| filter.keeps(&values.value(row)))
                    .map(|row| start + row as u64),
            ),
        }
    }
    let start = rows.start(rows.blocks.len()) as u64;
    for (at, (_, row)) in rows.tail.iter().enumerate() {
        let position = start + at as u64;
        if range.contains(&position) && filter.keeps(row.get(column).unwrap_or(&Value::Null)) {
            kept.push(position);
        }
    }
    kept
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

/// Whether the comparison `filter` keeps the smallest value of a column
/// in a block, and the largest: `None` when the bounds are not known, or
/// do not decide for the values between them (anything but a range
/// against a number).
fn ends_kept(bounds: Bounds, filter: &ColumnFilter) -> Option<(bool, bool)> {
    let bound = match filter {
        ColumnFilter::Gt(_, bound)
        | ColumnFilter::Gte(_, bound)
        | ColumnFilter::Lt(_, bound)
        | ColumnFilter::Lte(_, bound) => bound,
        _ => return None,
    };
    // A bound that is not a number is not compared as one (`against`).
    if !bound.is_number() {
        return None;
    }
    match bounds {
        Bounds::Int(low, high) => {
            let keeps = |value: i64| leaf_keeps(filter, &Value::from(value));
            Some((keeps(low), keeps(high)))
        }
        Bounds::Float(low, high) => {
            let keeps = |value: f64| leaf_keeps(filter, &Value::from(value));
            Some((keeps(low), keeps(high)))
        }
        Bounds::Unknown => None,
    }
}

/// Whether what a block says of its column, without decoding it, shows
/// that the comparison `filter` keeps none of its rows. `false` is always
/// a safe answer.
fn block_rules_out(stats: ColStats, filter: &ColumnFilter) -> bool {
    // What keeps neither end of a range keeps nothing between them.
    if let Some(ends) = ends_kept(stats.bounds, filter) {
        return ends == (false, false);
    }
    match filter {
        ColumnFilter::Eq(_, Value::Null) => !stats.nulls,
        ColumnFilter::Eq(_, wanted) => {
            let (low, high) = match stats.bounds {
                Bounds::Int(low, high) => (low as f64, high as f64),
                Bounds::Float(low, high) => (low, high),
                Bounds::Unknown => return false,
            };
            // As floats, where an integer past 2^53 may round onto a
            // bound: then the block is read, which is always right.
            wanted.as_f64().is_some_and(|w| w < low || high < w)
        }
        _ => false,
    }
}

/// Whether it shows that the comparison keeps every row: a range that
/// keeps both ends of a column with no null.
fn block_taken_whole(stats: ColStats, filter: &ColumnFilter) -> bool {
    !stats.nulls && ends_kept(stats.bounds, filter) == Some((true, true))
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
        // What the block says of the column, before any of it is decoded:
        // it may be passed over, or taken whole.
        match block.stats(column) {
            Some(stats) if block_rules_out(stats, filter) => {}
            Some(stats) if block_taken_whole(stats, filter) => kept.extend(from..to),
            Some(_) => {
                if let Some(values) = block.column(column) {
                    let inside = (from - start) as usize..(to - start) as usize;
                    scan_block(values, filter, inside, start, &mut kept)
                }
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
            let keeps = |value: f64| value.partial_cmp(&bound).is_some_and(keeps);
            if let Some((low, high)) = column.float_range() {
                match (keeps(low), keeps(high)) {
                    (false, false) => return,
                    (true, true) if column.nulls.is_none() => {
                        kept.extend(inside.map(|row| base + row as u64));
                        return;
                    }
                    _ => {}
                }
            }
            kept.extend(
                inside
                    .filter(|&row| !null(row) && keeps(values[row]))
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
    /// A collection whose blocks are read from disk decodes, of each
    /// block, the columns a query reads, and nothing of a block the head
    /// rules out.
    #[test]
    fn a_query_decodes_the_columns_it_reads() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = crate::storage::StorageEngine::new(tmp.path()).unwrap();
        engine.create_database("_system".to_string()).unwrap();
        let db = engine.get_database("_system").unwrap();
        db.create_collection("_columnar_lazy".to_string(), None)
            .unwrap();
        let columns = [
            ("host", ColumnType::String),
            ("v", ColumnType::Int64),
            ("ms", ColumnType::Float64),
            ("note", ColumnType::String),
        ]
        .into_iter()
        .map(|(name, data_type)| ColumnDef {
            name: name.to_string(),
            data_type,
            nullable: false,
            indexed: false,
            index_type: None,
        })
        .collect();
        let col = ColumnarCollection::new(
            "lazy".to_string(),
            "_system",
            db.db_arc(),
            columns,
            CompressionType::Lz4,
        )
        .unwrap();
        // Three blocks of 5,000 rows.
        for block in 0..3 {
            col.insert_rows(
                (block * 5000..(block + 1) * 5000)
                    .map(|i| {
                        serde_json::json!({"host": format!("h{}", i % 7), "v": i,
                            "ms": i as f64 / 2.0, "note": format!("n{i}")})
                    })
                    .collect(),
            )
            .unwrap();
        }
        let decoded = |col: &ColumnarCollection| -> Vec<(bool, usize)> {
            let store = col.store();
            let held = store.blocks.read();
            held.as_ref()
                .unwrap()
                .iter()
                .map(|block| block.decoded())
                .collect()
        };
        // As after a start, or after the budget took the blocks back.
        let forget = |col: &ColumnarCollection| *col.store().blocks.write() = None;

        // A range the second block holds: the others are passed over from
        // their heads, and the second gives the column compared and the
        // one returned.
        forget(&col);
        let range = ColumnFilter::And(vec![
            ColumnFilter::Gte("v".to_string(), serde_json::json!(7000)),
            ColumnFilter::Lt("v".to_string(), serde_json::json!(7010)),
        ]);
        let rows = col.scan_filtered(&range, &["host"]).unwrap();
        assert_eq!(rows.len(), 10);
        assert_eq!(rows[3], serde_json::json!({"host": "h3"}));
        assert_eq!(decoded(&col), [(false, 0), (false, 2), (false, 0)]);

        // A range that takes whole blocks decodes nothing to count them.
        forget(&col);
        let all = ColumnFilter::Gte("ms".to_string(), serde_json::json!(2500.0));
        assert_eq!(col.count_filtered(&all).unwrap(), 10_000);
        assert_eq!(decoded(&col), [(false, 0), (false, 0), (false, 0)]);
        let none = ColumnFilter::Eq("v".to_string(), serde_json::json!(99_999));
        assert_eq!(col.count_filtered(&none).unwrap(), 0);
        assert_eq!(decoded(&col), [(false, 0), (false, 0), (false, 0)]);

        // An equality on a string decodes that column, in every block.
        let host = ColumnFilter::Eq("host".to_string(), serde_json::json!("h2"));
        assert_eq!(col.count_filtered(&host).unwrap(), 15_000 / 7 + 1);
        assert_eq!(decoded(&col), [(false, 1), (false, 1), (false, 1)]);

        // Listing the ids decodes them, and what is kept counts what was
        // decoded and what is still as it is stored.
        let ids = col.list_row_uuids().unwrap();
        assert_eq!(ids.len(), 15_000);
        assert_eq!(decoded(&col), [(true, 1), (true, 1), (true, 1)]);
        let kept: usize = {
            let store = col.store();
            let held = store.blocks.read();
            held.as_ref().unwrap().iter().map(|b| b.bytes()).sum()
        };
        // The ids and a column of strings decoded, the three other columns
        // as they are stored: more than the ids, less than everything.
        assert!(kept >= 15_000 * 16, "{kept}");
        assert!(kept < 15_000 * (16 + 8 + 8 + 8 + 30), "{kept}");
        assert!(col.delete_row(&ids[12_000]).unwrap());
        // (Row 12,000 held "h2".)
        assert_eq!(col.count_filtered(&host).unwrap(), 15_000 / 7);
    }

    /// A collection that holds a `minmax` index, a kind that no longer
    /// exists: loading it removes the index and leaves the column plain.
    #[test]
    fn a_minmax_index_is_removed_when_its_collection_is_loaded() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = crate::storage::StorageEngine::new(tmp.path()).unwrap();
        engine.create_database("_system".to_string()).unwrap();
        let db = engine.get_database("_system").unwrap();
        db.create_collection("_columnar_mm".to_string(), None)
            .unwrap();
        let columns = ["v", "w"]
            .iter()
            .map(|name| ColumnDef {
                name: name.to_string(),
                data_type: ColumnType::Int64,
                nullable: false,
                indexed: false,
                index_type: None,
            })
            .collect();
        let col = ColumnarCollection::new(
            "mm".to_string(),
            "_system",
            db.db_arc(),
            columns,
            CompressionType::Lz4,
        )
        .unwrap();
        col.insert_rows(
            (0..5000)
                .map(|i| serde_json::json!({"v": i, "w": i % 9}))
                .collect(),
        )
        .unwrap();
        col.create_index("w", ColumnarIndexType::Hash).unwrap();

        // As 2.11 stored it: the column marked, the index's own record,
        // and a chunk of bounds.
        {
            let cf = col.ks.handle(&col.db).unwrap();
            let meta_key = format!("{}{}", COL_META_PREFIX, "mm");
            let stored = col.db.get_ks(&cf, meta_key.as_bytes()).unwrap().unwrap();
            let mut meta: Value = serde_json::from_slice(&stored).unwrap();
            meta["columns"][0]["indexed"] = Value::Bool(true);
            meta["columns"][0]["index_type"] = Value::String("minmax".to_string());
            meta["index_rows"]["v"] = serde_json::json!(5000);
            let mut batch = KvBatch::default();
            batch.put_ks(&cf, meta_key.as_bytes(), serde_json::to_vec(&meta).unwrap());
            batch.put_ks(
                &cf,
                format!("{}v", COL_IDX_META_PREFIX).as_bytes(),
                br#"{"column":"v","index_type":"minmax","created_at":0}"#,
            );
            batch.put_ks(
                &cf,
                format!("{}v:0", COL_IDX_MINMAX_PREFIX).as_bytes(),
                br#"{"min":0,"max":999,"count":1000}"#,
            );
            col.db.write(batch).unwrap();
        }

        let loaded = ColumnarCollection::load("mm".to_string(), "_system", db.db_arc()).unwrap();
        assert!(!loaded.has_index("v"));
        assert_eq!(loaded.get_index_type("w"), Some(ColumnarIndexType::Hash));
        let indexes = loaded.list_indexes().unwrap();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].column, "w");
        let cf = loaded.ks.handle(&loaded.db).unwrap();
        let left = loaded
            .db
            .prefix_iterator_ks(&cf, COL_IDX_MINMAX_PREFIX.as_bytes())
            .flatten()
            .filter(|(key, _)| key.starts_with(COL_IDX_MINMAX_PREFIX.as_bytes()))
            .count();
        assert_eq!(left, 0);
        // The column answers, and takes an index again.
        let range = ColumnFilter::Gte("v".to_string(), serde_json::json!(4990));
        assert_eq!(loaded.scan_filtered(&range, &["v"]).unwrap().len(), 10);
        loaded.create_index("v", ColumnarIndexType::Sorted).unwrap();
        // And a second load finds nothing to do.
        let again = ColumnarCollection::load("mm".to_string(), "_system", db.db_arc()).unwrap();
        assert_eq!(again.get_index_type("v"), Some(ColumnarIndexType::Sorted));
    }

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
    fn positions_are_packed_and_the_older_json_is_still_read() {
        for positions in [
            vec![],
            vec![0],
            vec![0, 1, 2, 3],
            vec![7, 500, 1_000, 70_000, 3_650_000, u64::MAX / 2],
        ] {
            let packed = encode_positions(&positions);
            assert_eq!(decode_positions(&packed).unwrap(), positions);
            // What 2.12.0 and earlier wrote.
            let json = serde_json::to_vec(&positions).unwrap();
            assert_eq!(decode_positions(&json).unwrap(), positions);
        }
        // A run of neighbours is a byte a row.
        let run: Vec<u64> = (1_000_000..1_010_000).collect();
        assert!(encode_positions(&run).len() < 10_010);
        // Cut short: an error, not a wrong answer.
        let packed = encode_positions(&[1, 2, 300]);
        assert!(decode_positions(&packed[..packed.len() - 1]).is_err());
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
