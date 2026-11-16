//! Column blocks: what a time series collection keeps beside its documents
//! so that an aggregation reads arrays instead of documents.
//!
//! A collection of type `timeseries` may declare a time field and the other
//! fields its aggregations read ([`BlocksDef`], stored under
//! `_stats:column_blocks`). Its documents stay what they are, and stay the
//! source: a block is derived from the documents of one range of keys
//! `[lo, hi)`, holds their time and their fields as arrays, and is thrown
//! away whenever a document of that range is written or deleted. A query
//! folds the blocks, and reads as documents whatever no block covers
//! (`sdbql/executor/execution/block_scan.rs`).
//!
//! **Rows.** Inside a block the rows are ordered by their string fields, in
//! the order the definition names them, then by time: the points of one
//! series are a run of rows, found by bisection, ordered by time. A string
//! field is a dictionary (sorted, so that codes order as the strings do) and
//! one code per row. A numeric field is all integers or all floats, as the
//! documents hold them, so that `SUM` and `MIN` return what they return over
//! the documents. A range in which one document does not fit that (a field
//! missing or null, a number among strings, a time that is not an integer)
//! gets a block without columns, which says only that the range was looked
//! at: its documents are read.
//!
//! **What keeps a block right.** A time series collection refuses updates,
//! so a block is wrong only if the set of keys in its range changed.
//! - Every document write announces its keys ([`BlockSet::wrote`], from
//!   `Collection::emit_change` and `Collection::publish_changes`), after
//!   the write: a block whose range holds one of them is deleted.
//! - A block is built from a snapshot taken after its range was registered
//!   as pending. A write announced before the registration is in the
//!   snapshot; one announced after it finds the pending range, and the
//!   block is not kept.
//! - The announcement follows the write, so a crash between the two leaves
//!   a block that no longer matches. A block read from disk is therefore
//!   not trusted until the keys of its range have been counted and hashed
//!   again, once per process ([`BlockSet::verified`]).
//!
//! Blocks are local to a node: nothing of them is replicated or dumped, and
//! each node builds its own from its documents.

use super::collection::{Collection, DOC_PREFIX};
use super::keyspace::{Keyspace, KsBatchExt, KsDbExt, KsState};
use super::serializer::deserialize_doc_projected;
use super::RocksDb as DB;
use crate::error::{DbError, DbResult};
use crate::sdbql::executor::exact_sum::ExactSum;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// Where a collection's definition is stored.
pub const BLOCKS_KEY: &str = "_stats:column_blocks";
/// `blk:<lo>` holds the block of the keys from `lo`.
pub const BLOCK_PREFIX: &str = "blk:";
/// The most documents a block holds. Blocks are sealed small, as documents
/// arrive, and merged up to this.
pub const BLOCK_ROWS: usize = 65_536;
/// The fewest documents a write seals into a block.
const SEAL_ROWS: usize = 1_024;
/// Adjacent blocks merged at once, each at most `BLOCK_ROWS / MERGE_FAN`.
const MERGE_FAN: usize = 8;
/// Rows of a block, in time order, that share a subtotal.
pub(crate) const CHUNK_ROWS: usize = 512;
/// Documents left unsealed behind the newest block by a write: the most
/// recent keys are the ones a late write is likely to fall among.
const TAIL_KEPT: usize = 512;

/// The fields a collection keeps in blocks.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlocksDef {
    /// The field holding a point's time, as integer milliseconds.
    pub time: String,
    /// The other fields: strings (a series' name, a tag) or numbers.
    pub fields: Vec<String>,
}

