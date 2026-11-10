//! Keyspaces: where a collection's keys live.
//!
//! A collection used to be one RocksDB column family, and every create or drop
//! of one rewrote and fsynced the whole OPTIONS file — a cost proportional to
//! the instance's total collection count (≈0.2 s per create at 1,232
//! collections). A keyspace decouples the two:
//!
//! - **Legacy**: the collection's own column family, keys stored as-is. What
//!   1.x wrote, and what a 2.0 instance still reads until the startup
//!   migration has moved the collection.
//! - **Shared**: the single [`SHARED_CF`] column family, every key prefixed by
//!   eight bytes — the database id then the collection id, both big-endian
//!   `u32`. Ids are never reused, so a dropped-then-recreated collection gets
//!   a fresh, empty range, and dropping a whole database is one range delete.
//!
//! Collection code keeps building *logical* keys (`doc:…`, `idx:…`). The
//! `*_ks` methods here are the only place the prefix is added or stripped:
//! [`KsDbExt`] mirrors the `*_cf` reads, writes and iterators of
//! [`Kv`], [`KsBatchExt`] mirrors [`KvBatch`]. A raw `*_cf` call
//! does not accept a [`KsCf`], so a site that was not converted fails to
//! compile rather than writing an unprefixed key into the shared family.
//!
//! Every shared-layout iterator is bounded to its keyspace (lower and upper
//! bound), so a scan that forgets to stop at the end of its logical prefix
//! still cannot walk into the next collection.

use crate::error::{DbError, DbResult};
use crate::storage::kv::{
    Direction, IterMode, Kv, KvBatch, KvCf, KvError, KvIter, KvPinned, KvReadOptions, KvSnapshot,
};
use crate::storage::RocksDb as DB;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};

/// The column family every shared-layout collection lives in. No `:` in the
/// name, so a 1.x binary's registry backfill never mistakes it for a
/// collection.
pub const SHARED_CF: &str = "__keyspaces__";

/// Database id of engine-level collections created without a database
/// (`StorageEngine::create_collection("bare")`).
pub const BARE_DB_ID: u32 = 0;

/// Never allocated: keeps every keyspace's exclusive upper bound
/// (`prefix + 1`) representable without a carry out of eight bytes.
pub const RESERVED_DB_ID: u32 = u32::MAX;

/// `(db_id << 32) | coll_id`.
pub type KsNum = u64;

pub fn ks_num(db_id: u32, coll_id: u32) -> KsNum {
    ((db_id as u64) << 32) | coll_id as u64
}

pub fn ks_db_id(ks: KsNum) -> u32 {
    (ks >> 32) as u32
}

/// The eight-byte key prefix of one shared keyspace, or nothing for a legacy
/// one. `Copy`, so a [`KsCf`] can carry it without borrowing the collection.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct KsPrefix {
    bytes: [u8; 8],
    len: u8,
}

impl KsPrefix {
    pub const LEGACY: KsPrefix = KsPrefix {
        bytes: [0; 8],
        len: 0,
    };

    pub fn shared(ks: KsNum) -> Self {
        KsPrefix {
            bytes: ks.to_be_bytes(),
            len: 8,
        }
    }

