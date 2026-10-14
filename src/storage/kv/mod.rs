//! The storage engine's vocabulary, owned by SoliDB.
//!
//! Collection code talks to the engine in these types — iteration modes,
//! read options, errors, pinned reads — instead of RocksDB's, so that the
//! engine underneath can change without touching it. Two engines exist:
//! RocksDB, and solistore (github.com/solisoft/solistore) behind the
//! `engine-solistore` feature. A data directory is opened with the engine
//! that created it (see [`open`]).

use std::fmt;
use std::ops::Deref;

use crate::error::DbError;

pub(crate) mod rocks;
mod select;
#[cfg(feature = "engine-solistore")]
pub(crate) mod solistore;

pub use select::{open, open_sync_log, EngineKind};

/// Which way an iterator walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Forward,
    Reverse,
}

/// Where an iterator starts. `From` keys are logical (keyspace-relative).
#[derive(Debug, Clone, Copy)]
pub enum IterMode<'a> {
    /// The first key, walking forward.
    Start,
    /// The last key, walking backward.
    End,
    /// The first key at or after this one (`Forward`), or the last at or
    /// before it (`Reverse`).
    From(&'a [u8], Direction),
}

impl<'a> IterMode<'a> {
    pub(crate) fn to_rocks(self) -> rust_rocksdb::IteratorMode<'a> {
        match self {
            IterMode::Start => rust_rocksdb::IteratorMode::Start,
            IterMode::End => rust_rocksdb::IteratorMode::End,
            IterMode::From(key, Direction::Forward) => {
                rust_rocksdb::IteratorMode::From(key, rust_rocksdb::Direction::Forward)
            }
            IterMode::From(key, Direction::Reverse) => {
                rust_rocksdb::IteratorMode::From(key, rust_rocksdb::Direction::Reverse)
            }
        }
    }

    #[cfg(feature = "engine-solistore")]
    pub(crate) fn to_solistore(self) -> ::solistore::IteratorMode<'a> {
        use ::solistore::{Direction as D, IteratorMode as M};
        match self {
            IterMode::Start => M::Start,
            IterMode::End => M::End,
            IterMode::From(key, Direction::Forward) => M::From(key, D::Forward),
            IterMode::From(key, Direction::Reverse) => M::From(key, D::Reverse),
        }
    }
}

/// Read options for a scan. Bounds set here are *physical* keys; collection
/// code bounds its scans with the keyspace iterators instead.
#[derive(Debug, Clone, Default)]
pub struct KvReadOptions {
    readahead: usize,
    lower: Option<Vec<u8>>,
    upper: Option<Vec<u8>>,
}

impl KvReadOptions {
    /// Bytes to read ahead for a long sequential scan (0: none). An engine
    /// that reads with `pread` and relies on the OS may ignore it.
    pub fn set_readahead_size(&mut self, bytes: usize) {
        self.readahead = bytes;
    }

    /// Inclusive physical lower bound.
    pub fn set_iterate_lower_bound(&mut self, key: Vec<u8>) {
        self.lower = Some(key);
    }

    /// Exclusive physical upper bound.
    pub fn set_iterate_upper_bound(&mut self, key: Vec<u8>) {
        self.upper = Some(key);
    }

    pub(crate) fn to_rocks(&self) -> rust_rocksdb::ReadOptions {
        let mut opts = rust_rocksdb::ReadOptions::default();
        if self.readahead > 0 {
            opts.set_readahead_size(self.readahead);
        }
        if let Some(lo) = &self.lower {
            opts.set_iterate_lower_bound(lo.clone());
        }
        if let Some(hi) = &self.upper {
            opts.set_iterate_upper_bound(hi.clone());
        }
        opts
    }

    #[cfg(feature = "engine-solistore")]
    pub(crate) fn to_solistore(&self) -> ::solistore::ReadOptions {
        // Readahead does not apply: solistore reads with pread and lets the
        // OS read ahead.
        ::solistore::ReadOptions {
            lower_bound: self.lower.clone(),
            upper_bound: self.upper.clone(),
        }
    }
}