impl BlocksDef {
    fn check(&self) -> DbResult<()> {
        let plain = |f: &String| !f.is_empty() && !f.contains('.') && !f.starts_with('_');
        let distinct = self
            .fields
            .iter()
            .enumerate()
            .all(|(i, f)| f != &self.time && !self.fields[..i].contains(f));
        if !plain(&self.time) || !self.fields.iter().all(plain) || !distinct {
            return Err(DbError::BadRequest(
                "column blocks take a time field and distinct top-level fields, none of them a \
                 system field"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// The time field then the others: the order of a row's values.
    pub(crate) fn projection(&self) -> Vec<String> {
        let mut fields = vec![self.time.clone()];
        fields.extend(self.fields.iter().cloned());
        fields
    }
}

/// One field of a block's rows.
#[derive(Serialize, Deserialize)]
pub(crate) enum Column {
    Int(Vec<i64>),
    Float(Vec<f64>),
    /// Integers and floats: each value as a float, and whether the document
    /// holds it as an integer (one a float holds exactly).
    Mixed {
        values: Vec<f64>,
        ints: Vec<bool>,
    },
    /// `dict` is sorted; a row holds the position of its string in it.
    Str {
        dict: Vec<String>,
        codes: Vec<u32>,
    },
}

/// The rows of a block, ordered by the string columns then by time.
#[derive(Serialize, Deserialize)]
pub(crate) struct Columns {
    pub(crate) min_time: i64,
    pub(crate) max_time: i64,
    pub(crate) time: Vec<i64>,
    /// One per field of the definition, in its order.
    pub(crate) cols: Vec<Column>,
    /// One per field: what is known of the whole column.
    pub(crate) totals: Vec<Option<Total>>,
    /// The rows in time order.
    pub(crate) by_time: Vec<u32>,
    /// One per field: what is known of each `CHUNK_ROWS` rows of
    /// `by_time`.
    pub(crate) chunks: Vec<Vec<Option<Total>>>,
}

/// The exact sum and the extremes of a numeric column, or of some rows of
/// it: an aggregate over all of them needs no row. The sum is the state the
/// rows would leave in a `SUM` one by one, integers counted as integers.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Total {
    pub(crate) sum: ExactSum,
    /// The first of the smallest values, as the document holds it.
    pub(crate) min: Extreme,
    pub(crate) max: Extreme,
}

/// A number as a document holds it: an integer or a float.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub(crate) enum Extreme {
    Int(i64),
    Float(f64),
}

impl Extreme {
    pub(crate) fn number(self) -> Option<serde_json::Number> {
        match self {
            Extreme::Int(i) => Some(i.into()),
            Extreme::Float(x) => serde_json::Number::from_f64(x),
        }
    }
}

/// Integers up to this size are exact as floats.
const EXACT_INT: i64 = 1 << 53;

/// The total of the rows `rows` of `column` (`None`: all of them); `None`
/// for strings and for no row.
fn total_of(column: &Column, rows: Option<&[u32]>) -> Option<Total> {
    let len = match column {
        Column::Int(values) => values.len(),
        Column::Float(values) => values.len(),
        Column::Mixed { values, .. } => values.len(),
        Column::Str { .. } => return None,
    };
    let mut at: Box<dyn Iterator<Item = usize>> = match rows {
        Some(rows) => Box::new(rows.iter().map(|r| *r as usize)),
        None => Box::new(0..len),
    };
    // A value as a float, and whether the document holds an integer.
    let value = |row: usize| match column {
        Column::Int(values) => (values[row] as f64, Some(values[row])),
        Column::Float(values) => (values[row], None),
        Column::Mixed { values, ints } => (values[row], ints[row].then_some(values[row] as i64)),
        Column::Str { .. } => (0.0, None),
    };
    let first = at.next()?;
    let mut sum = ExactSum::default();
    let (mut min, mut max) = (first, first);
    for row in std::iter::once(first).chain(at) {
        let (x, int) = value(row);
        match int {
            Some(i) => sum.add_i64(i),
            None => sum.add(x),
        }
        if x < value(min).0 {
            min = row;
        }
        if x > value(max).0 {
            max = row;
        }
    }
    let extreme = |row: usize| match value(row) {
        (_, Some(i)) => Extreme::Int(i),
        (x, None) => Extreme::Float(x),
    };
    sum.settle();
    Some(Total {
        sum,
        min: extreme(min),
        max: extreme(max),
    })
}

impl Columns {
    fn bytes(&self) -> usize {
        let col = |c: &Column| match c {
            Column::Int(v) => v.len() * 8,
            Column::Float(v) => v.len() * 8,
            Column::Mixed { values, .. } => values.len() * 9,
            Column::Str { dict, codes } => {
                codes.len() * 4 + dict.iter().map(|s| s.len() + 24).sum::<usize>()
            }
        };
        self.time.len() * 12 + self.cols.iter().map(col).sum::<usize>()
    }
}

/// A block as it is stored.
#[derive(Serialize, Deserialize)]
struct Stored {
    lo: String,
    hi: String,
    rows: u64,
    fingerprint: u64,
    columns: Option<Columns>,
}

/// The first byte of a stored block: the layout of what follows.
const FORMAT: u8 = 2;

fn decode(bytes: &[u8]) -> Option<Stored> {
    match bytes.split_first() {
        Some((&FORMAT, rest)) => bincode::deserialize(rest).ok(),
        _ => None,
    }
}

/// A block of the directory: the keys `[lo, hi)`.
pub(crate) struct Entry {
    pub(crate) lo: String,
    pub(crate) hi: String,
    /// Documents in the range when it was sealed.
    pub(crate) rows: u64,
    fingerprint: u64,
    /// No columns: the range's documents are read.
    pub(crate) opaque: bool,
    /// The columns, when they are kept in memory.
    cached: Option<Arc<Columns>>,
    /// Counted and hashed again since this process read it from disk.
    verified: AtomicBool,
    /// The times of its rows, both included (none: `MAX`, `MIN`).
    pub(crate) min_time: i64,
    pub(crate) max_time: i64,
    /// A merge with its neighbours failed: not tried again.
    unmergeable: AtomicBool,
    /// Whether the block is on disk: every block is, once listed. (Keeping
    /// the small ones in memory until they were merged was tried, to write
    /// each point once instead of three times: it changed nothing that
    /// was measured, and a restart then had them to seal again.)
    stored: AtomicBool,
}

/// A block as it is stored, from one of the directory.
#[derive(Serialize)]
struct StoredRef<'a> {
    lo: &'a str,
    hi: &'a str,
    rows: u64,
    fingerprint: u64,
    columns: Option<&'a Columns>,
}

/// The bytes of a block: its format, then the block.
fn encoded<T: Serialize>(block: &T) -> DbResult<Vec<u8>> {
    let mut bytes = vec![FORMAT];
    bincode::serialize_into(&mut bytes, block)
        .map_err(|e| DbError::InternalError(format!("column block: {e}")))?;
    Ok(bytes)
}

/// A range being sealed: what was written since it was registered.
struct Pending {
    lo: String,
    /// The smallest key written at or after `lo` since then.
    written_from: Option<String>,
}

#[derive(Default)]
struct Inner {
    loaded: bool,
    blocks: BTreeMap<String, Arc<Entry>>,
    pending: Option<Pending>,
    /// Moved each time `blocks` changes.
    changes: u64,
    /// What a query reads, as of that many changes.
    view: Option<(u64, Arc<Vec<Segment>>)>,
}

/// The blocks of one collection.
pub(crate) struct BlockSet {
    state: Arc<KsState>,
    pub(crate) def: BlocksDef,
    inner: parking_lot::Mutex<Inner>,
    /// One seal at a time.
    sealing: parking_lot::Mutex<()>,
    /// Waiting for, or with, the thread that seals (`Sealer`).
    queued: AtomicBool,
    /// Documents covered by blocks that have columns.
    sealed_rows: AtomicU64,
    /// Documents covered by any block.
    covered_rows: AtomicU64,
    /// A write of keys generated at or after this instant (the milliseconds
    /// a UUIDv7 starts with) is past every block and every seal under way:
    /// it has nothing to announce, and takes no lock. `u64::MAX` while that
    /// cannot be said — before the directory is read, and for good once a
    /// key that is not such a UUID was met. See [`BlockSet::seal_next`].
    frontier: AtomicU64,
    /// Writes announced, whichever way.
    writes: AtomicU64,
    /// The value of `writes` at which a seal found no document past the
    /// last block (`u64::MAX`: never): while `writes` has not moved, a
    /// query has no documents to look for there.
    tail_clean_at: AtomicU64,
}

/// What a query reads, in key order.
pub(crate) enum Segment {
    Block(Arc<Entry>),
    /// Documents with a key in `[lo, hi)`; `None`: to the end.
    Documents(String, Option<String>),
}

/// Reads of a series' newest row answered from blocks, since the process
/// started (`block_scan::newest_from_blocks`): for tests, which must know
/// the blocks answered and not the index beside them.
pub static NEWEST_FROM_BLOCKS: AtomicU64 = AtomicU64::new(0);

/// The keys a write touched.
pub(crate) enum Wrote<'a> {
    Key(&'a str),
    /// Keys between the two, both included.
    Span(&'a str, &'a str),
    /// Unknown, or all of them.
    All,
}

/// What `GET …/blocks` reports.
#[derive(Debug, Serialize)]
pub struct BlocksStatus {
    pub time: String,
    pub fields: Vec<String>,
    pub blocks: usize,
    /// Blocks without columns: their documents are read.
    pub opaque_blocks: usize,
    pub sealed_documents: u64,
    pub documents: usize,
}

type Sets = DashMap<usize, (Arc<KsState>, Option<Arc<BlockSet>>)>;
/// By the collection's shared write state (its address, kept alive by the
/// entry): its blocks, or `None` once it is known to declare none.
static SETS: once_cell::sync::Lazy<Sets> = once_cell::sync::Lazy::new(DashMap::new);
static HELD: AtomicUsize = AtomicUsize::new(0);

/// `SOLIDB_COLUMN_BLOCKS_MB`: what the columns kept in memory may take
/// (default 1024). Past it a block's columns are read from disk each time.
fn budget() -> usize {
    static BUDGET: once_cell::sync::Lazy<usize> = once_cell::sync::Lazy::new(|| {
        std::env::var("SOLIDB_COLUMN_BLOCKS_MB")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(1024)
            << 20
    });
    *BUDGET
}

fn doc_key(key: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(DOC_PREFIX.len() + key.len());
    k.extend_from_slice(DOC_PREFIX.as_bytes());
    k.extend_from_slice(key.as_bytes());
    k
}

/// The end of the document keys: `doc;`, which follows every `doc:` key.
fn doc_end() -> Vec<u8> {
    let mut end = DOC_PREFIX.as_bytes().to_vec();
    *end.last_mut().expect("non-empty prefix") += 1;
    end
}

fn block_key(lo: &str) -> Vec<u8> {
    format!("{BLOCK_PREFIX}{lo}").into_bytes()
}

/// The instant a generated key (a UUIDv7 in its canonical form) was made
/// at, in milliseconds: its first 48 bits. Such keys order as their
/// instants do. `None` for any other key.
fn key_ms(key: &str) -> Option<u64> {
    let bytes = key.as_bytes();
    let hex = |b: &u8| b.is_ascii_digit() || (b'a'..=b'f').contains(b);
    if bytes.len() < 36
        || bytes[8] != b'-'
        || bytes[13] != b'-'
        || bytes[14] != b'7'
        || !bytes[..8].iter().all(hex)
        || !bytes[9..13].iter().all(hex)
    {
        return None;
    }
    let high = u64::from_str_radix(&key[..8], 16).ok()?;
    let low = u64::from_str_radix(&key[9..13], 16).ok()?;
    Some(high << 16 | low)
}

/// The same for a key on every run of every build: it is stored.
fn key_hash(key: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in key {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    // FNV's low bits are weak, and the hashes are combined by XOR.
    h ^= h >> 32;
    h.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// The set for the collection whose keys live in `ks`, read once.
fn set_of(db: &Arc<DB>, ks: &Keyspace) -> Option<Arc<BlockSet>> {
    let state = ks.state();
    let address = Arc::as_ptr(state) as usize;
    if let Some(found) = SETS.get(&address) {
        return found.1.clone();
    }
    let def = ks.handle(db).and_then(|cf| {
        db.get_ks(&cf, BLOCKS_KEY.as_bytes())
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice::<BlocksDef>(&bytes).ok())
    });
    let set = def.map(|def| Arc::new(BlockSet::new(state.clone(), def)));
    if SETS.len() % 256 == 255 {
        SETS.retain(|_, (state, _)| !state.is_dead());
    }
    SETS.insert(address, (state.clone(), set.clone()));
    set
}

impl BlockSet {
    fn new(state: Arc<KsState>, def: BlocksDef) -> Self {
        BlockSet {
            state,
            def,
            inner: parking_lot::Mutex::new(Inner::default()),
            sealing: parking_lot::Mutex::new(()),
            queued: AtomicBool::new(false),
            sealed_rows: AtomicU64::new(0),
            covered_rows: AtomicU64::new(0),
            frontier: AtomicU64::new(u64::MAX),
            writes: AtomicU64::new(0),
            tail_clean_at: AtomicU64::new(u64::MAX),
        }
    }

    fn entry(&self, stored: Stored, verified: bool, on_disk: bool) -> Arc<Entry> {
        let opaque = stored.columns.is_none();
        let (min_time, max_time) = stored
            .columns
            .as_ref()
            .map_or((i64::MAX, i64::MIN), |c| (c.min_time, c.max_time));
        self.covered_rows.fetch_add(stored.rows, Ordering::Relaxed);
        let cached = stored.columns.and_then(|columns| {
            let bytes = columns.bytes();
            // What is not on disk is kept whatever the budget says: there
            // is nowhere else to read it from.
            (HELD.fetch_add(bytes, Ordering::Relaxed) + bytes <= budget() || !on_disk)
                .then(|| Arc::new(columns))
                .or_else(|| {
                    HELD.fetch_sub(bytes, Ordering::Relaxed);
                    None
                })
        });
        if !opaque {
            self.sealed_rows.fetch_add(stored.rows, Ordering::Relaxed);
        }
        Arc::new(Entry {
            lo: stored.lo,
            hi: stored.hi,
            rows: stored.rows,
            fingerprint: stored.fingerprint,
            opaque,
            cached,
            verified: AtomicBool::new(verified),
            min_time,
            max_time,
            unmergeable: AtomicBool::new(false),
            stored: AtomicBool::new(on_disk),
        })
    }

    fn forget(&self, entry: &Entry) {
        self.covered_rows.fetch_sub(entry.rows, Ordering::Relaxed);
        if !entry.opaque {
            self.sealed_rows.fetch_sub(entry.rows, Ordering::Relaxed);
        }
        if let Some(columns) = &entry.cached {
            HELD.fetch_sub(columns.bytes(), Ordering::Relaxed);
        }
    }

    /// Read the directory from disk, the first time it is needed. Blocks
    /// come back unverified.
    fn load(&self, inner: &mut Inner, coll: &Collection) {
        if inner.loaded {
            return;
        }
        inner.loaded = true;
        let Some(cf) = coll.ks.handle(&coll.db) else {
            return;
        };
        let lower = BLOCK_PREFIX.as_bytes().to_vec();
        let mut upper = lower.clone();
        *upper.last_mut().expect("non-empty prefix") += 1;
        let mut found = Vec::new();
        let _ = coll
            .db
            .for_each_ks(&cf, &lower, &upper, 0, None, |_, value| {
                // A block of another format is not read; sealing its range
                // again writes over it.
                if let Some(stored) = decode(value) {
                    found.push(stored);
                }
                true
            });
        // Past every block: after the instant the last of them ends in,
        // when all of them are bounded by generated keys.
        let mut frontier = Some(0u64);
        for stored in found {
            let bounded = (stored.lo.is_empty() || key_ms(&stored.lo).is_some())
                .then(|| key_ms(&stored.hi))
                .flatten();
            frontier = frontier.zip(bounded).map(|(f, hi)| f.max(hi + 1));
            let entry = self.entry(stored, false, true);
            inner.blocks.insert(entry.lo.clone(), entry);
        }
        self.frontier
            .store(frontier.unwrap_or(u64::MAX), Ordering::Release);
        inner.changes += 1;
    }

    /// Write the blocks that are in memory only: an explicit seal is meant
    /// to last, and a restart reads what is on disk.
    fn store_all(&self, coll: &Collection) -> DbResult<()> {
        let inner = self.inner.lock();
        let Some(cf) = coll.ks.handle(&coll.db) else {
            return Ok(());
        };
        for entry in inner
            .blocks
            .values()
            .filter(|e| !e.stored.load(Ordering::Acquire))
        {
            let block = StoredRef {
                lo: &entry.lo,
                hi: &entry.hi,
                rows: entry.rows,
                fingerprint: entry.fingerprint,
                columns: entry.cached.as_deref(),
            };
            // A block with columns that are nowhere cannot be written.
            if entry.opaque || block.columns.is_some() {
                coll.db
                    .put_ks(&cf, block_key(&entry.lo), encoded(&block)?)
                    .map_err(|e| DbError::InternalError(format!("column block: {e}")))?;
                entry.stored.store(true, Ordering::Release);
            }
        }
        Ok(())
    }

    fn delete_stored(&self, coll: &Collection, lo: &str) {
        if let Some(cf) = coll.ks.handle(&coll.db) {
            let _ = coll.db.delete_ks(&cf, block_key(lo));
        }
    }

    /// A write to `keys` was committed: the blocks holding one of them go.
    pub(crate) fn wrote(&self, coll: &Collection, keys: Wrote<'_>) {
        self.writes.fetch_add(1, Ordering::AcqRel);
        // New points, the usual write: past everything sealed or being
        // sealed, by the instant their keys were generated at.
        if let Wrote::Key(first) | Wrote::Span(first, _) = keys {
            if key_ms(first).is_some_and(|ms| ms >= self.frontier.load(Ordering::Acquire)) {
                return;
            }
        }
        let mut inner = self.inner.lock();
        let (from, to) = match keys {
            Wrote::Key(k) => (k, Some(k)),
            Wrote::Span(a, b) => (a, Some(b)),
            Wrote::All => ("", None),
        };
        if let Some(pending) = &mut inner.pending {
            if to.is_none_or(|to| to >= pending.lo.as_str())
                && pending
                    .written_from
                    .as_deref()
                    .is_none_or(|seen| from < seen)
            {
                pending.written_from = Some(from.to_string());
            }
        }
        // Before the directory is read, nothing is known of the blocks on
        // disk: they are checked against the keys when it is.
        if !inner.loaded {
            return;
        }
        // The blocks that start at or before `to` and end after `from`.
        let hit: Vec<String> = match to {
            None => inner.blocks.keys().cloned().collect(),
            Some(to) => inner
                .blocks
                .range::<str, _>((std::ops::Bound::Unbounded, std::ops::Bound::Included(to)))
                .rev()
                .take_while(|(_, entry)| entry.hi.as_str() > from)
                .map(|(lo, _)| lo.clone())
                .collect(),
        };
        for lo in hit {
            if let Some(entry) = inner.blocks.remove(&lo) {
                self.forget(&entry);
                if entry.stored.load(Ordering::Acquire) {
                    self.delete_stored(coll, &lo);
                }
                inner.changes += 1;
            }
        }
    }

    /// What a query reads, in key order: each block, and the documents no
    /// block covers.
    pub(crate) fn segments(&self, coll: &Collection) -> Arc<Vec<Segment>> {
        let mut inner = self.inner.lock();
        self.load(&mut inner, coll);
        // Built once per state of the directory: a query only shares it.
        if let Some((blocks, view)) = &inner.view {
            if *blocks == inner.changes {
                return view.clone();
            }
        }
        let mut out = Vec::with_capacity(inner.blocks.len() + 1);
        let mut at = "";
        for entry in inner.blocks.values() {
            if entry.lo.as_str() > at {
                out.push(Segment::Documents(at.to_string(), Some(entry.lo.clone())));
            }
            at = entry.hi.as_str();
            out.push(Segment::Block(entry.clone()));
        }
        out.push(Segment::Documents(at.to_string(), None));
        let view = Arc::new(out);
        inner.view = Some((inner.changes, view.clone()));
        view
    }

    /// Whether no document lies past the last block: a seal found none, and
    /// no write was announced since.
    pub(crate) fn tail_is_empty(&self) -> bool {
        self.tail_clean_at.load(Ordering::Acquire) == self.writes.load(Ordering::Acquire)
    }

    /// Documents no block with columns covers.
    pub(crate) fn unsealed(&self, coll: &Collection) -> u64 {
        (coll.count() as u64).saturating_sub(self.sealed_rows.load(Ordering::Relaxed))
    }

    /// Documents no block covers at all: what is left to seal.
    fn uncovered(&self, coll: &Collection) -> u64 {
        (coll.count() as u64).saturating_sub(self.covered_rows.load(Ordering::Relaxed))
    }

    /// Whether `entry` still describes the keys of its range. A block this
    /// process sealed, or checked once, does: every write since was
    /// announced. One read from disk is counted and hashed first, and
    /// deleted when it does not match.
    pub(crate) fn verified(&self, coll: &Collection, entry: &Arc<Entry>) -> bool {
        if entry.verified.load(Ordering::Acquire) {
            return true;
        }
        let (mut rows, mut fingerprint) = (0u64, 0u64);
        let read = for_each_doc(coll, &entry.lo, Some(&entry.hi), None, |key, _| {
            rows += 1;
            fingerprint ^= key_hash(key);
            true
        });
        if read.is_ok() && rows == entry.rows && fingerprint == entry.fingerprint {
            entry.verified.store(true, Ordering::Release);
            return true;
        }
        let mut inner = self.inner.lock();
        if inner
            .blocks
            .get(&entry.lo)
            .is_some_and(|e| Arc::ptr_eq(e, entry))
        {
            inner.blocks.remove(&entry.lo);
            inner.changes += 1;
            self.forget(entry);
            self.delete_stored(coll, &entry.lo);
        }
        false
    }

    /// The columns of `entry` if they are in memory.
    pub(crate) fn cached_columns(&self, entry: &Entry) -> Option<Arc<Columns>> {
        entry.cached.clone()
    }

    /// The columns of `entry`: from memory, or read again.
    pub(crate) fn columns(&self, coll: &Collection, entry: &Entry) -> Option<Arc<Columns>> {
        if let Some(columns) = &entry.cached {
            return Some(columns.clone());
        }
        let cf = coll.ks.handle(&coll.db)?;
        let bytes = coll.db.get_ks(&cf, block_key(&entry.lo)).ok().flatten()?;
        decode(&bytes)?.columns.map(Arc::new)
    }

    /// Seal the first range no block covers. `whole` seals a tail shorter
    /// than a block too, and several blocks at once. Returns how many
    /// blocks were added.
    fn seal_next(&self, coll: &Collection, whole: bool, at_once: usize) -> DbResult<usize> {
        // The range: from the end of the blocks that follow one another
        // from the start, to the next block if there is one.
        let (lo, bound) = {
            let mut inner = self.inner.lock();
            self.load(&mut inner, coll);
            let mut at = String::new();
            let mut bound = None;
            for entry in inner.blocks.values() {
                if entry.lo > at {
                    bound = Some(entry.lo.clone());
                    break;
                }
                at = entry.hi.clone();
            }
            inner.pending = Some(Pending {
                lo: at.clone(),
                written_from: None,
            });
            (at, bound)
        };
        // While every key is a generated one, the block will hold only keys
        // generated before now, and a write of keys generated from now on
        // need not announce itself (`frontier`): raised before the snapshot
        // is taken, so that a write that skips the announcement is either
        // in the snapshot or past the block.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        // Read before the snapshot, as the frontier is raised before it: a
        // write the snapshot does not hold moves it afterwards.
        let writes = self.writes.load(Ordering::Acquire);
        let generated = self.frontier.load(Ordering::Acquire) != u64::MAX;
        if generated {
            self.frontier.fetch_max(now, Ordering::AcqRel);
        }
        let sealed = self.seal_range(
            coll,
            &lo,
            bound.as_deref(),
            whole,
            generated.then_some(now),
            writes,
            at_once,
        );
        let mut inner = self.inner.lock();
        let pending = inner.pending.take();
        let blocks = sealed?;
        if blocks.is_empty() || self.state.is_dead() {
            return Ok(0);
        }
        // The blocks follow one another: the first that holds a key written
        // since the range was registered goes, and those after it.
        let written_from = pending.and_then(|p| p.written_from);
        let mut kept = 0;
        for stored in blocks {
            if written_from
                .as_deref()
                .is_some_and(|from| from < stored.hi.as_str())
            {
                break;
            }
            // Written before it is listed: a restart reads what is on disk.
            {
                let cf = coll.ks.handle(&coll.db).ok_or_else(|| {
                    DbError::CollectionNotFound(format!("{} (dropped while sealing)", coll.name))
                })?;
                coll.db
                    .put_ks(&cf, block_key(&stored.lo), encoded(&stored)?)
                    .map_err(|e| DbError::InternalError(format!("column block: {e}")))?;
            }
            let entry = self.entry(stored, true, true);
            inner.blocks.insert(entry.lo.clone(), entry);
            inner.changes += 1;
            kept += 1;
        }
        Ok(kept)
    }

    /// Merge the first run of small adjacent blocks into one: the points of
    /// a series are then in fewer blocks. Returns whether it did. The caller
    /// holds `sealing`.
    fn merge_next(&self, coll: &Collection) -> bool {
        let small = |e: &Entry| {
            !e.opaque
                && e.rows as usize <= BLOCK_ROWS / MERGE_FAN
                && e.verified.load(Ordering::Acquire)
                && !e.unmergeable.load(Ordering::Acquire)
        };
        // The run: adjacent, each small, together at most a block.
        let run: Vec<Arc<Entry>> = {
            let inner = self.inner.lock();
            let mut run: Vec<Arc<Entry>> = Vec::new();
            let mut rows = 0usize;
            for entry in inner.blocks.values() {
                let follows = run.last().is_some_and(|last| last.hi == entry.lo);
                if !small(entry) || !follows || rows + entry.rows as usize > BLOCK_ROWS {
                    if run.len() >= MERGE_FAN {
                        break;
                    }
                    run.clear();
                    rows = 0;
                }
                if small(entry) {
                    rows += entry.rows as usize;
                    run.push(entry.clone());
                }
            }
            if run.len() < MERGE_FAN {
                run.clear();
            }
            run
        };
        let (Some(first), Some(last)) = (run.first(), run.last()) else {
            return false;
        };
        let give_up = || {
            first.unmergeable.store(true, Ordering::Release);
            true
        };
        // Their rows, as one block's.
        let mut parts = Vec::with_capacity(run.len());
        for entry in &run {
            let Some(columns) = self.columns(coll, entry) else {
                return give_up();
            };
            parts.push(columns);
        }
        let Some(columns) = merged(&parts) else {
            return give_up();
        };
        let stored = Stored {
            lo: first.lo.clone(),
            hi: last.hi.clone(),
            rows: run.iter().map(|e| e.rows).sum(),
            fingerprint: run.iter().fold(0, |f, e| f ^ e.fingerprint),
            columns: Some(columns),
        };
        let persist = true;
        let bytes = match encoded(&stored) {
            Ok(bytes) => Some(bytes),
            Err(_) => return give_up(),
        };
        let Some(cf) = coll.ks.handle(&coll.db) else {
            return false;
        };
        // One write replaces them all, with none of them written to since.
        let mut inner = self.inner.lock();
        let untouched = run.iter().all(|entry| {
            inner
                .blocks
                .get(&entry.lo)
                .is_some_and(|held| Arc::ptr_eq(held, entry))
        });
        if !untouched {
            return true;
        }
        if let Some(bytes) = bytes {
            let mut batch = super::kv::KvBatch::default();
            for entry in run[1..].iter().filter(|e| e.stored.load(Ordering::Acquire)) {
                batch.delete_ks(&cf, block_key(&entry.lo));
            }
            batch.put_ks(&cf, block_key(&stored.lo), bytes);
            if coll.db.write(batch).is_err() {
                return false;
            }
        }
        for entry in &run {
            inner.blocks.remove(&entry.lo);
            self.forget(entry);
        }
        let entry = self.entry(stored, true, persist);
        inner.blocks.insert(entry.lo.clone(), entry);
        inner.changes += 1;
        true
    }

    /// Build the block of the first documents of `[lo, bound)`.
    /// `None` when the range is an open tail too short to seal.
    /// The blocks of the documents from `lo` to `bound` (the tail's end
    /// without one): at most `at_once` of them, each of up to
    /// [`BLOCK_ROWS`] documents, in key order and following one another.
    /// Read in one pass, then built on as many threads: what a block costs
    /// is decoding its documents and arranging them, and no block needs
    /// another's.
    #[allow(clippy::too_many_arguments)]
    fn seal_range(
        &self,
        coll: &Collection,
        lo: &str,
        bound: Option<&str>,
        whole: bool,
        before: Option<u64>,
        writes: u64,
        at_once: usize,
    ) -> DbResult<Vec<Stored>> {
        let fields = self.def.projection();
        let snapshot = coll.db.snapshot();
        let most = BLOCK_ROWS * at_once.max(1);
        let mut keys: Vec<String> = Vec::with_capacity(BLOCK_ROWS + 1);
        // The stored documents end to end, and where each ends; `None`
        // once a key is not text.
        let mut docs: Option<(Vec<u8>, Vec<usize>)> = Some((Vec::new(), Vec::new()));
        for_each_doc(coll, lo, bound, snapshot.as_ref(), |key, bytes| {
            let Ok(text) = std::str::from_utf8(&key[DOC_PREFIX.len()..]) else {
                docs = None;
                return true;
            };
            keys.push(text.to_string());
            if keys.len() > most {
                return false;
            }
            if let Some((all, ends)) = &mut docs {
                all.extend_from_slice(bytes);
                ends.push(all.len());
            }
            true
        })?;
        // Nothing past the last block, as of `writes` announcements.
        if bound.is_none() && keys.is_empty() {
            self.tail_clean_at.store(writes, Ordering::Release);
            return Ok(Vec::new());
        }
        // With `before`, writes of keys generated from that instant on do
        // not announce themselves: the blocks must end before them, and
        // every key in sight must be a generated one for that to mean
        // anything. One that is not ends the arrangement, and this seal.
        let mut clamped = false;
        if let Some(before) = before {
            let foreign = keys.iter().any(|key| key_ms(key).is_none())
                || bound.is_some_and(|bound| key_ms(bound).is_none());
            if foreign {
                self.frontier.store(u64::MAX, Ordering::Release);
                return Ok(Vec::new());
            }
            if let Some(at) = keys
                .iter()
                .position(|key| key_ms(key).is_some_and(|ms| ms >= before))
            {
                keys.truncate(at);
                clamped = true;
            }
        }
        // The last block ends where the next seal will start: at the first
        // key left out, at the bound, or just past the last key of the
        // tail. A write leaves the newest keys of the tail out: a late
        // write is likely to fall among them.
        let hi = if clamped {
            match keys.last() {
                Some(last) => format!("{last}\u{0}"),
                None => return Ok(Vec::new()),
            }
        } else if keys.len() > most {
            keys.pop().expect("more than asked")
        } else if let Some(bound) = bound {
            bound.to_string()
        } else if whole && !keys.is_empty() {
            format!("{}\u{0}", keys.last().expect("not empty"))
        } else if keys.len() >= SEAL_ROWS + TAIL_KEPT {
            let kept = keys.len() - TAIL_KEPT;
            keys.split_off(kept).swap_remove(0)
        } else {
            return Ok(Vec::new());
        };

        // One block per `BLOCK_ROWS` keys, each ending at the next one's
        // first key.
        let starts: Vec<usize> = (0..keys.len().max(1)).step_by(BLOCK_ROWS).collect();
        let build = |n: usize| -> Stored {
            let from = starts[n];
            let to = starts.get(n + 1).copied().unwrap_or(keys.len());
            let block_lo = if n == 0 { lo } else { keys[from].as_str() };
            let block_hi = starts
                .get(n + 1)
                .map_or(hi.as_str(), |next| keys[*next].as_str());
            let fingerprint = keys[from..to]
                .iter()
                .fold(0, |f, key| f ^ key_hash(&doc_key(key)));
            let columns = docs.as_ref().and_then(|(all, ends)| {
                let doc = |i: usize| {
                    let start = if i == 0 { 0 } else { ends[i - 1] };
                    &all[start..ends[i]]
                };
                // Straight from the stored bytes when every document
                // holds plain numbers and strings there; else through
                // decoded documents, which decide what a block can hold.
                columns_from_bytes((from..to).map(doc), &fields).or_else(|| {
                    (from..to)
                        .map(|i| row_of(doc(i), &fields))
                        .collect::<Option<Vec<_>>>()
                        .and_then(columns_of)
                })
            });
            Stored {
                lo: block_lo.to_string(),
                hi: block_hi.to_string(),
                rows: (to - from) as u64,
                fingerprint,
                columns,
            }
        };
        if starts.len() == 1 {
            return Ok(vec![build(0)]);
        }
        let next = std::sync::atomic::AtomicUsize::new(0);
        let built: parking_lot::Mutex<Vec<(usize, Stored)>> = parking_lot::Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..starts.len().min(at_once) {
                scope.spawn(|| loop {
                    let n = next.fetch_add(1, Ordering::Relaxed);
                    if n >= starts.len() {
                        return;
                    }
                    let block = build(n);
                    built.lock().push((n, block));
                });
            }
        });
        let mut built = built.into_inner();
        // (A thread that died left its block out: nothing after it either.)
        built.sort_by_key(|(n, _)| *n);
        Ok(built
            .into_iter()
            .enumerate()
            .take_while(|(at, (n, _))| at == n)
            .map(|(_, (_, block))| block)
            .collect())
    }
}