    pub fn is_legacy(&self) -> bool {
        self.len == 0
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    /// The physical key for a logical one.
    #[inline]
    pub fn key(&self, logical: &[u8]) -> Vec<u8> {
        if self.len == 0 {
            return logical.to_vec();
        }
        let mut k = Vec::with_capacity(8 + logical.len());
        k.extend_from_slice(self.as_bytes());
        k.extend_from_slice(logical);
        k
    }

    /// Exclusive upper bound of the whole keyspace, `None` for legacy (the end
    /// of the column family is the end of the collection).
    pub fn upper(&self) -> Option<Vec<u8>> {
        if self.len == 0 {
            return None;
        }
        let n = u64::from_be_bytes(self.bytes);
        // RESERVED_DB_ID is never allocated, so this cannot overflow.
        Some((n + 1).to_be_bytes().to_vec())
    }

    /// Strip the prefix from a physical key read back from the engine
    /// (a slice of the same buffer, no copy).
    #[inline]
    fn strip(&self, physical: bytes::Bytes) -> bytes::Bytes {
        if self.len == 0 {
            physical
        } else {
            physical.slice(self.len as usize..)
        }
    }
}

/// Liveness shared by every handle of one keyspace. Interned per
/// (RocksDB instance, keyspace), so marking it dead reaches handles that other
/// code still holds.
#[derive(Debug, Default)]
pub struct KsState {
    dead: AtomicBool,
    /// Moved after every committed change to the keyspace's documents or
    /// indexes (`Collection::invalidate_cached_queries`). A transaction that
    /// read a range of the collection compares it at commit.
    write_gen: AtomicU64,
    /// Held shared by a transaction commit writing this keyspace, from its
    /// checks until its generation has moved; exclusively by a commit that
    /// has to find the generation unchanged. See [`ReadTracker`].
    commit_gate: parking_lot::RwLock<()>,
}

impl KsState {
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Acquire)
    }

    pub fn write_generation(&self) -> u64 {
        self.write_gen.load(Ordering::Acquire)
    }

    pub fn bump_write_generation(&self) {
        self.write_gen.fetch_add(1, Ordering::AcqRel);
    }

    pub fn commit_gate(&self) -> &parking_lot::RwLock<()> {
        &self.commit_gate
    }
}

/// What a transaction at `repeatable_read` or `serializable` read from one
/// collection, recorded where every read of a collection passes: the
/// keyspace calls below. The commit checks it (`StorageEngine::
/// commit_transaction`) and refuses when another writer got in between.
///
/// - A document read by its key (`get`, `multi_get` of a `doc:` key) is
///   remembered with the revision it had, or as absent.
/// - Any other read — an iterator, an index entry, a scan — marks the
///   collection as read by range: the commit then requires that nothing at
///   all was committed to it since the transaction first opened it. That is
///   coarse, and it is what covers a document that appears (a phantom).
/// - Reads of index definitions and other metadata are not data reads.
#[derive(Debug)]
pub struct ReadTracker {
    state: Arc<KsState>,
    /// The keyspace's write generation when the transaction first opened it,
    /// taken before anything was read.
    generation: u64,
    ranged: AtomicBool,
    /// Document key -> the revision read (`None`: there was no document).
    points: parking_lot::Mutex<HashMap<Vec<u8>, Option<Box<str>>>>,
    /// Set when a key was read a second time at another revision.
    conflict: parking_lot::Mutex<Option<String>>,
}

/// Past this many documents read by key in one collection, the transaction
/// is held to the coarser range rule instead of growing without bound.
const MAX_TRACKED_POINTS: usize = 100_000;

const DOC_KEY_PREFIX: &[u8] = b"doc:";
/// Legacy TTL expiry entries live under the document prefix.
const LEGACY_TTL_KEY_PREFIX: &[u8] = b"doc:ttl_exp:";
/// Definitions, not data: reading them says nothing about the documents.
const METADATA_KEY_PREFIXES: &[&[u8]] = &[
    b"idx_meta:",
    b"geo_meta:",
    b"ft_meta:",
    b"ttl_meta:",
    b"vec_meta:",
];

impl ReadTracker {
    fn new(state: Arc<KsState>) -> Self {
        ReadTracker {
            generation: state.write_generation(),
            state,
            ranged: AtomicBool::new(false),
            points: parking_lot::Mutex::new(HashMap::new()),
            conflict: parking_lot::Mutex::new(None),
        }
    }

    /// The transaction read this collection other than by document key.
    pub fn mark_ranged(&self) {
        self.ranged.store(true, Ordering::Release);
    }

    pub fn is_ranged(&self) -> bool {
        self.ranged.load(Ordering::Acquire)
    }

    /// Whether anything was committed to the collection since the
    /// transaction opened it.
    pub fn generation_moved(&self) -> bool {
        self.state.write_generation() != self.generation
    }

    pub fn state(&self) -> &Arc<KsState> {
        &self.state
    }