/// An error from the storage engine.
#[derive(Debug, Clone)]
pub struct KvError(String);

impl fmt::Display for KvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for KvError {}

impl From<rust_rocksdb::Error> for KvError {
    fn from(e: rust_rocksdb::Error) -> Self {
        KvError(e.into_string())
    }
}

#[cfg(feature = "engine-solistore")]
impl From<::solistore::Error> for KvError {
    fn from(e: ::solistore::Error) -> Self {
        KvError(e.to_string())
    }
}

impl KvError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        KvError(message.into())
    }
}

impl From<KvError> for DbError {
    fn from(e: KvError) -> Self {
        DbError::InternalError(e.0)
    }
}

/// A value read without copying it out of the engine's cache.
pub enum KvPinned<'a> {
    Rocks(rust_rocksdb::DBPinnableSlice<'a>),
    #[cfg(feature = "engine-solistore")]
    Solistore(::solistore::Bytes),
}

impl Deref for KvPinned<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            KvPinned::Rocks(slice) => slice,
            #[cfg(feature = "engine-solistore")]
            KvPinned::Solistore(bytes) => bytes,
        }
    }
}

impl AsRef<[u8]> for KvPinned<'_> {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

// ---------------------------------------------------------------- the engine

/// RocksDB as SoliDB opens it. Named outside this module only where a
/// database is opened.
pub type RawRocks = rust_rocksdb::DBWithThreadMode<rust_rocksdb::MultiThreaded>;

/// Write options: `sync` makes the write durable before it returns.
#[derive(Debug, Clone, Copy, Default)]
pub struct KvWriteOptions {
    sync: bool,
}

impl KvWriteOptions {
    pub fn set_sync(&mut self, sync: bool) {
        self.sync = sync;
    }

    /// Options with `sync` set.
    pub fn synced() -> Self {
        KvWriteOptions { sync: true }
    }

    fn to_rocks(self) -> rust_rocksdb::WriteOptions {
        let mut opts = rust_rocksdb::WriteOptions::default();
        opts.set_sync(self.sync);
        opts
    }

    #[cfg(feature = "engine-solistore")]
    fn to_solistore(self) -> ::solistore::WriteOptions {
        ::solistore::WriteOptions { sync: self.sync }
    }
}

/// Memory the engine holds, database-wide.
#[derive(Debug, Clone, Copy, Default)]
pub struct KvMemory {
    /// Live memtables.
    pub memtable_bytes: u64,
    /// Live memtables plus sealed ones waiting for a flush.
    pub memtable_total_bytes: u64,
    /// Index and filter blocks of open table files.
    pub table_readers_bytes: u64,
    /// Table files on disk.
    pub table_files: u64,
    pub block_cache_bytes: u64,
    /// Part of the block cache that cannot be evicted (0 where the engine
    /// does not pin).
    pub block_cache_pinned_bytes: u64,
}

/// The storage engine: a set of named column families of ordered keys.
/// Method names follow RocksDB's, so code reads the same whichever engine
/// is underneath; the types are SoliDB's. Methods without `_cf` act on the
/// default column family.
pub struct Kv {
    engine: Engine,
}

enum Engine {
    Rocks(RawRocks),
    #[cfg(feature = "engine-solistore")]
    Solistore(::solistore::Db),
}

impl From<RawRocks> for Kv {
    fn from(db: RawRocks) -> Self {
        Kv {
            engine: Engine::Rocks(db),
        }
    }
}

#[cfg(feature = "engine-solistore")]
impl From<::solistore::Db> for Kv {
    fn from(db: ::solistore::Db) -> Self {
        Kv {
            engine: Engine::Solistore(db),
        }
    }
}

/// A handle of one engine used with another: a bug, not a runtime state.
#[cfg(feature = "engine-solistore")]
#[cold]
fn mixed_engines() -> ! {
    panic!("a column family or batch of one storage engine used with another")
}