/// Threads the seal of a whole collection builds blocks on: the CPUs, at
/// most 8.
fn seal_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get().min(8))
}

/// Each document with a key in `[lo, hi)`, as its `doc:` key and stored
/// bytes, until `f` returns false.
pub(crate) fn for_each_doc(
    coll: &Collection,
    lo: &str,
    hi: Option<&str>,
    snapshot: Option<&super::kv::KvSnapshot<'_>>,
    f: impl FnMut(&[u8], &[u8]) -> bool,
) -> DbResult<()> {
    let Some(cf) = coll.ks.handle(&coll.db) else {
        return Ok(());
    };
    let lower = doc_key(lo);
    let upper = hi.map_or_else(doc_end, doc_key);
    if lower >= upper {
        return Ok(());
    }
    coll.db
        .for_each_ks(&cf, &lower, &upper, 256 * 1024, snapshot, f)
        .map_err(|e| DbError::InternalError(format!("reading documents: {e}")))
}

/// The time and fields of one stored document, or `None` when it does not
/// decode.
fn row_of(bytes: &[u8], fields: &[String]) -> Option<Vec<Value>> {
    let doc = deserialize_doc_projected(bytes, fields).ok()?;
    let object = doc.as_object()?;
    Some(
        fields
            .iter()
            .map(|f| object.get(f).cloned().unwrap_or(Value::Null))
            .collect(),
    )
}