    /// The documents read by key: key (without the `doc:` prefix) and the
    /// revision read.
    pub fn points(&self) -> Vec<(Vec<u8>, Option<Box<str>>)> {
        self.points
            .lock()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// A key this transaction read twice at two revisions, if any.
    pub fn conflict(&self) -> Option<String> {
        self.conflict.lock().clone()
    }

    fn is_metadata(logical: &[u8]) -> bool {
        METADATA_KEY_PREFIXES.iter().any(|p| logical.starts_with(p))
    }

    /// A point read of `logical`, which returned `value`.
    fn note_get(&self, logical: &[u8], value: Option<&[u8]>) {
        if Self::is_metadata(logical) {
            return;
        }
        let Some(key) = logical
            .strip_prefix(DOC_KEY_PREFIX)
            .filter(|_| !logical.starts_with(LEGACY_TTL_KEY_PREFIX))
        else {
            self.mark_ranged();
            return;
        };
        let rev: Option<Box<str>> = match value {
            None => None,
            Some(bytes) => match stored_rev(bytes) {
                Some(rev) => Some(rev),
                None => {
                    self.mark_ranged();
                    return;
                }
            },
        };
        let mut points = self.points.lock();
        match points.get(key) {
            Some(seen) if *seen != rev => {
                let mut conflict = self.conflict.lock();
                if conflict.is_none() {
                    *conflict = Some(format!(
                        "Read conflict: document '{}' changed since this transaction \
                         first read it; roll back and retry",
                        String::from_utf8_lossy(key)
                    ));
                }
            }
            Some(_) => {}
            None if points.len() >= MAX_TRACKED_POINTS => self.mark_ranged(),
            None => {
                points.insert(key.to_vec(), rev);
            }
        }
    }

    /// A read of a range starting at `start` (`None`: the whole keyspace).
    fn note_range(&self, start: Option<&[u8]>) {
        if start.is_some_and(Self::is_metadata) {
            return;
        }
        self.mark_ranged();
    }
}

/// The revision of a stored document, from its header when it has one.
fn stored_rev(bytes: &[u8]) -> Option<Box<str>> {
    if let Some(rev) = crate::storage::serializer::StoredDocRef::parse(bytes)
        .and_then(|doc| std::str::from_utf8(doc.rev).ok())
    {
        return Some(rev.into());
    }
    crate::storage::serializer::deserialize_doc(bytes)
        .ok()
        .map(|doc| doc.rev.into())
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum KsId {
    Legacy(Arc<str>),
    Shared(KsNum),
}

static STATES: Lazy<DashMap<(usize, KsId), Weak<KsState>>> = Lazy::new(DashMap::new);

fn db_key(db: &Arc<DB>) -> usize {
    Arc::as_ptr(db) as usize
}

fn intern_state(db: &Arc<DB>, id: &KsId) -> Arc<KsState> {
    let key = (db_key(db), id.clone());
    if let Some(existing) = STATES.get(&key).and_then(|w| w.upgrade()) {
        return existing;
    }
    let mut entry = STATES.entry(key).or_default();
    if let Some(existing) = entry.upgrade() {
        return existing;
    }
    let state = Arc::new(KsState::default());
    *entry = Arc::downgrade(&state);
    state
}

/// Mark a keyspace dead: every handle of it now reports `CollectionNotFound`.
pub fn mark_dead(db: &Arc<DB>, id: &KsId) {
    if let Some(state) = STATES
        .get(&(db_key(db), id.clone()))
        .and_then(|w| w.upgrade())
    {
        state.dead.store(true, Ordering::Release);
    }
}

/// Mark dead every shared keyspace of one database (a database drop).
pub fn mark_database_dead(db: &Arc<DB>, db_id: u32) {
    let me = db_key(db);
    for entry in STATES.iter() {
        let (ptr, id) = entry.key();
        if *ptr != me {
            continue;
        }
        if let KsId::Shared(n) = id {
            if ks_db_id(*n) == db_id {
                if let Some(state) = entry.value().upgrade() {
                    state.dead.store(true, Ordering::Release);
                }
            }
        }
    }
}

/// Drop interned entries whose handles are all gone (called opportunistically).
pub fn prune_states() {
    STATES.retain(|_, w| w.strong_count() > 0);
}

/// Where one collection's keys live. Owned by `Collection`; cheap to clone.
#[derive(Clone, Debug)]
pub struct Keyspace {
    cf_name: Arc<str>,
    prefix: KsPrefix,
    id: KsId,
    state: Arc<KsState>,
    /// Set on the handle a tracking transaction reads through.
    tracker: Option<Arc<ReadTracker>>,
}

impl Keyspace {
    /// A 1.x collection in its own column family.
    pub fn legacy(db: &Arc<DB>, cf_name: &str) -> Self {
        let cf_name: Arc<str> = Arc::from(cf_name);
        let id = KsId::Legacy(cf_name.clone());
        Keyspace {
            state: intern_state(db, &id),
            cf_name,
            prefix: KsPrefix::LEGACY,
            id,
            tracker: None,
        }
    }

    /// A collection in the shared column family.
    pub fn shared(db: &Arc<DB>, ks: KsNum) -> Self {
        let id = KsId::Shared(ks);
        Keyspace {
            state: intern_state(db, &id),
            cf_name: Arc::from(SHARED_CF),
            prefix: KsPrefix::shared(ks),
            id,
            tracker: None,
        }
    }

    pub fn id(&self) -> &KsId {
        &self.id
    }

    pub fn state(&self) -> &Arc<KsState> {
        &self.state
    }

    /// A tracker for a transaction that starts reading this keyspace now.
    pub fn new_tracker(&self) -> Arc<ReadTracker> {
        Arc::new(ReadTracker::new(Arc::clone(&self.state)))
    }

    /// This keyspace, its reads recorded on `tracker`.
    pub fn tracked(&self, tracker: Arc<ReadTracker>) -> Keyspace {
        Keyspace {
            tracker: Some(tracker),
            ..self.clone()
        }
    }

    pub fn tracker(&self) -> Option<&Arc<ReadTracker>> {
        self.tracker.as_ref()
    }

    pub fn prefix(&self) -> KsPrefix {
        self.prefix
    }

    pub fn is_legacy(&self) -> bool {
        self.prefix.is_legacy()
    }

    pub fn cf_name(&self) -> &str {
        &self.cf_name
    }

    pub fn is_dead(&self) -> bool {
        self.state.is_dead()
    }

    /// The handle to read and write through, or `None` when the keyspace was
    /// dropped (or, for legacy, its column family is gone).
    pub fn handle<'a>(&self, db: &'a DB) -> Option<KsCf<'a>> {
        if self.state.is_dead() {
            return None;
        }
        db.cf_handle(&self.cf_name).map(|cf| KsCf {
            cf,
            prefix: self.prefix,
            tracker: self.tracker.clone(),
        })
    }

