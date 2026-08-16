//! Optional per-collection document versioning — point-in-time / `AS OF` reads.
//!
//! When versioning is enabled on a collection, every insert/update/delete also
//! writes an immutable history record into the collection's column family, in the
//! *same* atomic `WriteBatch` as the live mutation. History keys are
//! `docv:<key>:<inverted_ts>` where `inverted_ts = u64::MAX - ts_micros`, so the
//! newest version sorts first within a document's prefix and an `AS OF T` read is
//! a single forward seek to the first version with `ts <= T`.
//!
//! Scope: single-document `insert`/`update`/`delete`, `insert_batch` /
//! `upsert_batch`, and transactional write batches record history.
//! and secondary indexes are current-version only — `AS OF` answers
//! primary-key reads (`DOC_AS_OF`) and history listing (`DOC_HISTORY`), not
//! index-accelerated historical queries. History is capped per document
//! (`SOLIDB_MAX_VERSIONS`, default 100).

use super::{Collection, DOCV_PREFIX, ROW_POLICY_META_KEY, VERSIONING_META_KEY};
use crate::error::{DbError, DbResult};
use dashmap::DashMap;
use rust_rocksdb::{AsColumnFamilyRef, Direction, IteratorMode, WriteBatch};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_VERSIONS: usize = 100;

/// One historical version of a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionRecord {
    /// Commit time in epoch microseconds (monotonic; see `next_version_micros`).
    pub ts: u64,
    /// True when this version is a delete tombstone.
    #[serde(default)]
    pub deleted: bool,
    /// The document value at this version (absent for tombstones).
    #[serde(default)]
    pub value: Option<Value>,
}

// Monotonic microsecond clock: rapid writes to the same key still get distinct,
// correctly-ordered version timestamps (real wall-clock micros, bumped by +1 on
// ties so two versions never share a key).
static VERSION_CLOCK: AtomicU64 = AtomicU64::new(0);

fn next_version_micros() -> u64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0);
    loop {
        let last = VERSION_CLOCK.load(Ordering::Relaxed);
        let ts = now.max(last + 1);
        if VERSION_CLOCK
            .compare_exchange_weak(last, ts, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return ts;
        }
    }
}

fn max_versions() -> usize {
    std::env::var("SOLIDB_MAX_VERSIONS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_VERSIONS)
}

/// Per-column-family cache of the versioning flag, so the write hot path never
/// hits RocksDB just to learn versioning is off.
fn versioned_cache() -> &'static DashMap<String, bool> {
    static CACHE: OnceLock<DashMap<String, bool>> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
}

impl Collection {
    /// Cache key that is unique per (storage engine, collection): the CF name
    /// alone collides when several `StorageEngine` instances share a process
    /// (e.g. tests), so fold in the RocksDB instance pointer.
    fn version_cache_key(&self) -> String {
        format!("{:p}/{}", std::sync::Arc::as_ptr(&self.db), self.name)
    }

    fn version_prefix(key: &str) -> String {
        format!("{}{}:", DOCV_PREFIX, key)
    }

    fn version_key(key: &str, ts_micros: u64) -> String {
        // Inverted, fixed-width hex → newest-first, lexicographic == numeric order.
        format!("{}{}:{:016x}", DOCV_PREFIX, key, u64::MAX - ts_micros)
    }

    /// Whether this collection records document history.
    pub fn is_versioned(&self) -> bool {
        let ck = self.version_cache_key();
        if let Some(v) = versioned_cache().get(&ck) {
            return *v;
        }
        let enabled = self
            .db
            .cf_handle(&self.name)
            .and_then(|cf| {
                self.db
                    .get_cf(&cf, VERSIONING_META_KEY.as_bytes())
                    .ok()
                    .flatten()
            })
            .is_some();
        versioned_cache().insert(ck, enabled);
        enabled
    }

    /// Enable versioning. Subsequent single-document writes record history.
    pub fn enable_versioning(&self) -> DbResult<()> {
        let cf = self
            .db
            .cf_handle(&self.name)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        self.db
            .put_cf(&cf, VERSIONING_META_KEY.as_bytes(), b"1")
            .map_err(|e| DbError::InternalError(format!("enable_versioning: {}", e)))?;
        versioned_cache().insert(self.version_cache_key(), true);
        Ok(())
    }

    /// Disable versioning (existing history is retained until pruned/dropped).
    pub fn disable_versioning(&self) -> DbResult<()> {
        if let Some(cf) = self.db.cf_handle(&self.name) {
            let _ = self.db.delete_cf(&cf, VERSIONING_META_KEY.as_bytes());
        }
        versioned_cache().insert(self.version_cache_key(), false);
        Ok(())
    }

    /// Append a version record into the same `WriteBatch` as the mutation (atomic).
    /// `value = None` records a delete tombstone.
    pub(crate) fn append_version_to_batch<C: AsColumnFamilyRef>(
        &self,
        batch: &mut WriteBatch,
        cf: &C,
        key: &str,
        value: Option<&Value>,
    ) {
        let ts = next_version_micros();
        let record = VersionRecord {
            ts,
            deleted: value.is_none(),
            value: value.cloned(),
        };
        if let Ok(bytes) = serde_json::to_vec(&record) {
            batch.put_cf(cf, Self::version_key(key, ts), bytes);
        }
    }