/// A field of a stored document that a column takes as it is.
#[derive(Clone, Copy)]
enum Cell<'a> {
    Int(i64),
    Float(f64),
    Str(&'a str),
}

/// The MessagePack value `raw` as a cell: an integer an `i64` holds, a
/// finite 64-bit float, a string. `None` for anything else, which is left
/// to the decoder.
fn cell_of(raw: &[u8]) -> Option<Cell<'_>> {
    let (&marker, rest) = raw.split_first()?;
    fn text(bytes: Option<&[u8]>) -> Option<Cell<'_>> {
        std::str::from_utf8(bytes?).ok().map(Cell::Str)
    }
    Some(match marker {
        0x00..=0x7f => Cell::Int(i64::from(marker)),
        0xe0..=0xff => Cell::Int(i64::from(marker as i8)),
        0xcc => Cell::Int(i64::from(*rest.first()?)),
        0xcd => Cell::Int(i64::from(u16::from_be_bytes(
            rest.get(..2)?.try_into().ok()?,
        ))),
        0xce => Cell::Int(i64::from(u32::from_be_bytes(
            rest.get(..4)?.try_into().ok()?,
        ))),
        0xcf => Cell::Int(i64::try_from(u64::from_be_bytes(rest.get(..8)?.try_into().ok()?)).ok()?),
        0xd0 => Cell::Int(i64::from(*rest.first()? as i8)),
        0xd1 => Cell::Int(i64::from(i16::from_be_bytes(
            rest.get(..2)?.try_into().ok()?,
        ))),
        0xd2 => Cell::Int(i64::from(i32::from_be_bytes(
            rest.get(..4)?.try_into().ok()?,
        ))),
        0xd3 => Cell::Int(i64::from_be_bytes(rest.get(..8)?.try_into().ok()?)),
        0xcb => {
            let f = f64::from_be_bytes(rest.get(..8)?.try_into().ok()?);
            if !f.is_finite() {
                return None;
            }
            Cell::Float(f)
        }
        0xa0..=0xbf => text(rest.get(..usize::from(marker & 0x1f)))?,
        0xd9 => text(rest.get(1..1 + usize::from(*rest.first()?)))?,
        0xda => {
            let len = usize::from(u16::from_be_bytes(rest.get(..2)?.try_into().ok()?));
            text(rest.get(2..2 + len))?
        }
        0xdb => {
            let len = u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize;
            text(rest.get(4..4usize.checked_add(len)?))?
        }
        _ => return None,
    })
}

