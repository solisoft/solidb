//! The storage engine's vocabulary, owned by SoliDB.
//!
//! Collection code talks to the engine in these types — iteration modes,
//! read options, errors, pinned reads — instead of RocksDB's, so that the
//! engine underneath can change (solistore, see the integration plan in
//! `~/Work/soli/store/docs/solidb-integration.md`) without touching it.
//! Today every value here wraps or converts to its RocksDB counterpart.

use std::fmt;
use std::ops::Deref;

use crate::error::DbError;

pub(crate) mod rocks;

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

impl From<KvError> for DbError {
    fn from(e: KvError) -> Self {
        DbError::InternalError(e.0)
    }
}

/// A value read without copying it out of the engine's cache.
pub enum KvPinned<'a> {
    Rocks(rust_rocksdb::DBPinnableSlice<'a>),
}

impl Deref for KvPinned<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            KvPinned::Rocks(slice) => slice,
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
}

impl From<RawRocks> for Kv {
    fn from(db: RawRocks) -> Self {
        Kv {
            engine: Engine::Rocks(db),
        }
    }
}

impl Kv {
    pub fn cf_handle(&self, name: &str) -> Option<KvCf<'_>> {
        match &self.engine {
            Engine::Rocks(db) => db.cf_handle(name).map(KvCf::Rocks),
        }
    }

    pub fn cf_names(&self) -> Vec<String> {
        match &self.engine {
            Engine::Rocks(db) => db.cf_names(),
        }
    }

    // ---- reads

    pub fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Vec<u8>>, KvError> {
        match &self.engine {
            Engine::Rocks(db) => Ok(db.get(key)?),
        }
    }

    pub fn get_cf<K: AsRef<[u8]>>(&self, cf: &KvCf, key: K) -> Result<Option<Vec<u8>>, KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => Ok(db.get_cf(h, key)?),
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
        }
    }

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
                    })
                    .collect();
                db.multi_get_cf(keys)
                    .into_iter()
                    .map(|r| r.map_err(KvError::from))
                    .collect()
            }
        }
    }

    // ---- writes

    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&self, key: K, value: V) -> Result<(), KvError> {
        match &self.engine {
            Engine::Rocks(db) => Ok(db.put(key, value)?),
        }
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
        }
    }

    pub fn delete<K: AsRef<[u8]>>(&self, key: K) -> Result<(), KvError> {
        match &self.engine {
            Engine::Rocks(db) => Ok(db.delete(key)?),
        }
    }

    pub fn delete_cf<K: AsRef<[u8]>>(&self, cf: &KvCf, key: K) -> Result<(), KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => Ok(db.delete_cf(h, key)?),
        }
    }

    /// Apply a batch atomically.
    pub fn write(&self, batch: &KvBatch) -> Result<(), KvError> {
        self.write_opt(batch, &KvWriteOptions::default())
    }

    pub fn write_opt(&self, batch: &KvBatch, opts: &KvWriteOptions) -> Result<(), KvError> {
        match (&self.engine, batch) {
            (Engine::Rocks(db), KvBatch::Rocks(b)) => Ok(db.write_opt(b, &opts.to_rocks())?),
        }
    }

    // ---- iterators

    pub fn iterator(&self, mode: IterMode) -> KvIter<'_> {
        match &self.engine {
            Engine::Rocks(db) => KvIter::Rocks(db.iterator(mode.to_rocks())),
        }
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
        match &self.engine {
            Engine::Rocks(db) => db.compact_range(start, end),
        }
    }

    pub fn compact_range_cf<S: AsRef<[u8]>, E: AsRef<[u8]>>(
        &self,
        cf: &KvCf,
        start: Option<S>,
        end: Option<E>,
    ) {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => db.compact_range_cf(h, start, end),
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
        }
    }

    pub fn flush(&self) -> Result<(), KvError> {
        match &self.engine {
            Engine::Rocks(db) => Ok(db.flush()?),
        }
    }

    pub fn flush_cf(&self, cf: &KvCf) -> Result<(), KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => Ok(db.flush_cf(h)?),
        }
    }

    pub fn drop_cf(&self, name: &str) -> Result<(), KvError> {
        match &self.engine {
            Engine::Rocks(db) => Ok(db.drop_cf(name)?),
        }
    }

    /// Create a column family. Only the 1.x per-collection layout creates
    /// them at run time; this goes with it.
    pub fn create_cf(&self, name: &str) -> Result<(), KvError> {
        match &self.engine {
            Engine::Rocks(db) => Ok(db.create_cf(name, &rocks::tuned_cf_options())?),
        }
    }

    /// Drop the table files wholly inside `[from, to)` without compacting.
    /// Used by the 1.x migration to free space as it goes.
    pub fn delete_file_in_range_cf(
        &self,
        cf: &KvCf,
        from: &[u8],
        to: &[u8],
    ) -> Result<(), KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => Ok(db.delete_file_in_range_cf(h, from, to)?),
        }
    }

    /// An integer property of a column family (RocksDB's `rocksdb.*`
    /// names); `Ok(None)` when the engine has no such figure.
    pub fn property_int_value_cf(&self, cf: &KvCf, name: &str) -> Result<Option<u64>, KvError> {
        match (&self.engine, cf) {
            (Engine::Rocks(db), KvCf::Rocks(h)) => Ok(db.property_int_value_cf(h, name)?),
        }
    }

    /// A consistent copy of the whole database at `target` (hard links).
    pub fn checkpoint(&self, target: &std::path::Path) -> Result<(), KvError> {
        match &self.engine {
            Engine::Rocks(db) => {
                let checkpoint = rust_rocksdb::checkpoint::Checkpoint::new(db)?;
                Ok(checkpoint.create_checkpoint(target)?)
            }
        }
    }
}

