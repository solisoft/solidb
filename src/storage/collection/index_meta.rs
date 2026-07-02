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
//! them. (Same shape as `storage::document_cache`.) The DB half of the key is
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

        let cf = self.db.cf_handle(&self.name)?;
        let mut snapshot = IndexMetaSnapshot::default();

        macro_rules! load {
            ($prefix:expr, $target:expr) => {
                let prefix = $prefix.as_bytes();
                for (key, value) in self.db.prefix_iterator_cf(&cf, prefix).flatten() {
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