/// [`columns_of`] for stored documents, their fields read where they lie:
/// what sealing a block costs is mostly decoding its documents, and a
/// decoded document is a map of owned values built to be read once. `None`
/// whenever a document is not the plain case — an older format, a field
/// missing or of another kind, a system field asked for — and then the
/// caller decodes: the two must give the same columns, which a test holds
/// them to.
fn columns_from_bytes<'a>(
    docs: impl Iterator<Item = &'a [u8]>,
    fields: &[String],
) -> Option<Columns> {
    let width = fields.len();
    if width == 0 || fields.iter().any(|f| f.starts_with('_')) {
        return None;
    }
    let mut time: Vec<i64> = Vec::new();
    let mut cells: Vec<Vec<Cell<'a>>> = vec![Vec::new(); width - 1];
    let mut found: Vec<Option<Cell<'a>>> = vec![None; width];
    for bytes in docs {
        let doc = super::serializer::StoredDocRef::parse(bytes)?;
        found.iter_mut().for_each(|f| *f = None);
        let mut plain = true;
        super::raw_rows::visit_map(doc.data, |key, value| {
            // (A field given twice: the last one, as a decoded map keeps.)
            if let Some(at) = fields.iter().position(|f| f.as_bytes() == key) {
                found[at] = cell_of(value);
                plain &= found[at].is_some();
            }
        })?;
        if !plain {
            return None;
        }
        let Cell::Int(at) = found[0]? else {
            return None;
        };
        time.push(at);
        for (column, cell) in cells.iter_mut().zip(&found[1..]) {
            column.push((*cell)?);
        }
    }
    if time.is_empty() {
        return None;
    }
    let cols = cells
        .iter()
        .map(|column| {
            let ints = column.iter().filter(|c| matches!(c, Cell::Int(_))).count();
            let strs = column.iter().filter(|c| matches!(c, Cell::Str(_))).count();
            let number = |c: &Cell<'_>| match *c {
                Cell::Int(i) => i as f64,
                Cell::Float(f) => f,
                Cell::Str(_) => 0.0,
            };
            if strs == column.len() {
                let text = |c: &Cell<'a>| match *c {
                    Cell::Str(s) => s,
                    _ => "",
                };
                // The distinct strings first (a series has thousands of
                // points for one name), then their ranks: a row's code is
                // its string's rank in the sorted dictionary.
                let mut ranks: std::collections::HashMap<&str, u32, foldhash::fast::RandomState> =
                    std::collections::HashMap::default();
                for c in column {
                    ranks.entry(text(c)).or_insert(0);
                }
                let mut dict: Vec<&str> = ranks.keys().copied().collect();
                dict.sort_unstable();
                for (rank, s) in dict.iter().enumerate() {
                    if let Some(held) = ranks.get_mut(s) {
                        *held = rank as u32;
                    }
                }
                let codes = column
                    .iter()
                    .map(|c| ranks.get(text(c)).copied().unwrap_or(0))
                    .collect();
                Some(Column::Str {
                    dict: dict.into_iter().map(str::to_string).collect(),
                    codes,
                })
            } else if strs > 0 {
                // Strings among numbers: for the decoder to refuse.
                None
            } else if ints == column.len() {
                Some(Column::Int(
                    column
                        .iter()
                        .map(|c| match *c {
                            Cell::Int(i) => i,
                            _ => 0,
                        })
                        .collect(),
                ))
            } else if ints == 0 {
                Some(Column::Float(column.iter().map(number).collect()))
            } else {
                // Integers among floats are kept as floats, which holds
                // those up to 2^53 exactly.
                let fits = |c: &Cell<'_>| match *c {
                    Cell::Int(i) => i.unsigned_abs() <= EXACT_INT as u64,
                    _ => true,
                };
                column.iter().all(fits).then(|| Column::Mixed {
                    values: column.iter().map(number).collect(),
                    ints: column.iter().map(|c| matches!(c, Cell::Int(_))).collect(),
                })
            }
        })
        .collect::<Option<Vec<Column>>>()?;
    Some(arrange(time, cols))
}