/// A handle on one column family of a [`Kv`].
pub enum KvCf<'a> {
    Rocks(std::sync::Arc<rust_rocksdb::BoundColumnFamily<'a>>),
}

/// Writes applied together by [`Kv::write`].
pub enum KvBatch {
    Rocks(rust_rocksdb::WriteBatch),
}

impl Default for KvBatch {
    fn default() -> Self {
        KvBatch::Rocks(rust_rocksdb::WriteBatch::default())
    }
}

impl KvBatch {
    pub fn put<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, key: K, value: V) {
        match self {
            KvBatch::Rocks(b) => b.put(key, value),
        }
    }

    pub fn put_cf<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, cf: &KvCf, key: K, value: V) {
        match (self, cf) {
            (KvBatch::Rocks(b), KvCf::Rocks(h)) => b.put_cf(h, key, value),
        }
    }

    pub fn delete<K: AsRef<[u8]>>(&mut self, key: K) {
        match self {
            KvBatch::Rocks(b) => b.delete(key),
        }
    }

    pub fn delete_cf<K: AsRef<[u8]>>(&mut self, cf: &KvCf, key: K) {
        match (self, cf) {
            (KvBatch::Rocks(b), KvCf::Rocks(h)) => b.delete_cf(h, key),
        }
    }

    pub fn delete_range_cf<K: AsRef<[u8]>>(&mut self, cf: &KvCf, from: K, to: K) {
        match (self, cf) {
            (KvBatch::Rocks(b), KvCf::Rocks(h)) => b.delete_range_cf(h, from, to),
        }
    }

    /// Number of operations.
    pub fn len(&self) -> usize {
        match self {
            KvBatch::Rocks(b) => b.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Size of the batch's encoding in bytes.
    pub fn size_in_bytes(&self) -> usize {
        match self {
            KvBatch::Rocks(b) => b.size_in_bytes(),
        }
    }
}

/// An iterator over one column family: physical keys and values.
pub enum KvIter<'a> {
    Rocks(rust_rocksdb::DBIteratorWithThreadMode<'a, RawRocks>),
}

impl Iterator for KvIter<'_> {
    type Item = Result<(Box<[u8]>, Box<[u8]>), KvError>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            KvIter::Rocks(it) => it.next().map(|r| r.map_err(KvError::from)),
        }
    }
}
