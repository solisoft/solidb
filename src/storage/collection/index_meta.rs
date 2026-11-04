//! Process-wide cache of index definitions.
//!
//! Every index-metadata getter used to run a `prefix_iterator_cf` over the
//! `*_meta:` key range plus a serde_json parse per definition — and the write
//! path calls those getters ~5-6 times per single-document insert. This module
//! loads all definitions for a collection once and shares the snapshot until
//! an index (or the collection itself) is created or dropped.
//!
//! The cache is a process-global keyed by `(DB instance, column family)`
//! rather than a field on `Collection`: `StorageEngine` and `Database` each
//! keep their own handle cache and build independent `Collection` instances
//! for the same CF, so a per-instance cache could not be invalidated across
//! them. The DB half of the key is
//! the `Arc` data pointer, with a stored `Weak` to detect a dead-and-reused
//! allocation — several engines can be open in one process (tests, embedded
//! use) with colliding CF names like `"users"`.

use super::*;
use crate::storage::RocksDb as DB;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::sync::{Arc, Weak};

/// One collection's index definitions of every type.
#[derive(Debug, Default)]
pub(crate) struct IndexMetaSnapshot {
    pub indexes: Vec<Index>,
    pub fulltext: Vec<FulltextIndex>,
    pub geo: Vec<GeoIndex>,
    pub ttl: Vec<TtlIndex>,
}

struct CacheEntry {
    /// The DB this snapshot was loaded from; a key is only trusted when this
    /// still upgrades to the caller's exact `Arc<DB>` allocation.
    db: Weak<DB>,
    snapshot: Arc<IndexMetaSnapshot>,
}

type CacheKey = (usize, String);

static INDEX_META_CACHE: Lazy<DashMap<CacheKey, CacheEntry>> = Lazy::new(DashMap::new);

fn cache_key(db: &DB, cf_name: &str) -> CacheKey {
    (db as *const DB as usize, cf_name.to_string())
}

/// Drop the cached definitions for a column family. Must be called after
/// anything that creates or drops an index — or the CF itself, so a
/// same-name recreate cannot serve the previous incarnation's definitions.
pub(crate) fn invalidate_index_meta(db: &DB, cf_name: &str) {
    INDEX_META_CACHE.remove(&cache_key(db, cf_name));
}

impl Collection {
    /// Cached index definitions for this collection. `None` when the column
    /// family no longer exists (mirrors the `cf_handle` checks the per-call
    /// scans used to do). Definitions are kept in `*_meta:` key order so
    /// "first matching index" tie-breaks stay identical to a direct scan.
    pub(crate) fn index_meta(&self) -> Option<Arc<IndexMetaSnapshot>> {
        let key = cache_key(&self.db, &self.name);
        if let Some(entry) = INDEX_META_CACHE.get(&key) {
            // Reject an entry whose DB died (and whose address may since have
            // been reused by a different engine instance).
            if entry
                .db
                .upgrade()
                .is_some_and(|db| Arc::ptr_eq(&db, &self.db))
            {
                return Some(Arc::clone(&entry.snapshot));
            }
        }

        let cf = self.ks.handle(&self.db)?;
        let mut snapshot = IndexMetaSnapshot::default();

        macro_rules! load {
            ($prefix:expr, $target:expr) => {
                let prefix = $prefix.as_bytes();
                for (key, value) in self.db.prefix_iterator_ks(&cf, prefix).flatten() {
                    if !key.starts_with(prefix) {
                        break;
                    }
                    if let Ok(def) = serde_json::from_slice(&value) {
                        $target.push(def);
                    }
                }
            };
        }

        load!(IDX_META_PREFIX, snapshot.indexes);
        load!(FT_META_PREFIX, snapshot.fulltext);
        load!(GEO_META_PREFIX, snapshot.geo);
        load!(TTL_META_PREFIX, snapshot.ttl);

        let snapshot = Arc::new(snapshot);
        INDEX_META_CACHE.insert(
            key,
            CacheEntry {
                db: Arc::downgrade(&self.db),
                snapshot: Arc::clone(&snapshot),
            },
        );
        Some(snapshot)
    }

    /// Invalidate this collection's cached index definitions.
    pub(crate) fn invalidate_index_meta(&self) {
        invalidate_index_meta(&self.db, &self.name);
    }
}

/// Per collection, process-wide like the cache above: a write generation,
/// and the answer to "does this sparse index hold every document?" for the
/// generation it was computed at.
///
/// The generation is bumped by [`bump_write_generation`], which runs from
/// `Collection::invalidate_cached_queries` — after every document write
/// (`emit_change`), truncate and index DDL or rebuild, each already
/// committed. A reader loads the generation *before* counting, so a count
/// that races a write is stored under a generation the write then moves
/// past, and is recomputed on the next read instead of being trusted.
#[derive(Default)]
struct Completeness {
    generation: std::sync::atomic::AtomicU64,
    answers: DashMap<String, (u64, bool)>,
}

struct CompletenessEntry {
    db: Weak<DB>,
    state: Arc<Completeness>,
}

static COMPLETENESS: Lazy<DashMap<CacheKey, CompletenessEntry>> = Lazy::new(DashMap::new);

/// The live completeness state of a collection, created when absent or when
/// the stored one belongs to a DB that died (its address reused).
fn completeness(db: &Arc<DB>, cf_name: &str) -> Arc<Completeness> {
    let key = cache_key(db, cf_name);
    if let Some(entry) = COMPLETENESS.get(&key) {
        if entry
            .db
            .upgrade()
            .is_some_and(|live| Arc::ptr_eq(&live, db))
        {
            return Arc::clone(&entry.state);
        }
    }
    let state = Arc::new(Completeness::default());
    COMPLETENESS.insert(
        key,
        CompletenessEntry {
            db: Arc::downgrade(db),
            state: Arc::clone(&state),
        },
    );
    state
}

/// Move a collection's write generation, dropping every cached answer.
/// Nothing to do when no answer was ever cached for it.
pub(crate) fn bump_write_generation(db: &DB, cf_name: &str) {
    if let Some(entry) = COMPLETENESS.get(&cache_key(db, cf_name)) {
        entry
            .state
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
}

impl Collection {
    /// Whether `index` has an entry for every document. Always for a
    /// non-sparse index; for a sparse one, when no document has all its index
    /// fields null — its entry count equals [`Self::count`]. That count reads
    /// every key of the index, so the answer is cached until the collection's
    /// next write.
    pub(crate) fn index_holds_every_document(&self, index: &Index) -> bool {
        if !index.sparse {
            return true;
        }
        let state = completeness(&self.db, &self.name);
        let generation = state.generation.load(std::sync::atomic::Ordering::Acquire);
        if let Some(answer) = state.answers.get(&index.name) {
            if answer.0 == generation {
                return answer.1;
            }
        }
        let complete = self.index_entry_count(index) >= self.count();
        state
            .answers
            .insert(index.name.clone(), (generation, complete));
        complete
    }
}