/// The rows as columns, ordered by the string columns then by time; `None`
/// when a row does not fit (see the module's notes).
fn columns_of(rows: Vec<Vec<Value>>) -> Option<Columns> {
    let width = rows.first().map_or(0, Vec::len);
    if width == 0 {
        return Some(Columns {
            min_time: i64::MAX,
            max_time: i64::MIN,
            time: Vec::new(),
            cols: Vec::new(),
            totals: Vec::new(),
            by_time: Vec::new(),
            chunks: Vec::new(),
        });
    }
    let mut time = Vec::with_capacity(rows.len());
    for row in &rows {
        time.push(row[0].as_number().filter(|n| n.is_i64())?.as_i64()?);
    }
    #[derive(PartialEq, Clone, Copy)]
    enum Kind {
        Int,
        Float,
        Mixed,
        Str,
    }
    let kind = |v: &Value| match v {
        Value::Number(n) if n.is_i64() => Some(Kind::Int),
        Value::Number(n) if n.is_f64() => Some(Kind::Float),
        Value::String(_) => Some(Kind::Str),
        _ => None,
    };
    // Integers among floats are kept as floats, which holds those up to
    // 2^53 exactly.
    let fits = |v: &Value| v.as_i64().is_none_or(|i| i.abs() <= EXACT_INT);
    let mut kinds = Vec::with_capacity(width - 1);
    for c in 1..width {
        let mut held = kind(&rows[0][c])?;
        for row in &rows[1..] {
            let this = kind(&row[c])?;
            if this != held {
                if this == Kind::Str || held == Kind::Str {
                    return None;
                }
                held = Kind::Mixed;
            }
        }
        if held == Kind::Mixed && !rows.iter().all(|row| fits(&row[c])) {
            return None;
        }
        kinds.push(held);
    }
    // A row's code is its string's rank in the sorted dictionary.
    let cols = kinds
        .iter()
        .enumerate()
        .map(|(i, kind)| {
            let values = || rows.iter().map(|row| &row[i + 1]);
            match kind {
                Kind::Int => Column::Int(values().filter_map(Value::as_i64).collect()),
                Kind::Float => Column::Float(values().filter_map(Value::as_f64).collect()),
                Kind::Mixed => Column::Mixed {
                    values: values().filter_map(Value::as_f64).collect(),
                    ints: values().map(Value::is_i64).collect(),
                },
                Kind::Str => {
                    let mut dict: Vec<String> = values()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect();
                    dict.sort_unstable();
                    dict.dedup();
                    let codes = values()
                        .map(|v| {
                            let s = v.as_str().unwrap_or_default();
                            dict.binary_search_by(|d| d.as_str().cmp(s)).unwrap_or(0) as u32
                        })
                        .collect();
                    Column::Str { dict, codes }
                }
            }
        })
        .collect();
    Some(arrange(time, cols))
}