    /// [`Keyspace::handle`], as a `CollectionNotFound` error naming `what`.
    pub fn live<'a>(&self, db: &'a DB, what: &str) -> DbResult<KsCf<'a>> {
        self.handle(db)
            .ok_or_else(|| DbError::CollectionNotFound(format!("{} (dropped mid-operation)", what)))
    }

    /// Physical `[lo, hi)` of the whole keyspace. Legacy: the first and just
    /// past the last key of the column family, or `None` when it is empty.
    pub fn physical_range(&self, db: &DB) -> Option<(Vec<u8>, Vec<u8>)> {
        if let Some(hi) = self.prefix.upper() {
            return Some((self.prefix.as_bytes().to_vec(), hi));
        }
        let cf = db.cf_handle(&self.cf_name)?;
        let first = db.iterator_cf(&cf, IterMode::Start).next()?.ok()?.0;
        let mut last = db.iterator_cf(&cf, IterMode::End).next()?.ok()?.0.to_vec();
        last.push(0);
        Some((first.to_vec(), last))
    }

    /// Approximate on-disk size of the keyspace (flushed data only).
    pub fn approximate_size(&self, db: &DB) -> u64 {
        let Some(cf) = db.cf_handle(&self.cf_name) else {
            return 0;
        };
        let Some((lo, hi)) = self.physical_range(db) else {
            return 0;
        };
        db.approximate_size_cf(&cf, &lo, &hi)
    }
}