impl Kv {
    pub fn engine_kind(&self) -> EngineKind {
        match &self.engine {
            Engine::Rocks(_) => EngineKind::Rocksdb,
            #[cfg(feature = "engine-solistore")]
            Engine::Solistore(_) => EngineKind::Solistore,
        }
    }

    pub fn cf_handle(&self, name: &str) -> Option<KvCf<'_>> {
        match &self.engine {
            Engine::Rocks(db) => db.cf_handle(name).map(KvCf::Rocks),
            #[cfg(feature = "engine-solistore")]
            Engine::Solistore(db) => db.cf_handle(name).map(KvCf::Solistore),
        }
    }

    /// The default column family.
    pub fn default_cf(&self) -> KvCf<'_> {
        match &self.engine {
            Engine::Rocks(db) => KvCf::Rocks(
                db.cf_handle(rust_rocksdb::DEFAULT_COLUMN_FAMILY_NAME)
                    .expect("RocksDB is opened with its default column family listed"),
            ),
            #[cfg(feature = "engine-solistore")]
            Engine::Solistore(db) => KvCf::Solistore(db.default_cf()),
        }
    }

    pub fn cf_names(&self) -> Vec<String> {
        match &self.engine {
            Engine::Rocks(db) => db.cf_names(),
            #[cfg(feature = "engine-solistore")]
            Engine::Solistore(db) => db.cf_names(),
        }
    }

    // ---- reads

    pub fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>, KvError> {
        self.get_cf(&self.default_cf(), key)
    }

    pub fn get_cf<K: AsRef<[u8]>>(&self, cf: &KvCf, key: K) -> Result<Option<Vec<u8>>, KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => Ok(db.get_cf(h, key)?),
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(db), KvCf::Solistore(h)) => Ok(db.get(h, key)?.map(|v| v.to_vec())),
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    pub fn get_pinned_cf<K: AsRef<[u8]>>(
        &self,
        cf: &KvCf,
        key: K,
    ) -> Result<Option<KvPinned<'_>>, KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => {
                Ok(db.get_pinned_cf(h, key)?.map(KvPinned::Rocks))
            }
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(db), KvCf::Solistore(h)) => {
                Ok(db.get(h, key)?.map(KvPinned::Solistore))
            }
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    /// Several keys at once, read at one point in time.
    pub fn multi_get_cf<'c, 'h: 'c, K: AsRef<[u8]>>(
        &self,
        keys: impl IntoIterator<Item = (&'c KvCf<'h>, K)>,
    ) -> Vec<Result<Option<Vec<u8>>, KvError>> {
        match &self.engine {
            Engine::Rocks(db) => {
                let keys: Vec<_> = keys
                    .into_iter()
                    .map(|(cf, k)| match cf {
                        KvCf::Rocks(h) => (h, k),
                        #[cfg(feature = "engine-solistore")]
                        KvCf::Solistore(_) => mixed_engines(),
                    })
                    .collect();
                db.multi_get_cf(keys)
                    .into_iter()
                    .map(|r| r.map_err(KvError::from))
                    .collect()
            }
            #[cfg(feature = "engine-solistore")]
            Engine::Solistore(db) => solistore::multi_get(db, keys),
        }
    }

    // ---- writes

    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&self, key: K, value: V) -> Result<(), KvError> {
        self.put_cf(&self.default_cf(), key, value)
    }

    pub fn put_cf<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        cf: &KvCf,
        key: K,
        value: V,
    ) -> Result<(), KvError> {
        self.put_cf_opt(cf, key, value, &KvWriteOptions::default())
    }

    pub fn put_cf_opt<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        cf: &KvCf,
        key: K,
        value: V,
        opts: &KvWriteOptions,
    ) -> Result<(), KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => {
                Ok(db.put_cf_opt(h, key, value, &opts.to_rocks())?)
            }
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(db), KvCf::Solistore(h)) => {
                let mut batch = ::solistore::WriteBatch::new();
                batch.put(h, key, value);
                Ok(db.write_opt(batch, &opts.to_solistore())?)
            }
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    pub fn delete<K: AsRef<[u8]>>(&self, key: K) -> Result<(), KvError> {
        self.delete_cf(&self.default_cf(), key)
    }

    pub fn delete_cf<K: AsRef<[u8]>>(&self, cf: &KvCf, key: K) -> Result<(), KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => Ok(db.delete_cf(h, key)?),
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(db), KvCf::Solistore(h)) => Ok(db.delete(h, key)?),
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    /// Apply a batch atomically.
    pub fn write(&self, batch: KvBatch) -> Result<(), KvError> {
        self.write_opt(batch, &KvWriteOptions::default())
    }

    pub fn write_opt(&self, batch: KvBatch, opts: &KvWriteOptions) -> Result<(), KvError> {
        match (&self.engine, batch) {
            (_, KvBatch::Empty) => Ok(()),
            (Engine::Rocks(db), KvBatch::Rocks(b)) => Ok(db.write_opt(&b, &opts.to_rocks())?),
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(db), KvBatch::Solistore(b)) => {
                Ok(db.write_opt(b, &opts.to_solistore())?)
            }
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    // ---- iterators

    pub fn iterator(&self, mode: IterMode) -> KvIter<'_> {
        self.iterator_cf(&self.default_cf(), mode)
    }

    /// Seek to `prefix` in the default column family and walk forward to its
    /// end; the caller stops when keys no longer start with it.
    pub fn prefix_iterator<P: AsRef<[u8]>>(&self, prefix: P) -> KvIter<'_> {
        self.iterator(IterMode::From(prefix.as_ref(), Direction::Forward))
    }

    pub fn iterator_cf(&self, cf: &KvCf, mode: IterMode) -> KvIter<'_> {
        self.iterator_cf_opt(cf, KvReadOptions::default(), mode)
    }

    pub fn iterator_cf_opt<'a>(
        &'a self,
        cf: &KvCf<'_>,
        opts: KvReadOptions,
        mode: IterMode,
    ) -> KvIter<'a> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => {
                KvIter::Rocks(db.iterator_cf_opt(h, opts.to_rocks(), mode.to_rocks()))
            }
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(db), KvCf::Solistore(h)) => {
                match db.iterator_opt(h, opts.to_solistore(), mode.to_solistore()) {
                    Ok(it) => KvIter::Solistore(it),
                    Err(e) => KvIter::Failed(Some(e.into())),
                }
            }
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    /// Seek to `prefix` and walk forward to the end of the column family;
    /// the caller stops when keys no longer start with it (RocksDB's
    /// `prefix_iterator_cf` without a prefix extractor).
    pub fn prefix_iterator_cf<'a, P: AsRef<[u8]>>(
        &'a self,
        cf: &KvCf<'_>,
        prefix: P,
    ) -> KvIter<'a> {
        self.iterator_cf(cf, IterMode::From(prefix.as_ref(), Direction::Forward))
    }

    // ---- maintenance

    pub fn compact_range<S: AsRef<[u8]>, E: AsRef<[u8]>>(&self, start: Option<S>, end: Option<E>) {
        self.compact_range_cf(&self.default_cf(), start, end)
    }

    pub fn compact_range_cf<S: AsRef<[u8]>, E: AsRef<[u8]>>(
        &self,
        cf: &KvCf,
        start: Option<S>,
        end: Option<E>,
    ) {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => db.compact_range_cf(h, start, end),
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(db), KvCf::Solistore(h)) => {
                let (start, end) = (
                    start.as_ref().map(|s| s.as_ref()),
                    end.as_ref().map(|e| e.as_ref()),
                );
                if let Err(e) = db.compact_range(h, start, end) {
                    tracing::warn!("compact_range on {}: {}", h.name(), e);
                }
            }
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    /// Approximate on-disk size of `[start, end)` (flushed data only); 0
    /// when the engine cannot say.
    pub fn approximate_size_cf(&self, cf: &KvCf, start: &[u8], end: &[u8]) -> u64 {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => db
                .get_approximate_sizes_cf(h, &[rust_rocksdb::Range::new(start, end)])
                .ok()
                .and_then(|sizes| sizes.first().copied())
                .unwrap_or(0),
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(db), KvCf::Solistore(h)) => {
                db.approximate_size(h, start, end).unwrap_or(0)
            }
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    pub fn flush(&self) -> Result<(), KvError> {
        match &self.engine {
            Engine::Rocks(db) => Ok(db.flush()?),
            #[cfg(feature = "engine-solistore")]
            Engine::Solistore(db) => Ok(db.flush()?),
        }
    }

    pub fn flush_cf(&self, cf: &KvCf) -> Result<(), KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => Ok(db.flush_cf(h)?),
            // One LSM for every column family: a flush covers them all.
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(db), KvCf::Solistore(_)) => Ok(db.flush()?),
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    pub fn drop_cf(&self, name: &str) -> Result<(), KvError> {
        match &self.engine {
            Engine::Rocks(db) => Ok(db.drop_cf(name)?),
            #[cfg(feature = "engine-solistore")]
            Engine::Solistore(db) => Ok(db.drop_cf(name)?),
        }
    }

    /// Create a column family. Only the 1.x per-collection layout creates
    /// them at run time; this goes with it.
    pub fn create_cf(&self, name: &str) -> Result<(), KvError> {
        match &self.engine {
            Engine::Rocks(db) => Ok(db.create_cf(name, &rocks::tuned_cf_options())?),
            #[cfg(feature = "engine-solistore")]
            Engine::Solistore(db) => db.create_cf(name).map(drop).map_err(Into::into),
        }
    }

    /// Drop the table files wholly inside `[from, to)` without compacting.
    /// Used by the 1.x migration to free space as it goes; solistore never
    /// holds a 1.x layout.
    pub fn delete_file_in_range_cf(
        &self,
        cf: &KvCf,
        from: &[u8],
        to: &[u8],
    ) -> Result<(), KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => Ok(db.delete_file_in_range_cf(h, from, to)?),
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(_), KvCf::Solistore(_)) => Ok(()),
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    /// An integer property of one column family (RocksDB's `rocksdb.*`
    /// names); `Ok(None)` when the engine has no such figure. solistore
    /// keeps its figures database-wide: see [`Kv::memory`].
    pub fn property_int_value_cf(&self, cf: &KvCf, name: &str) -> Result<Option<u64>, KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => Ok(db.property_int_value_cf(h, name)?),
            #[cfg(feature = "engine-solistore")]
            (Engine::Solistore(_), KvCf::Solistore(_)) => Ok(None),
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
        }
    }

    /// Memory held by the engine. RocksDB's figures are per column family
    /// and summed over `cfs`; its block cache is shared and read once.
    pub fn memory(&self, cfs: &[String]) -> KvMemory {
        match &self.engine {
            Engine::Rocks(db) => rocks::memory(db, cfs),
            #[cfg(feature = "engine-solistore")]
            Engine::Solistore(db) => solistore::memory(db),
        }
    }

    /// A consistent copy of the whole database at `target` (hard links),
    /// with the `ENGINE` file that says how to open it.
    pub fn checkpoint(&self, target: &std::path::Path) -> Result<(), KvError> {
        match &self.engine {
            Engine::Rocks(db) => {
                let checkpoint = rust_rocksdb::checkpoint::Checkpoint::new(db)?;
                checkpoint.create_checkpoint(target)?;
            }
            #[cfg(feature = "engine-solistore")]
            Engine::Solistore(db) => db.checkpoint(target)?,
        }
        select::record(target, self.engine_kind())
    }
}