/// The rows of several blocks as one block's; `None` when a field is not of
/// one kind in all of them.
fn merged(parts: &[Arc<Columns>]) -> Option<Columns> {
    let first = parts.first()?;
    let mut time = Vec::with_capacity(parts.iter().map(|p| p.time.len()).sum());
    for part in parts {
        time.extend_from_slice(&part.time);
    }
    let mut cols = Vec::with_capacity(first.cols.len());
    for (at, column) in first.cols.iter().enumerate() {
        let number = |c: &Column| !matches!(c, Column::Str { .. });
        let all_of = |wanted: fn(&Column) -> bool| {
            parts
                .iter()
                .all(|part| part.cols.get(at).is_some_and(wanted))
        };
        cols.push(match column {
            _ if number(column) && all_of(|c| matches!(c, Column::Int(_))) => {
                let mut all = Vec::with_capacity(time.len());
                for part in parts {
                    if let Column::Int(values) = &part.cols[at] {
                        all.extend_from_slice(values);
                    }
                }
                Column::Int(all)
            }
            _ if number(column) && all_of(|c| matches!(c, Column::Float(_))) => {
                let mut all = Vec::with_capacity(time.len());
                for part in parts {
                    if let Column::Float(values) = &part.cols[at] {
                        all.extend_from_slice(values);
                    }
                }
                Column::Float(all)
            }
            // Integers in one, floats in another: both, as floats.
            Column::Int(_) | Column::Float(_) | Column::Mixed { .. } => {
                let mut all = Vec::with_capacity(time.len());
                let mut whole = Vec::with_capacity(time.len());
                for part in parts {
                    match part.cols.get(at)? {
                        Column::Int(values) => {
                            if values.iter().any(|v| v.abs() > EXACT_INT) {
                                return None;
                            }
                            all.extend(values.iter().map(|v| *v as f64));
                            whole.extend(std::iter::repeat_n(true, values.len()));
                        }
                        Column::Float(values) => {
                            all.extend_from_slice(values);
                            whole.extend(std::iter::repeat_n(false, values.len()));
                        }
                        Column::Mixed { values, ints } => {
                            all.extend_from_slice(values);
                            whole.extend_from_slice(ints);
                        }
                        Column::Str { .. } => return None,
                    }
                }
                Column::Mixed {
                    values: all,
                    ints: whole,
                }
            }
            Column::Str { .. } => {
                let mut dict: Vec<String> = Vec::new();
                for part in parts {
                    let Column::Str { dict: theirs, .. } = part.cols.get(at)? else {
                        return None;
                    };
                    dict.extend(theirs.iter().cloned());
                }
                dict.sort_unstable();
                dict.dedup();
                let mut all = Vec::with_capacity(time.len());
                for part in parts {
                    let Column::Str {
                        dict: theirs,
                        codes,
                    } = &part.cols[at]
                    else {
                        return None;
                    };
                    // Where each of the part's strings is in the merged
                    // dictionary.
                    let moved: Vec<u32> = theirs
                        .iter()
                        .map(|s| dict.binary_search(s).unwrap_or(0) as u32)
                        .collect();
                    all.extend(codes.iter().map(|c| moved[*c as usize]));
                }
                Column::Str { dict, codes: all }
            }
        });
    }
    Some(arrange(time, cols))
}

/// Rows in any order as a block's columns: ordered by the string columns
/// then by time, with what is known of each column and of each chunk of
/// the rows in time order.
fn arrange(time: Vec<i64>, cols: Vec<Column>) -> Columns {
    let mut order: Vec<u32> = (0..time.len() as u32).collect();
    order.sort_by(|&a, &b| {
        let (a, b) = (a as usize, b as usize);
        cols.iter()
            .filter_map(|column| match column {
                Column::Str { codes, .. } => Some(codes[a].cmp(&codes[b])),
                _ => None,
            })
            .find(|o| o.is_ne())
            .unwrap_or_else(|| time[a].cmp(&time[b]).then(a.cmp(&b)))
    });
    let rows = || order.iter().map(|r| *r as usize);
    let cols: Vec<Column> = cols
        .into_iter()
        .map(|column| match column {
            Column::Int(values) => Column::Int(rows().map(|r| values[r]).collect()),
            Column::Float(values) => Column::Float(rows().map(|r| values[r]).collect()),
            Column::Mixed { values, ints } => Column::Mixed {
                values: rows().map(|r| values[r]).collect(),
                ints: rows().map(|r| ints[r]).collect(),
            },
            Column::Str { dict, codes } => Column::Str {
                dict,
                codes: rows().map(|r| codes[r]).collect(),
            },
        })
        .collect();
    let time: Vec<i64> = rows().map(|r| time[r]).collect();
    let mut by_time: Vec<u32> = (0..time.len() as u32).collect();
    by_time.sort_by_key(|r| (time[*r as usize], *r));
    let chunks = cols
        .iter()
        .map(|column| {
            by_time
                .chunks(CHUNK_ROWS)
                .map(|rows| total_of(column, Some(rows)))
                .collect()
        })
        .collect();
    Columns {
        totals: cols.iter().map(|column| total_of(column, None)).collect(),
        min_time: time.iter().copied().min().unwrap_or(i64::MAX),
        max_time: time.iter().copied().max().unwrap_or(i64::MIN),
        time,
        cols,
        by_time,
        chunks,
    }
}

impl Collection {
    /// The blocks of this collection, when it declares some.
    pub(crate) fn column_blocks(&self) -> Option<Arc<BlockSet>> {
        if self.collection_type.read().as_str() != "timeseries" {
            return None;
        }
        set_of(&self.db, &self.ks)
    }

    /// The fields this collection keeps in blocks.
    pub fn column_blocks_definition(&self) -> Option<BlocksDef> {
        self.column_blocks().map(|set| set.def.clone())
    }

    /// Declare (or replace) the fields kept in blocks. The blocks of an
    /// earlier definition are deleted; [`Self::seal_column_blocks`] builds
    /// the new ones.
    pub fn set_column_blocks(&self, def: BlocksDef) -> DbResult<()> {
        def.check()?;
        if self.collection_type.read().as_str() != "timeseries" {
            return Err(DbError::BadRequest(format!(
                "'{}' is not a timeseries collection: column blocks rely on documents that \
                 are never updated",
                self.name
            )));
        }
        self.replace_column_blocks(Some(def))
    }

    /// Stop keeping blocks, and delete them.
    pub fn drop_column_blocks(&self) -> DbResult<()> {
        self.replace_column_blocks(None)
    }

    fn replace_column_blocks(&self, def: Option<BlocksDef>) -> DbResult<()> {
        let cf = self.ks.handle(&self.db).ok_or_else(|| {
            DbError::CollectionNotFound(format!("{} (dropped mid-operation)", self.name))
        })?;
        let failed = |e| DbError::InternalError(format!("column blocks: {e}"));
        let state = self.ks.state();
        let address = Arc::as_ptr(state) as usize;
        // No seal of the definition being replaced may finish after its
        // blocks are deleted.
        let old = set_of(&self.db, &self.ks);
        let _sealing = old.as_ref().map(|set| set.sealing.lock());
        if let Some(old) = &old {
            old.wrote(self, Wrote::All);
            let mut inner = old.inner.lock();
            old.load(&mut inner, self);
            inner.changes += 1;
            for (lo, entry) in std::mem::take(&mut inner.blocks) {
                old.forget(&entry);
                old.delete_stored(self, &lo);
            }
        }
        match &def {
            Some(def) => {
                let bytes = serde_json::to_vec(def)
                    .map_err(|e| DbError::InternalError(format!("column blocks: {e}")))?;
                self.db
                    .put_ks(&cf, BLOCKS_KEY.as_bytes(), bytes)
                    .map_err(failed)?;
            }
            None => self
                .db
                .delete_ks(&cf, BLOCKS_KEY.as_bytes())
                .map_err(failed)?,
        }
        let set = def.map(|def| Arc::new(BlockSet::new(state.clone(), def)));
        SETS.insert(address, (state.clone(), set));
        // A query answered from blocks must not be answered from the cache
        // of before, nor the other way round.
        self.invalidate_cached_queries();
        Ok(())
    }

