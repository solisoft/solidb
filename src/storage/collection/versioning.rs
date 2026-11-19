//! Optional per-collection document versioning — point-in-time / `AS OF` reads.
//!
//! When versioning is enabled on a collection, every insert/update/delete also
//! writes an immutable history record into the collection's column family, in the
//! *same* atomic `KvBatch` as the live mutation. History keys are
//! `docv:<key>:<inverted_ts>` where `inverted_ts = u64::MAX - ts_micros`, so the
//! newest version sorts first within a document's prefix and an `AS OF T` read is
//! a single forward seek to the first version with `ts <= T`.
//!
//! Scope: single-document `insert`/`update`/`delete`, `insert_batch` /
//! `upsert_batch`, and transactional write batches record history. Enabling
//! versioning records the collection as it stands as a baseline version, so
//! history starts at that instant: an `AS OF` before it finds nothing.
//! Secondary indexes are current-version only — `AS OF` answers
//! primary-key reads (`DOC_AS_OF`) and history listing (`DOC_HISTORY`), not
//! index-accelerated historical queries. History is capped per document
//! (`SOLIDB_MAX_VERSIONS`, default 100).

use super::{Collection, DOCV_PREFIX, ROW_POLICY_META_KEY, VERSIONING_META_KEY};
use crate::error::{DbError, DbResult};
use crate::storage::keyspace::{KsBatchExt, KsCf, KsDbExt, KsId};
use crate::storage::kv::KvBatch;
use crate::storage::kv::{Direction, IterMode};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_VERSIONS: usize = 100;

/// Named instants of a versioned collection: `docv_tag:<name>` -> epoch
/// microseconds, as decimal text (card #1001). Not under `docv:`, which
/// holds document versions only.
const VERSION_TAG_PREFIX: &str = "docv_tag:";

/// What [`Collection::restore_as_of`] wrote: the documents put back, as now
/// stored, and the keys removed.
#[derive(Debug, Default)]
pub struct Restored {
    pub written: Vec<Value>,
    pub removed: Vec<String>,
}

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