    /// Reconstruct the document as of `as_of_micros` (epoch microseconds): the
    /// newest version with `ts <= as_of_micros`. Returns `None` if the document
    /// did not exist then (never created, or its latest version at that time was
    /// a delete tombstone).
    pub fn get_as_of(&self, key: &str, as_of_micros: u64) -> DbResult<Option<Value>> {
        let cf = self
            .db
            .cf_handle(&self.name)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        let prefix = Self::version_prefix(key);
        // First key >= inverted(as_of) is the newest version with ts <= as_of.
        let seek = Self::version_key(key, as_of_micros);
        let iter = self
            .db
            .iterator_cf(&cf, IteratorMode::From(seek.as_bytes(), Direction::Forward));
        for item in iter {
            let (k, v) = match item {
                Ok(kv) => kv,
                Err(_) => break,
            };
            if !k.starts_with(prefix.as_bytes()) {
                break; // moved past this document's versions
            }
            let record: VersionRecord = match serde_json::from_slice(&v) {
                Ok(r) => r,
                Err(_) => continue,
            };
            return Ok(if record.deleted { None } else { record.value });
        }
        Ok(None)
    }

    /// All live documents as of `as_of_micros`. Full `docv:` scan; no indexes.
    pub fn scan_as_of(&self, as_of_micros: u64) -> DbResult<Vec<Value>> {
        let cf = self
            .db
            .cf_handle(&self.name)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        let prefix = DOCV_PREFIX.as_bytes();
        let mut chosen: std::collections::HashMap<String, VersionRecord> =
            std::collections::HashMap::new();
        for item in self.db.prefix_iterator_cf(&cf, prefix) {
            let Ok((k, v)) = item else {
                break;
            };
            if !k.starts_with(prefix) {
                break;
            }
            let key_str = String::from_utf8_lossy(&k);
            // docv:<doc_key>:<inverted_hex>
            let rest = key_str.strip_prefix(DOCV_PREFIX).unwrap_or(&key_str);
            let Some((doc_key, _)) = rest.rsplit_once(':') else {
                continue;
            };
            let Ok(record) = serde_json::from_slice::<VersionRecord>(&v) else {
                continue;
            };
            if record.ts > as_of_micros {
                continue;
            }
            chosen
                .entry(doc_key.to_string())
                .and_modify(|cur| {
                    if record.ts > cur.ts {
                        *cur = record.clone();
                    }
                })
                .or_insert(record);
        }
        Ok(chosen
            .into_values()
            .filter(|r| !r.deleted)
            .filter_map(|r| r.value)
            .collect())
    }

    /// Full version history for a document, newest first. Each entry is
    /// `{ ts: <epoch millis>, ts_micros, deleted, value }`.
    pub fn doc_history(&self, key: &str) -> Vec<Value> {
        let Some(cf) = self.db.cf_handle(&self.name) else {
            return Vec::new();
        };
        let prefix = Self::version_prefix(key);
        let mut out = Vec::new();
        for item in self.db.prefix_iterator_cf(&cf, prefix.as_bytes()) {
            let Ok((k, v)) = item else {
                break;
            };
            if !k.starts_with(prefix.as_bytes()) {
                break;
            }
            if let Ok(record) = serde_json::from_slice::<VersionRecord>(&v) {
                out.push(serde_json::json!({
                    "ts": record.ts / 1000,
                    "ts_micros": record.ts,
                    "deleted": record.deleted,
                    "value": record.value,
                }));
            }
        }
        out
    }

    /// Drop the oldest versions of `key` beyond the retention cap.
    pub(crate) fn prune_versions(&self, key: &str) {
        let max = max_versions();
        let Some(cf) = self.db.cf_handle(&self.name) else {
            return;
        };
        let prefix = Self::version_prefix(key);
        let mut to_delete: Vec<Box<[u8]>> = Vec::new();
        for (idx, item) in self
            .db
            .prefix_iterator_cf(&cf, prefix.as_bytes())
            .enumerate()
        {
            let Ok((k, _)) = item else {
                break;
            };
            if !k.starts_with(prefix.as_bytes()) {
                break;
            }
            if idx >= max {
                to_delete.push(k); // newest-first, so these are the oldest
            }
        }
        for k in to_delete {
            let _ = self.db.delete_cf(&cf, k);
        }
    }

    pub fn set_row_policy(&self, expr: Option<&str>) -> DbResult<()> {
        let cf = self
            .db
            .cf_handle(&self.name)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        match expr {
            Some(s) => {
                self.db
                    .put_cf(&cf, ROW_POLICY_META_KEY.as_bytes(), s.as_bytes())
                    .map_err(|e| DbError::InternalError(format!("set_row_policy: {e}")))?;
            }
            None => {
                let _ = self.db.delete_cf(&cf, ROW_POLICY_META_KEY.as_bytes());
            }
        }
        Ok(())
    }

    pub fn get_row_policy(&self) -> Option<String> {
        let cf = self.db.cf_handle(&self.name)?;
        self.db
            .get_cf(&cf, ROW_POLICY_META_KEY.as_bytes())
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
    }
}