    /// Seal every range no block covers, the tail included. Returns the
    /// number of blocks added.
    pub fn seal_column_blocks(&self) -> DbResult<usize> {
        let Some(set) = self.column_blocks() else {
            return Ok(0);
        };
        let _sealing = set.sealing.lock();
        let mut added = 0;
        // Points generated in the very millisecond a seal starts in are
        // left to the next one: a moment later they are sealed too.
        for _ in 0..5 {
            loop {
                let sealed = set.seal_next(self, true, seal_threads())?;
                if sealed == 0 {
                    break;
                }
                added += sealed;
            }
            if set.uncovered(self) == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        while set.merge_next(self) {}
        set.store_all(self)?;
        Ok(added)
    }

    /// Seal what waits behind the newest block, and merge the small
    /// blocks: what the sealing thread does for a collection.
    fn seal_behind(&self, set: &BlockSet) {
        let waiting = (SEAL_ROWS + TAIL_KEPT) as u64;
        let _sealing = set.sealing.lock();
        loop {
            let uncovered = set.uncovered(self);
            if uncovered < waiting {
                break;
            }
            // Beside the writes, one block at a time. When whole blocks
            // wait (a bulk load writes faster than one thread seals),
            // several at once, on half the threads a full seal takes: the
            // rest are the writers'.
            let behind = (uncovered / BLOCK_ROWS as u64) as usize;
            let at_once = behind.clamp(1, (seal_threads() / 2).max(1));
            match set.seal_next(self, false, at_once) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("sealing a column block of '{}' failed: {}", self.name, e);
                    break;
                }
            }
        }
        while set.merge_next(self) {}
    }

    /// After a write: when enough documents wait behind the newest block
    /// (new points, or a range whose block a write threw away), hand the
    /// collection to the thread that seals. The write does not wait for
    /// it. If that thread cannot be started, `here` says whether this one
    /// may seal instead: not a thread others wait for.
    pub(crate) fn seal_column_blocks_behind(&self, set: &BlockSet, here: bool) {
        if set.uncovered(self) < (SEAL_ROWS + TAIL_KEPT) as u64
            || set.queued.swap(true, Ordering::AcqRel)
        {
            return;
        }
        if !send_to_sealer(self) {
            set.queued.store(false, Ordering::Release);
            if here {
                self.seal_behind(set);
            }
        }
    }

    /// A committed write to `keys`: the blocks holding them go, and what
    /// waits unsealed is handed to the sealing thread (`seal`: whether this
    /// thread may do it itself, should there be none).
    pub(crate) fn column_blocks_wrote(&self, keys: Wrote<'_>, seal: bool) {
        let Some(set) = self.column_blocks() else {
            return;
        };
        set.wrote(self, keys);
        self.seal_column_blocks_behind(&set, seal);
    }

    /// The state of this collection's blocks.
    pub fn column_blocks_status(&self) -> Option<BlocksStatus> {
        let set = self.column_blocks()?;
        let mut inner = set.inner.lock();
        set.load(&mut inner, self);
        Some(BlocksStatus {
            time: set.def.time.clone(),
            fields: set.def.fields.clone(),
            blocks: inner.blocks.len(),
            opaque_blocks: inner.blocks.values().filter(|e| e.opaque).count(),
            sealed_documents: set.sealed_rows.load(Ordering::Relaxed),
            documents: self.count(),
        })
    }
}

/// The thread that seals the blocks of the collections of one database
/// instance. Sealing reads a thousand documents and more: on the thread of
/// the write that crossed the threshold, it made that write, and those
/// queued behind it on a busy server, wait for it.
struct Sealer {
    jobs: std::sync::mpsc::Sender<Collection>,
    /// Set at shutdown: what is still queued is dropped, not sealed.
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// By database instance (its address; the collections queued keep it
/// alive, and [`stop_sealer`] removes the entry before it goes).
static SEALERS: once_cell::sync::Lazy<DashMap<usize, Sealer>> =
    once_cell::sync::Lazy::new(DashMap::new);

/// Queue `coll` for sealing; false when no thread could be started.
fn send_to_sealer(coll: &Collection) -> bool {
    let key = Arc::as_ptr(&coll.db) as usize;
    let jobs = match SEALERS.get(&key) {
        Some(sealer) => sealer.jobs.clone(),
        None => {
            let (jobs, queue) = std::sync::mpsc::channel::<Collection>();
            let stop = Arc::new(AtomicBool::new(false));
            let stopped = stop.clone();
            let spawned = std::thread::Builder::new()
                .name("solidb-seal".to_string())
                .spawn(move || {
                    for coll in queue {
                        let Some(set) = coll.column_blocks() else {
                            continue;
                        };
                        // Cleared first: a write that arrives while this
                        // seal runs queues the collection again.
                        set.queued.store(false, Ordering::Release);
                        if !stopped.load(Ordering::Acquire) {
                            coll.seal_behind(&set);
                        }
                    }
                });
            let Ok(thread) = spawned else {
                return false;
            };
            SEALERS
                .entry(key)
                .or_insert(Sealer {
                    jobs,
                    stop,
                    thread: Some(thread),
                })
                .jobs
                .clone()
        }
    };
    jobs.send(coll.clone()).is_ok()
}

/// Stop the sealing thread of `db` and wait for it: called when the last
/// handle on the engine goes, before the database is closed. Blocks not
/// sealed yet are sealed by the next process, or read as documents.
pub(crate) fn stop_sealer(db: &Arc<DB>) {
    let Some((_, mut sealer)) = SEALERS.remove(&(Arc::as_ptr(db) as usize)) else {
        return;
    };
    sealer.stop.store(true, Ordering::Release);
    let thread = sealer.thread.take();
    // The queue closes when its last sender goes: the thread then ends.
    drop(sealer);
    if let Some(thread) = thread {
        let _ = thread.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::document::Document;
    use crate::storage::serializer::serialize_doc;
    use serde_json::json;

    fn stored(docs: &[Value]) -> Vec<Vec<u8>> {
        docs.iter()
            .enumerate()
            .map(|(i, data)| {
                let doc = Document::with_key("c", format!("k{i:05}"), data.clone());
                serialize_doc(&doc).unwrap()
            })
            .collect()
    }

    fn decoded(bytes: &[Vec<u8>], fields: &[String]) -> Option<Columns> {
        bytes
            .iter()
            .map(|b| row_of(b, fields))
            .collect::<Option<Vec<_>>>()
            .and_then(columns_of)
    }

    fn same(docs: &[Value], fields: &[&str], plain: bool) {
        let fields: Vec<String> = fields.iter().map(|f| f.to_string()).collect();
        let bytes = stored(docs);
        let direct = columns_from_bytes(bytes.iter().map(Vec::as_slice), &fields);
        assert_eq!(direct.is_some(), plain, "{docs:?}");
        if let Some(direct) = direct {
            let through = decoded(&bytes, &fields).expect("the decoder takes what the bytes give");
            assert_eq!(
                bincode::serialize(&direct).unwrap(),
                bincode::serialize(&through).unwrap(),
                "{docs:?}"
            );
        }
    }

    /// Columns read from the stored bytes are the columns decoded documents
    /// give, and anything but plain numbers and strings is left to the
    /// decoder.
    #[test]
    fn columns_from_bytes_are_the_decoded_ones() {
        let points: Vec<Value> = (0..3_000i64)
            .map(|i| {
                json!({
                    "series": format!("s{:03}", (i * 7) % 41),
                    "ts": 1_760_000_000_000i64 + (i / 41) * 1_000 - (i % 5) * 3,
                    "v": ((i * 7919) % 1000) as f64 / if i % 3 == 0 { 4.0 } else { 3.0 },
                    "n": (i * 31) % 17 - 8,
                    "m": if i % 2 == 0 { json!(i % 9) } else { json!((i % 9) as f64 + 0.5) },
                    "big": i64::MAX - i,
                    "other": {"left": "out"},
                    "name": "é𝄞",
                })
            })
            .collect();
        same(
            &points,
            &["ts", "series", "v", "n", "m", "big", "name"],
            true,
        );
        same(&points, &["ts", "v"], true);
        same(&points[..1], &["ts", "series"], true);
        // Integers every width MessagePack has, and a long string.
        let widths: Vec<Value> = [
            0i64,
            -1,
            127,
            128,
            -32,
            -33,
            255,
            256,
            -129,
            65_535,
            65_536,
            -32_769,
            1 << 32,
            -(1 << 40),
            i64::MIN + 1,
        ]
        .iter()
        .map(|n| json!({"ts": n, "s": "x".repeat((n.unsigned_abs() % 300) as usize)}))
        .collect();
        same(&widths, &["ts", "s"], true);

        // Left to the decoder: a field missing, null, a boolean, an array,
        // strings among numbers, a time that is not an integer, an integer
        // past i64, a system field, no document.
        let plain = json!({"ts": 5, "v": 1.5, "s": "a"});
        for odd in [
            json!({"ts": 5, "s": "a"}),
            json!({"ts": 5, "v": null, "s": "a"}),
            json!({"ts": 5, "v": true, "s": "a"}),
            json!({"ts": 5, "v": [1], "s": "a"}),
            json!({"ts": 5, "v": "1.5", "s": "a"}),
            json!({"ts": 5.5, "v": 1.5, "s": "a"}),
            json!({"ts": 5, "v": u64::MAX, "s": "a"}),
        ] {
            same(&[plain.clone(), odd], &["ts", "v", "s"], false);
        }
        same(std::slice::from_ref(&plain), &["ts", "_key"], false);
        same(&[], &["ts", "v"], false);
        // Mixed numbers one of which a float does not hold exactly.
        same(
            &[
                json!({"ts": 1, "v": 1.5}),
                json!({"ts": 2, "v": (1i64 << 53) + 1}),
            ],
            &["ts", "v"],
            false,
        );
    }
}