thread_local! {
    /// Version timestamps a replicated write takes from its origin
    /// ([`at_origin`]): document key -> epoch microseconds.
    static ORIGIN_STAMPS: std::cell::RefCell<Option<std::collections::HashMap<String, u64>>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `f` with the versions it writes stamped with their origin's time
/// (card #998): a replica applying a peer's writes records them at the
/// instant the origin made them, not when they arrived, so `AS OF` answers
/// the same on every node. `stamps` maps a document key to epoch
/// microseconds; a key it does not name takes this node's clock.
pub fn at_origin<T>(stamps: std::collections::HashMap<String, u64>, f: impl FnOnce() -> T) -> T {
    let previous = ORIGIN_STAMPS.with(|s| s.borrow_mut().replace(stamps));
    let out = f();
    ORIGIN_STAMPS.with(|s| *s.borrow_mut() = previous);
    out
}

/// The version timestamp of a replicated write: its HLC physical time
/// (milliseconds) as microseconds, the logical counter in the sub-
/// millisecond digits so writes of one millisecond keep their order.
pub fn origin_micros(hlc_ms: u64, hlc_count: u32) -> u64 {
    hlc_ms.saturating_mul(1000) + u64::from(hlc_count.min(999))
}

fn version_micros_for(key: &str) -> u64 {
    ORIGIN_STAMPS
        .with(|s| s.borrow().as_ref().and_then(|m| m.get(key).copied()))
        .unwrap_or_else(next_version_micros)
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
fn versioned_cache() -> &'static DashMap<(usize, KsId), bool> {
    static CACHE: OnceLock<DashMap<(usize, KsId), bool>> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
}

impl Collection {
    /// Cache key that is unique per (storage engine, collection): the CF name
    /// alone collides when several `StorageEngine` instances share a process
    /// (e.g. tests), so fold in the RocksDB instance pointer.
    fn version_cache_key(&self) -> (usize, KsId) {
        // By keyspace, not name: a dropped and recreated collection must not
        // inherit its predecessor's versioning flag. Built without
        // formatting: every write asks.
        (
            std::sync::Arc::as_ptr(&self.db) as usize,
            self.ks.id().clone(),
        )
    }

    fn version_prefix(key: &str) -> String {
        format!("{}{}:", DOCV_PREFIX, key)
    }

    fn version_key(key: &str, ts_micros: u64) -> String {
        // Inverted, fixed-width hex → newest-first, lexicographic == numeric order.
        format!("{}{}:{:016x}", DOCV_PREFIX, key, u64::MAX - ts_micros)
    }

    /// Whether `full_key` is a version of exactly `prefix`'s document.
    ///
    /// `docv:<key>:` is also a prefix of every version of `<key>:<anything>`
    /// (audit D6: `prune_versions("a")` walked and deleted the history of all
    /// `a:*` documents, and `get_as_of("victim")` could return a record stored
    /// under `victim:ffff…`). A version of `<key>` has exactly the 16 hex
    /// digits of the timestamp after the prefix; any other document's
    /// remainder is longer, since it holds another `:<16 hex>` at least.
    fn is_own_version(full_key: &[u8], prefix: &[u8]) -> bool {
        full_key.len() == prefix.len() + 16
            && full_key[prefix.len()..]
                .iter()
                .all(|b| b.is_ascii_hexdigit())
    }

    /// Exclusive upper bound for one document's versions: every own version
    /// sorts at or below `docv:<key>:ffffffffffffffff`, so iteration stops
    /// there instead of walking the rest of the `<key>:*` family.
    fn version_upper_bound(key: &str) -> Vec<u8> {
        let mut b = format!("{}{}:ffffffffffffffff", DOCV_PREFIX, key).into_bytes();
        b.push(0);
        b
    }

    /// Whether this collection records document history.
    pub fn is_versioned(&self) -> bool {
        let ck = self.version_cache_key();
        if let Some(v) = versioned_cache().get(&ck) {
            return *v;
        }
        let enabled = self
            .ks
            .handle(&self.db)
            .and_then(|cf| {
                self.db
                    .get_ks(&cf, VERSIONING_META_KEY.as_bytes())
                    .ok()
                    .flatten()
            })
            .is_some();
        versioned_cache().insert(ck, enabled);
        enabled
    }

    /// Enable versioning. Subsequent writes record history, and the
    /// collection as it stands is recorded as a baseline (card #997): a
    /// document written before and never since used to be missing from
    /// `AS OF` reads, even at the current instant. Enabling an already
    /// versioned collection does nothing.
    pub fn enable_versioning(&self) -> DbResult<()> {
        if self.is_versioned() {
            return Ok(());
        }
        let cf = self
            .ks
            .handle(&self.db)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        // Stamped before the flag goes on: a write that races the baseline
        // is versioned after it, so its own version is the newer one.
        let baseline_ts = next_version_micros();
        self.db
            .put_ks(&cf, VERSIONING_META_KEY.as_bytes(), b"1")
            .map_err(|e| DbError::InternalError(format!("enable_versioning: {}", e)))?;
        versioned_cache().insert(self.version_cache_key(), true);
        self.write_baseline(&cf, baseline_ts)
    }

    /// The collection's state at `ts` as history: a version of every live
    /// document, and a tombstone for every key whose history still shows
    /// it alive although it is gone (deleted while versioning was off).
    /// Written in chunks, so enabling a large collection holds neither the
    /// collection nor one huge batch in memory.
    fn write_baseline(&self, cf: &KsCf, ts: u64) -> DbResult<()> {
        const CHUNK: usize = 1_000;
        let write = |batch: KvBatch| {
            self.db
                .write(batch)
                .map_err(|e| DbError::InternalError(format!("versioning baseline: {e}")))
        };
        let put = |batch: &mut KvBatch, key: &str, value: Option<Value>| {
            let record = VersionRecord {
                ts,
                deleted: value.is_none(),
                value,
            };
            if let Ok(bytes) = serde_json::to_vec(&record) {
                batch.put_ks(cf, Self::version_key(key, ts), bytes);
            }
        };

        let mut batch = KvBatch::default();
        let mut staged = 0usize;
        self.for_each_value(None, |doc| {
            let Some(key) = doc.get("_key").and_then(Value::as_str).map(str::to_string) else {
                return Ok(());
            };
            put(&mut batch, &key, Some(doc));
            staged += 1;
            if staged == CHUNK {
                write(std::mem::take(&mut batch))?;
                staged = 0;
            }
            Ok(())
        })?;

        // Keys with history but no live document: the newest version at or
        // before `ts` decides. Versions sort newest first within a key, so
        // the first one at or before `ts` met for a key is it. A set, not
        // "the key before": the history of `a` and of `a:b` interleave.
        let prefix = DOCV_PREFIX.as_bytes();
        let mut decided: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut gone = Vec::new();
        for item in self.db.prefix_iterator_ks(cf, prefix) {
            let Ok((k, v)) = item else {
                break;
            };
            if !k.starts_with(prefix) {
                break;
            }
            let key_str = String::from_utf8_lossy(&k);
            let rest = key_str.strip_prefix(DOCV_PREFIX).unwrap_or(&key_str);
            let Some((doc_key, ts_hex)) = rest.rsplit_once(':') else {
                continue;
            };
            if ts_hex.len() != 16 || !ts_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            if decided.contains(doc_key) {
                continue;
            }
            let Ok(record) = serde_json::from_slice::<VersionRecord>(&v) else {
                continue;
            };
            if record.ts > ts {
                continue; // written after the baseline: not this pass's to judge
            }
            decided.insert(doc_key.to_string());
            if record.deleted {
                continue;
            }
            let live = self
                .db
                .get_ks(cf, Self::doc_key(doc_key))
                .ok()
                .flatten()
                .is_some();
            if !live {
                gone.push(doc_key.to_string());
            }
        }
        for key in gone {
            put(&mut batch, &key, None);
            staged += 1;
            if staged == CHUNK {
                write(std::mem::take(&mut batch))?;
                staged = 0;
            }
        }
        if staged > 0 {
            write(batch)?;
        }
        Ok(())
    }

    /// Versioning as another node set it (`Operation::SetVersioning`):
    /// the payload is `{"enabled": bool}`.
    pub fn apply_versioning(&self, payload: Option<&[u8]>) -> DbResult<()> {
        let enabled = payload
            .and_then(|b| serde_json::from_slice::<Value>(b).ok())
            .and_then(|v| v.get("enabled").and_then(Value::as_bool))
            .ok_or_else(|| DbError::BadRequest("SetVersioning: no `enabled`".to_string()))?;
        if enabled {
            self.enable_versioning()
        } else {
            self.disable_versioning()
        }
    }

    /// Disable versioning (existing history is retained until pruned/dropped).
    pub fn disable_versioning(&self) -> DbResult<()> {
        if let Some(cf) = self.ks.handle(&self.db) {
            let _ = self.db.delete_ks(&cf, VERSIONING_META_KEY.as_bytes());
        }
        versioned_cache().insert(self.version_cache_key(), false);
        Ok(())
    }

    /// Append a version record into the same `KvBatch` as the mutation (atomic).
    /// `value = None` records a delete tombstone.
    pub(crate) fn append_version_to_batch(
        &self,
        batch: &mut KvBatch,
        cf: &KsCf,
        key: &str,
        value: Option<&Value>,
    ) {
        let ts = version_micros_for(key);
        let record = VersionRecord {
            ts,
            deleted: value.is_none(),
            value: value.cloned(),
        };
        if let Ok(bytes) = serde_json::to_vec(&record) {
            batch.put_ks(cf, Self::version_key(key, ts), bytes);
        }
    }

    /// Reconstruct the document as of `as_of_micros` (epoch microseconds): the
    /// newest version with `ts <= as_of_micros`. Returns `None` if the document
    /// did not exist then (never created, or its latest version at that time was
    /// a delete tombstone).
    pub fn get_as_of(&self, key: &str, as_of_micros: u64) -> DbResult<Option<Value>> {
        let cf = self
            .ks
            .handle(&self.db)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        let prefix = Self::version_prefix(key);
        // First key >= inverted(as_of) is the newest version with ts <= as_of.
        let seek = Self::version_key(key, as_of_micros);
        let upper = Self::version_upper_bound(key);
        let iter = self.db.iterator_ks_bounded(
            &cf,
            IterMode::From(seek.as_bytes(), Direction::Forward),
            None,
            Some(&upper),
        );
        for item in iter {
            let (k, v) = match item {
                Ok(kv) => kv,
                Err(_) => break,
            };
            if !k.starts_with(prefix.as_bytes()) {
                break; // moved past this document's versions
            }
            if !Self::is_own_version(&k, prefix.as_bytes()) {
                continue; // a version of `<key>:…`, not of `<key>`
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
            .ks
            .handle(&self.db)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        let prefix = DOCV_PREFIX.as_bytes();
        let mut chosen: std::collections::HashMap<String, VersionRecord> =
            std::collections::HashMap::new();
        for item in self.db.prefix_iterator_ks(&cf, prefix) {
            let Ok((k, v)) = item else {
                break;
            };
            if !k.starts_with(prefix) {
                break;
            }
            let key_str = String::from_utf8_lossy(&k);
            // docv:<doc_key>:<inverted_hex>
            let rest = key_str.strip_prefix(DOCV_PREFIX).unwrap_or(&key_str);
            let Some((doc_key, ts_hex)) = rest.rsplit_once(':') else {
                continue;
            };
            if ts_hex.len() != 16 || !ts_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
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
        let Some(cf) = self.ks.handle(&self.db) else {
            return Vec::new();
        };
        let prefix = Self::version_prefix(key);
        let mut out = Vec::new();
        let upper = Self::version_upper_bound(key);
        let iter = self.db.iterator_ks_bounded(
            &cf,
            IterMode::From(prefix.as_bytes(), Direction::Forward),
            None,
            Some(&upper),
        );
        for item in iter {
            let Ok((k, v)) = item else {
                break;
            };
            if !k.starts_with(prefix.as_bytes()) {
                break;
            }
            if !Self::is_own_version(&k, prefix.as_bytes()) {
                continue;
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
        let Some(cf) = self.ks.handle(&self.db) else {
            return;
        };
        let prefix = Self::version_prefix(key);
        let mut to_delete: Vec<bytes::Bytes> = Vec::new();
        let upper = Self::version_upper_bound(key);
        let iter = self.db.iterator_ks_bounded(
            &cf,
            IterMode::From(prefix.as_bytes(), Direction::Forward),
            None,
            Some(&upper),
        );
        // A named instant (card #1001) keeps the version in effect then: the
        // newest one at or before it.
        let tags: Vec<u64> = self.version_tags().into_iter().map(|(_, ts)| ts).collect();
        let mut own = 0usize;
        let mut newer_ts = u64::MAX;
        for item in iter {
            let Ok((k, _)) = item else {
                break;
            };
            if !k.starts_with(prefix.as_bytes()) {
                break;
            }
            if !Self::is_own_version(&k, prefix.as_bytes()) {
                continue; // another document's history: never ours to prune
            }
            let ts = std::str::from_utf8(&k[prefix.len()..])
                .ok()
                .and_then(|hex| u64::from_str_radix(hex, 16).ok())
                .map_or(0, |inverted| u64::MAX - inverted);
            let tagged = tags.iter().any(|&t| ts <= t && t < newer_ts);
            if own >= max && !tagged {
                to_delete.push(k); // newest-first, so these are the oldest
            }
            own += 1;
            newer_ts = ts;
        }
        for k in to_delete {
            let _ = self.db.delete_ks(&cf, k);
        }
    }

    // --- Named instants (card #1001) -----------------------------------

    fn tag_key(name: &str) -> String {
        format!("{VERSION_TAG_PREFIX}{name}")
    }

    /// Name an instant of a versioned collection: `micros` (epoch
    /// microseconds), or now. `SYSTEM_TIME AS OF VERSION_TAG(…)` reads the
    /// collection as it was then, and pruning keeps the versions it needs.
    /// Returns the instant named.
    pub fn create_version_tag(&self, name: &str, micros: Option<u64>) -> DbResult<u64> {
        if !self.is_versioned() {
            return Err(DbError::BadRequest(format!(
                "'{}' keeps no history: enable versioning first",
                self.name
            )));
        }
        let name = name.trim();
        if name.is_empty() || name.len() > 128 {
            return Err(DbError::BadRequest(
                "a version tag needs a name of 1 to 128 characters".to_string(),
            ));
        }
        let ts = micros.unwrap_or_else(next_version_micros);
        self.put_version_tag(name, ts)?;
        Ok(ts)
    }

    fn put_version_tag(&self, name: &str, ts: u64) -> DbResult<()> {
        let cf = self
            .ks
            .handle(&self.db)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        self.db
            .put_ks(
                &cf,
                Self::tag_key(name).as_bytes(),
                ts.to_string().as_bytes(),
            )
            .map_err(|e| DbError::InternalError(format!("version tag: {e}")))?;
        // A cached `AS OF VERSION_TAG(…)` result read the old instant.
        self.invalidate_cached_queries();
        Ok(())
    }

    /// Remove a named instant; `false` when there was none.
    pub fn delete_version_tag(&self, name: &str) -> DbResult<bool> {
        if self.version_tag(name).is_none() {
            return Ok(false);
        }
        let cf = self
            .ks
            .handle(&self.db)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        self.db
            .delete_ks(&cf, Self::tag_key(name).as_bytes())
            .map_err(|e| DbError::InternalError(format!("version tag: {e}")))?;
        self.invalidate_cached_queries();
        Ok(true)
    }

    /// The instant `name` names, in epoch microseconds.
    pub fn version_tag(&self, name: &str) -> Option<u64> {
        let cf = self.ks.handle(&self.db)?;
        self.db
            .get_ks(&cf, Self::tag_key(name).as_bytes())
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| s.parse().ok())
    }

    /// Every named instant, by name.
    pub fn version_tags(&self) -> Vec<(String, u64)> {
        let Some(cf) = self.ks.handle(&self.db) else {
            return Vec::new();
        };
        let prefix = VERSION_TAG_PREFIX.as_bytes();
        let mut out = Vec::new();
        for item in self.db.prefix_iterator_ks(&cf, prefix) {
            let Ok((k, v)) = item else {
                break;
            };
            if !k.starts_with(prefix) {
                break;
            }
            let name = String::from_utf8_lossy(&k[prefix.len()..]).to_string();
            if let Some(ts) = std::str::from_utf8(&v).ok().and_then(|s| s.parse().ok()) {
                out.push((name, ts));
            }
        }
        out
    }

    /// A named instant another node set or removed
    /// (`Operation::SetVersionTag`): `{"name", "micros"}`, or `{"name",
    /// "deleted": true}`.
    pub fn apply_version_tag(&self, payload: Option<&[u8]>) -> DbResult<()> {
        let v: Value = payload
            .and_then(|b| serde_json::from_slice(b).ok())
            .ok_or_else(|| DbError::BadRequest("SetVersionTag: no payload".to_string()))?;
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| DbError::BadRequest("SetVersionTag: no `name`".to_string()))?;
        if v.get("deleted").and_then(Value::as_bool) == Some(true) {
            self.delete_version_tag(name)?;
            return Ok(());
        }
        let ts = v
            .get("micros")
            .and_then(Value::as_u64)
            .ok_or_else(|| DbError::BadRequest("SetVersionTag: no `micros`".to_string()))?;
        self.put_version_tag(name, ts)
    }

    // --- Restore (card #1001) -------------------------------------------

    /// Every document key with a live document or a history.
    fn keys_with_history_or_live(&self) -> DbResult<Vec<String>> {
        let mut keys = std::collections::BTreeSet::new();
        self.for_each_value(Some(&["_key".to_string()]), |doc| {
            if let Some(k) = doc.get("_key").and_then(Value::as_str) {
                keys.insert(k.to_string());
            }
            Ok(())
        })?;
        if let Some(cf) = self.ks.handle(&self.db) {
            let prefix = DOCV_PREFIX.as_bytes();
            for item in self.db.prefix_iterator_ks(&cf, prefix) {
                let Ok((k, _)) = item else {
                    break;
                };
                if !k.starts_with(prefix) {
                    break;
                }
                let full = String::from_utf8_lossy(&k[prefix.len()..]).to_string();
                if let Some((doc_key, hex)) = full.rsplit_once(':') {
                    if hex.len() == 16 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                        keys.insert(doc_key.to_string());
                    }
                }
            }
        }
        Ok(keys.into_iter().collect())
    }

    /// Put the collection back as it was at `as_of_micros` (card #1001):
    /// every document that differs from its version then is written again
    /// with that version, every one that did not exist then is removed, and
    /// every one removed since comes back. Only `keys` when given. These are
    /// ordinary writes, versioned like any other, so a restore is itself in
    /// the history and can be undone by restoring to an instant before it.
    /// Fields `_rev` and `_updated_at` are not compared and get new values.
    pub fn restore_as_of(&self, as_of_micros: u64, keys: Option<&[String]>) -> DbResult<Restored> {
        if !self.is_versioned() {
            return Err(DbError::BadRequest(format!(
                "'{}' keeps no history: there is nothing to restore from",
                self.name
            )));
        }
        let keys: Vec<String> = match keys {
            Some(keys) => keys.to_vec(),
            None => self.keys_with_history_or_live()?,
        };
        let content = |v: &Value| -> Value {
            let mut v = v.clone();
            if let Some(o) = v.as_object_mut() {
                o.remove("_rev");
                o.remove("_updated_at");
            }
            v
        };
        let mut puts: Vec<(String, Value)> = Vec::new();
        let mut removes: Vec<String> = Vec::new();
        for key in keys {
            let then = self.get_as_of(&key, as_of_micros)?;
            let now = self.get(&key).ok().map(|d| d.to_value());
            match (then, now) {
                (None, Some(_)) => removes.push(key),
                (Some(then), now) => {
                    let then = content(&then);
                    if now.as_ref().map(content).as_ref() != Some(&then) {
                        puts.push((key, then));
                    }
                }
                (None, None) => {}
            }
        }
        const CHUNK: usize = 1_000;
        for chunk in puts.chunks(CHUNK) {
            self.upsert_batch(chunk.to_vec())?;
        }
        for chunk in removes.chunks(CHUNK) {
            self.delete_batch(chunk.to_vec())?;
        }
        let written = puts
            .iter()
            .filter_map(|(k, _)| self.get(k).ok().map(|d| d.to_value()))
            .collect();
        Ok(Restored {
            written,
            removed: removes,
        })
    }

    pub fn set_row_policy(&self, expr: Option<&str>) -> DbResult<()> {
        let cf = self
            .ks
            .handle(&self.db)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;
        match expr {
            Some(s) => {
                self.db
                    .put_ks(&cf, ROW_POLICY_META_KEY.as_bytes(), s.as_bytes())
                    .map_err(|e| DbError::InternalError(format!("set_row_policy: {e}")))?;
            }
            None => {
                let _ = self.db.delete_ks(&cf, ROW_POLICY_META_KEY.as_bytes());
            }
        }
        Ok(())
    }

    pub fn get_row_policy(&self) -> Option<String> {
        let cf = self.ks.handle(&self.db)?;
        self.db
            .get_ks(&cf, ROW_POLICY_META_KEY.as_bytes())
            .ok()
            .flatten()
            .and_then(|b| String::from_utf8(b).ok())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_version_requires_exact_timestamp_remainder() {
        let prefix = Collection::version_prefix("a");
        let own = Collection::version_key("a", 42);
        let other = Collection::version_key("a:b", 42);
        let hexy_other = Collection::version_key("a:0123456789abcdef", 42);
        assert!(Collection::is_own_version(
            own.as_bytes(),
            prefix.as_bytes()
        ));
        assert!(!Collection::is_own_version(
            other.as_bytes(),
            prefix.as_bytes()
        ));
        assert!(!Collection::is_own_version(
            hexy_other.as_bytes(),
            prefix.as_bytes()
        ));
        // Every own version sorts below the per-key upper bound.
        assert!(own.as_bytes() < Collection::version_upper_bound("a").as_slice());
    }
}