/// A handle on one column family of a [`Kv`].
pub enum KvCf<'a> {
    Rocks(std::sync::Arc<rust_rocksdb::BoundColumnFamily<'a>>),
    #[cfg(feature = "engine-solistore")]
    Solistore(::solistore::ColumnFamily),
}

/// Writes applied together by [`Kv::write`]. A batch takes its engine from
/// the first column family written to it, so `KvBatch::default()` needs no
/// database.
#[derive(Default)]
pub enum KvBatch {
    #[default]
    Empty,
    Rocks(rust_rocksdb::WriteBatch),
    #[cfg(feature = "engine-solistore")]
    Solistore(::solistore::WriteBatch),
}

impl KvBatch {
    fn for_cf(&mut self, cf: &KvCf) -> &mut Self {
        if let KvBatch::Empty = self {
            *self = match cf {
                KvCf::Rocks(_) => KvBatch::Rocks(rust_rocksdb::WriteBatch::default()),
                #[cfg(feature = "engine-solistore")]
                KvCf::Solistore(_) => KvBatch::Solistore(::solistore::WriteBatch::new()),
            };
        }
        self
    }

    pub fn put_cf<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, cf: &KvCf, key: K, value: V) {
        match (self.for_cf(cf), cf) {
            (KvBatch::Rocks(b), KvCf::Rocks(h)) => b.put_cf(h, key, value),
            #[cfg(feature = "engine-solistore")]
            (KvBatch::Solistore(b), KvCf::Solistore(h)) => b.put(h, key, value),
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
            #[cfg(not(feature = "engine-solistore"))]
            (KvBatch::Empty, _) => unreachable!("typed by for_cf"),
        }
    }

    pub fn delete_cf<K: AsRef<[u8]>>(&mut self, cf: &KvCf, key: K) {
        match (self.for_cf(cf), cf) {
            (KvBatch::Rocks(b), KvCf::Rocks(h)) => b.delete_cf(h, key),
            #[cfg(feature = "engine-solistore")]
            (KvBatch::Solistore(b), KvCf::Solistore(h)) => b.delete(h, key),
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
            #[cfg(not(feature = "engine-solistore"))]
            (KvBatch::Empty, _) => unreachable!("typed by for_cf"),
        }
    }

    pub fn delete_range_cf<K: AsRef<[u8]>>(&mut self, cf: &KvCf, from: K, to: K) {
        match (self.for_cf(cf), cf) {
            (KvBatch::Rocks(b), KvCf::Rocks(h)) => b.delete_range_cf(h, from, to),
            #[cfg(feature = "engine-solistore")]
            (KvBatch::Solistore(b), KvCf::Solistore(h)) => b.delete_range(h, from, to),
            #[cfg(feature = "engine-solistore")]
            _ => mixed_engines(),
            #[cfg(not(feature = "engine-solistore"))]
            (KvBatch::Empty, _) => unreachable!("typed by for_cf"),
        }
    }

    /// Number of operations.
    pub fn len(&self) -> usize {
        match self {
            KvBatch::Empty => 0,
            KvBatch::Rocks(b) => b.len(),
            #[cfg(feature = "engine-solistore")]
            KvBatch::Solistore(b) => b.len() as usize,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Size of the batch's encoding in bytes.
    pub fn size_in_bytes(&self) -> usize {
        match self {
            KvBatch::Empty => 0,
            KvBatch::Rocks(b) => b.size_in_bytes(),
            #[cfg(feature = "engine-solistore")]
            KvBatch::Solistore(b) => b.size_in_bytes(),
        }
    }
}

/// An iterator over one column family: physical keys and values.
pub enum KvIter<'a> {
    Rocks(rust_rocksdb::DBIteratorWithThreadMode<'a, RawRocks>),
    #[cfg(feature = "engine-solistore")]
    Solistore(::solistore::DbIterator),
    /// The iterator could not be created; yields the error once.
    #[cfg(feature = "engine-solistore")]
    Failed(Option<KvError>),
}

impl Iterator for KvIter<'_> {
    type Item = Result<(bytes::Bytes, bytes::Bytes), KvError>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            KvIter::Rocks(it) => it.next().map(|r| {
                r.map(|(k, v)| (bytes::Bytes::from(k), bytes::Bytes::from(v)))
                    .map_err(KvError::from)
            }),
            #[cfg(feature = "engine-solistore")]
            KvIter::Solistore(it) => it.next().map(|r| r.map_err(KvError::from)),
            #[cfg(feature = "engine-solistore")]
            KvIter::Failed(e) => e.take().map(Err),
        }
    }
}