/// A column family together with the key prefix of one keyspace.
pub struct KsCf<'a> {
    pub(crate) cf: KvCf<'a>,
    pub(crate) prefix: KsPrefix,
    /// Records what is read through this handle (see [`ReadTracker`]).
    pub(crate) tracker: Option<Arc<ReadTracker>>,
}

impl KsCf<'_> {
    pub fn prefix(&self) -> KsPrefix {
        self.prefix
    }

    #[inline]
    pub fn key(&self, logical: &[u8]) -> Vec<u8> {
        self.prefix.key(logical)
    }
}

/// Iterator over one keyspace yielding logical (prefix-stripped) keys.
pub struct KsIter<'a> {
    inner: KvIter<'a>,
    prefix: KsPrefix,
}

impl Iterator for KsIter<'_> {
    type Item = Result<(bytes::Bytes, bytes::Bytes), KvError>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let prefix = self.prefix;
        self.inner
            .next()
            .map(|r| r.map(|(k, v)| (prefix.strip(k), v)))
    }
}

impl std::iter::FusedIterator for KsIter<'_> {}

/// Read options bounded to the keyspace, optionally narrowed by logical bounds.
fn bounded_opts(prefix: KsPrefix, lower: Option<&[u8]>, upper: Option<&[u8]>) -> KvReadOptions {
    let mut opts = KvReadOptions::default();
    match (lower, prefix.is_legacy()) {
        (Some(lo), _) => opts.set_iterate_lower_bound(prefix.key(lo)),
        (None, false) => opts.set_iterate_lower_bound(prefix.as_bytes().to_vec()),
        (None, true) => {}
    }
    match (upper, prefix.upper()) {
        (Some(hi), _) => opts.set_iterate_upper_bound(prefix.key(hi)),
        (None, Some(end)) => opts.set_iterate_upper_bound(end),
        (None, None) => {}
    }
    opts
}

/// `*_cf` operations of [`Kv`], through a keyspace.
pub trait KsDbExt {
    fn get_ks<K: AsRef<[u8]>>(&self, cf: &KsCf, key: K) -> Result<Option<Vec<u8>>, KvError>;
    fn get_pinned_ks<K: AsRef<[u8]>>(
        &self,
        cf: &KsCf,
        key: K,
    ) -> Result<Option<KvPinned<'_>>, KvError>;
    fn put_ks<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        cf: &KsCf,
        key: K,
        value: V,
    ) -> Result<(), KvError>;
    fn delete_ks<K: AsRef<[u8]>>(&self, cf: &KsCf, key: K) -> Result<(), KvError>;
    fn multi_get_ks<K: AsRef<[u8]>, I: IntoIterator<Item = K>>(
        &self,
        cf: &KsCf,
        keys: I,
    ) -> Vec<Result<Option<Vec<u8>>, KvError>>;
    /// [`Kv::multi_get_pinned_cf`] on logical keys: the values without a copy.
    fn multi_get_pinned_ks<K: AsRef<[u8]>>(
        &self,
        cf: &KsCf,
        keys: &[K],
    ) -> Vec<Result<Option<KvPinned<'_>>, KvError>>;
    /// Seek to the logical `prefix` and iterate forward to the end of the
    /// keyspace (like `prefix_iterator_cf` without a prefix extractor: the
    /// caller stops when keys no longer start with `prefix`).
    fn prefix_iterator_ks<P: AsRef<[u8]>>(&self, cf: &KsCf, prefix: P) -> KsIter<'_>;
    /// `iterator_cf`, bounded to the keyspace. `From` keys are logical.
    fn iterator_ks(&self, cf: &KsCf, mode: IterMode) -> KsIter<'_>;
    /// `iterator_ks` with extra logical bounds (`lower` inclusive, `upper`
    /// exclusive) — the replacement for `iterator_cf_opt` with
    /// `set_iterate_*_bound`.
    fn iterator_ks_bounded(
        &self,
        cf: &KsCf,
        mode: IterMode,
        lower: Option<&[u8]>,
        upper: Option<&[u8]>,
    ) -> KsIter<'_>;
    /// `iterator_cf_opt`: the caller's read options (readahead, …) with the
    /// keyspace bounds added. Do not set iterate bounds on `opts` — they would
    /// be physical; use [`KsDbExt::iterator_ks_bounded`] for logical bounds.
    fn iterator_ks_opt(&self, cf: &KsCf, opts: KvReadOptions, mode: IterMode) -> KsIter<'_>;
    /// Compact `[start, end)` (logical); `None` means the keyspace edge.
    fn compact_range_ks(&self, cf: &KsCf, start: Option<&[u8]>, end: Option<&[u8]>);
    /// Call `f` with each logical key and value in `[lower, upper)`
    /// (logical), in order, until it returns `false` — borrowed, see
    /// [`Kv::for_each_cf_opt`]. `readahead` as in
    /// [`KvReadOptions::set_readahead_size`].
    fn for_each_ks(
        &self,
        cf: &KsCf,
        lower: &[u8],
        upper: &[u8],
        readahead: usize,
        snapshot: Option<&KvSnapshot<'_>>,
        f: impl FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), KvError>;
    /// Approximate sizes of logical `[start, end)` ranges (see
    /// [`Kv::approximate_sizes_cf`]).
    fn approximate_sizes_ks(&self, cf: &KsCf, ranges: &[(Vec<u8>, Vec<u8>)]) -> Vec<u64>;
    /// [`Kv::split_points_cf`] of logical `[lo, hi)`, as logical keys.
    fn split_points_ks(
        &self,
        cf: &KsCf,
        lo: &[u8],
        hi: &[u8],
        n: usize,
        snapshot: Option<&KvSnapshot<'_>>,
    ) -> Option<Vec<Vec<u8>>>;
}

impl KsDbExt for Kv {
    fn get_ks<K: AsRef<[u8]>>(&self, cf: &KsCf, key: K) -> Result<Option<Vec<u8>>, KvError> {
        let key = key.as_ref();
        let found = if cf.prefix.is_legacy() {
            self.get_cf(&cf.cf, key)
        } else {
            self.get_cf(&cf.cf, cf.key(key))
        };
        if let Some(tracker) = &cf.tracker {
            match &found {
                Ok(value) => tracker.note_get(key, value.as_deref()),
                Err(_) => tracker.mark_ranged(),
            }
        }
        found
    }

    fn get_pinned_ks<K: AsRef<[u8]>>(
        &self,
        cf: &KsCf,
        key: K,
    ) -> Result<Option<KvPinned<'_>>, KvError> {
        let key = key.as_ref();
        let found = if cf.prefix.is_legacy() {
            self.get_pinned_cf(&cf.cf, key)
        } else {
            self.get_pinned_cf(&cf.cf, cf.key(key))
        };
        if let Some(tracker) = &cf.tracker {
            match &found {
                Ok(value) => tracker.note_get(key, value.as_deref()),
                Err(_) => tracker.mark_ranged(),
            }
        }
        found
    }

    fn put_ks<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        cf: &KsCf,
        key: K,
        value: V,
    ) -> Result<(), KvError> {
        if cf.prefix.is_legacy() {
            return self.put_cf(&cf.cf, key, value);
        }
        self.put_cf(&cf.cf, cf.key(key.as_ref()), value)
    }

    fn delete_ks<K: AsRef<[u8]>>(&self, cf: &KsCf, key: K) -> Result<(), KvError> {
        if cf.prefix.is_legacy() {
            return self.delete_cf(&cf.cf, key);
        }
        self.delete_cf(&cf.cf, cf.key(key.as_ref()))
    }

    fn multi_get_ks<K: AsRef<[u8]>, I: IntoIterator<Item = K>>(
        &self,
        cf: &KsCf,
        keys: I,
    ) -> Vec<Result<Option<Vec<u8>>, KvError>> {
        let physical: Vec<Vec<u8>> = keys.into_iter().map(|k| cf.key(k.as_ref())).collect();
        let found = self.multi_get_cf(physical.iter().map(|k| (&cf.cf, k)));
        if let Some(tracker) = &cf.tracker {
            let strip = cf.prefix.as_bytes().len();
            for (key, value) in physical.iter().zip(&found) {
                match value {
                    Ok(value) => tracker.note_get(&key[strip..], value.as_deref()),
                    Err(_) => tracker.mark_ranged(),
                }
            }
        }
        found
    }

    fn multi_get_pinned_ks<K: AsRef<[u8]>>(
        &self,
        cf: &KsCf,
        keys: &[K],
    ) -> Vec<Result<Option<KvPinned<'_>>, KvError>> {
        let found = if cf.prefix.is_legacy() {
            self.multi_get_pinned_cf(&cf.cf, keys)
        } else {
            let physical: Vec<Vec<u8>> = keys.iter().map(|k| cf.key(k.as_ref())).collect();
            self.multi_get_pinned_cf(&cf.cf, &physical)
        };
        if let Some(tracker) = &cf.tracker {
            for (key, value) in keys.iter().zip(&found) {
                match value {
                    Ok(value) => tracker.note_get(key.as_ref(), value.as_deref()),
                    Err(_) => tracker.mark_ranged(),
                }
            }
        }
        found
    }

    fn prefix_iterator_ks<P: AsRef<[u8]>>(&self, cf: &KsCf, prefix: P) -> KsIter<'_> {
        self.iterator_ks(cf, IterMode::From(prefix.as_ref(), Direction::Forward))
    }

    fn iterator_ks(&self, cf: &KsCf, mode: IterMode) -> KsIter<'_> {
        self.iterator_ks_bounded(cf, mode, None, None)
    }

    fn iterator_ks_bounded(
        &self,
        cf: &KsCf,
        mode: IterMode,
        lower: Option<&[u8]>,
        upper: Option<&[u8]>,
    ) -> KsIter<'_> {
        if let Some(tracker) = &cf.tracker {
            tracker.note_range(match &mode {
                IterMode::From(k, _) => Some(k),
                _ => lower,
            });
        }
        let opts = bounded_opts(cf.prefix, lower, upper);
        let inner = match mode {
            IterMode::From(k, dir) => {
                let physical = cf.key(k);
                self.iterator_cf_opt(&cf.cf, opts, IterMode::From(&physical, dir))
            }
            other => self.iterator_cf_opt(&cf.cf, opts, other),
        };
        KsIter {
            inner,
            prefix: cf.prefix,
        }
    }

    fn iterator_ks_opt(&self, cf: &KsCf, read: KvReadOptions, mode: IterMode) -> KsIter<'_> {
        if let Some(tracker) = &cf.tracker {
            tracker.note_range(match &mode {
                IterMode::From(k, _) => Some(k),
                _ => None,
            });
        }
        let mut opts = read;
        if let Some(end) = cf.prefix.upper() {
            opts.set_iterate_lower_bound(cf.prefix.as_bytes().to_vec());
            opts.set_iterate_upper_bound(end);
        }
        let inner = match mode {
            IterMode::From(k, dir) => {
                let physical = cf.key(k);
                self.iterator_cf_opt(&cf.cf, opts, IterMode::From(&physical, dir))
            }
            other => self.iterator_cf_opt(&cf.cf, opts, other),
        };
        KsIter {
            inner,
            prefix: cf.prefix,
        }
    }

    fn compact_range_ks(&self, cf: &KsCf, start: Option<&[u8]>, end: Option<&[u8]>) {
        if cf.prefix.is_legacy() {
            self.compact_range_cf(&cf.cf, start, end);
            return;
        }
        let lo = start.map_or_else(|| cf.prefix.as_bytes().to_vec(), |s| cf.key(s));
        let hi = end.map_or_else(|| cf.prefix.upper().unwrap_or_default(), |e| cf.key(e));
        self.compact_range_cf(&cf.cf, Some(&lo), Some(&hi));
    }

    fn for_each_ks(
        &self,
        cf: &KsCf,
        lower: &[u8],
        upper: &[u8],
        readahead: usize,
        snapshot: Option<&KvSnapshot<'_>>,
        mut f: impl FnMut(&[u8], &[u8]) -> bool,
    ) -> Result<(), KvError> {
        if let Some(tracker) = &cf.tracker {
            tracker.note_range(Some(lower));
        }
        let mut opts = bounded_opts(cf.prefix, Some(lower), Some(upper));
        opts.set_readahead_size(readahead);
        let start = cf.key(lower);
        let strip = cf.prefix.as_bytes().len();
        self.for_each_cf_opt(&cf.cf, opts, &start, snapshot, |k, v| f(&k[strip..], v))
    }

    fn split_points_ks(
        &self,
        cf: &KsCf,
        lo: &[u8],
        hi: &[u8],
        n: usize,
        snapshot: Option<&KvSnapshot<'_>>,
    ) -> Option<Vec<Vec<u8>>> {
        if let Some(tracker) = &cf.tracker {
            tracker.note_range(Some(lo));
        }
        let strip = cf.prefix.as_bytes().len();
        let points = self.split_points_cf(&cf.cf, &cf.key(lo), &cf.key(hi), n, snapshot)?;
        Some(points.into_iter().map(|k| k[strip..].to_vec()).collect())
    }

    fn approximate_sizes_ks(&self, cf: &KsCf, ranges: &[(Vec<u8>, Vec<u8>)]) -> Vec<u64> {
        let physical: Vec<(Vec<u8>, Vec<u8>)> =
            ranges.iter().map(|(s, e)| (cf.key(s), cf.key(e))).collect();
        self.approximate_sizes_cf(&cf.cf, &physical)
    }
}

/// [`KvBatch`] writes through a keyspace.
pub trait KsBatchExt {
    fn put_ks<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, cf: &KsCf, key: K, value: V);
    fn delete_ks<K: AsRef<[u8]>>(&mut self, cf: &KsCf, key: K);
    /// Range delete `[from, to)`, both logical.
    fn delete_range_ks<K: AsRef<[u8]>>(&mut self, cf: &KsCf, from: K, to: K);
}

impl KsBatchExt for KvBatch {
    fn put_ks<K: AsRef<[u8]>, V: AsRef<[u8]>>(&mut self, cf: &KsCf, key: K, value: V) {
        if cf.prefix.is_legacy() {
            self.put_cf(&cf.cf, key, value);
        } else {
            self.put_cf(&cf.cf, cf.key(key.as_ref()), value);
        }
    }

    fn delete_ks<K: AsRef<[u8]>>(&mut self, cf: &KsCf, key: K) {
        if cf.prefix.is_legacy() {
            self.delete_cf(&cf.cf, key);
        } else {
            self.delete_cf(&cf.cf, cf.key(key.as_ref()));
        }
    }

    fn delete_range_ks<K: AsRef<[u8]>>(&mut self, cf: &KsCf, from: K, to: K) {
        self.delete_range_cf(&cf.cf, cf.key(from.as_ref()), cf.key(to.as_ref()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ks_numbers_and_prefixes() {
        let ks = ks_num(7, 3);
        assert_eq!(ks_db_id(ks), 7);
        let p = KsPrefix::shared(ks);
        assert_eq!(p.as_bytes(), &[0, 0, 0, 7, 0, 0, 0, 3]);
        assert_eq!(p.key(b"doc:a"), b"\0\0\0\x07\0\0\0\x03doc:a".to_vec());
        assert_eq!(p.upper().unwrap(), vec![0, 0, 0, 7, 0, 0, 0, 4]);
        // The last collection id of a database rolls over into the next db id.
        let last = KsPrefix::shared(ks_num(7, u32::MAX));
        assert_eq!(last.upper().unwrap(), vec![0, 0, 0, 8, 0, 0, 0, 0]);
        assert!(KsPrefix::LEGACY.upper().is_none());
        assert_eq!(KsPrefix::LEGACY.key(b"doc:a"), b"doc:a".to_vec());
    }
}
