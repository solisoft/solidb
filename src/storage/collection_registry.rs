//! Collection registry: which collections exist, held in `_meta`.
//!
//! Collection existence used to be read straight off RocksDB's column-family
//! map via `DB::cf_names()`. That has two costs on an instance with many
//! collections, and the second is the expensive one:
//!
//! 1. It clones every column-family name in the *whole instance* on every
//!    call — 963 `String` allocations to list one database's collections.
//! 2. It takes a read lock on the CF map, and `create_cf`/`drop_cf` hold the
//!    matching **write** lock for the entire duration of their OPTIONS
//!    rewrite. So listing collections could block for hundreds of
//!    milliseconds behind an unrelated collection being created in another
//!    database.
//!
//! A `coll:{db}:{name}` key per collection in the `_meta` column family
//! answers the same question with a prefix scan and no CF-map lock at all.
//!
//! The column-family map stays the underlying truth: [`backfill`] runs on
//! every startup and adopts any column family that has no entry, so a crash
//! between `create_cf` and the registry write — or a downgrade to a binary
//! that never wrote entries — heals itself rather than losing a collection.

use rust_rocksdb::WriteBatch;
use serde::{Deserialize, Serialize};

use super::engine::META_CF;
use super::keyspace::{self, Keyspace, KsNum, BARE_DB_ID, RESERVED_DB_ID, SHARED_CF};
use super::RocksDb as DB;
use crate::error::{DbError, DbResult};
use std::sync::Arc;

/// `_meta` key prefix for registry entries.
pub(crate) const ENTRY_PREFIX: &str = "coll:";

/// What the registry records about a collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionRecord {
    /// "document", "edge" or "blob".
    #[serde(default = "default_type")]
    pub type_: String,
    /// Milliseconds since the epoch, for diagnostics and for a future sweep
    /// of collections nothing has used.
    #[serde(default)]
    pub created_ms: u64,
    /// Shared-layout keyspace (`db_id << 32 | coll_id`). Absent for a legacy
    /// collection living in its own column family — which is what every
    /// record written by 1.x reads as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ks: Option<KsNum>,
    /// Set while the startup migration copies this legacy collection into
    /// the shared keyspace it names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrating_to: Option<KsNum>,
}

fn default_type() -> String {
    "document".to_string()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Whether the registry can be consulted at all.
///
/// False for a `Database` built over a bare RocksDB handle rather than by
/// `StorageEngine` — there is no `_meta` column family to hold entries. All
/// callers fall back to the column-family map in that case, so the registry
/// is an optimisation, never a way for a collection to disappear.
pub fn available(db: &DB) -> bool {
    db.cf_handle(META_CF).is_some()
}

/// The `_meta` key for one collection.
pub(crate) fn entry_key(cf_name: &str) -> String {
    format!("{}{}", ENTRY_PREFIX, cf_name)
}

/// The `coll:{db}:` prefix covering one database's collections.
pub(crate) fn database_prefix(db_name: &str) -> String {
    format!("{}{}:", ENTRY_PREFIX, db_name)
}

/// Record a collection. Written *after* its column family exists, so a crash
/// in between leaves an orphan that [`backfill`] adopts, rather than an entry
/// promising a column family that was never created.
pub fn record(db: &DB, cf_name: &str, type_: &str) -> DbResult<()> {
    let meta_cf = db
        .cf_handle(META_CF)
        .ok_or_else(|| DbError::InternalError("_meta column family missing".to_string()))?;

    let value = serde_json::to_vec(&CollectionRecord {
        type_: type_.to_string(),
        created_ms: now_ms(),
        ks: None,
        migrating_to: None,
    })
    .map_err(|e| DbError::InternalError(format!("Failed to encode collection record: {e}")))?;

    db.put_cf(&meta_cf, entry_key(cf_name).as_bytes(), value)
        .map_err(|e| DbError::InternalError(format!("Failed to record collection: {e}")))
}

/// Forget a collection.
pub fn forget(db: &DB, cf_name: &str) -> DbResult<()> {
    let meta_cf = db
        .cf_handle(META_CF)
        .ok_or_else(|| DbError::InternalError("_meta column family missing".to_string()))?;
    db.delete_cf(&meta_cf, entry_key(cf_name).as_bytes())
        .map_err(|e| DbError::InternalError(format!("Failed to forget collection: {e}")))
}

/// Add every `coll:{db}:` deletion for `db_name` to an existing batch, so a
/// database drop removes its collections' entries atomically with its own
/// metadata key.
pub fn forget_database_in_batch(db: &DB, batch: &mut WriteBatch, db_name: &str) {
    let Some(meta_cf) = db.cf_handle(META_CF) else {
        return;
    };
    let prefix = database_prefix(db_name);
    for key in scan_keys(db, &prefix) {
        batch.delete_cf(&meta_cf, key.as_bytes());
    }
}

/// Collection names registered under `db_name`.
pub fn list(db: &DB, db_name: &str) -> Vec<String> {
    let prefix = database_prefix(db_name);
    scan_keys(db, &prefix)
        .into_iter()
        .filter_map(|key| key.strip_prefix(&prefix).map(|s| s.to_string()))
        .collect()
}

/// Every registered collection, as `{db}:{collection}` column-family names.
pub fn list_all(db: &DB) -> Vec<String> {
    scan_keys(db, ENTRY_PREFIX)
        .into_iter()
        .filter_map(|key| key.strip_prefix(ENTRY_PREFIX).map(|s| s.to_string()))
        .collect()
}

/// Raw `_meta` keys under `prefix`.
fn scan_keys(db: &DB, prefix: &str) -> Vec<String> {
    let Some(meta_cf) = db.cf_handle(META_CF) else {
        return Vec::new();
    };
    db.prefix_iterator_cf(&meta_cf, prefix.as_bytes())
        .filter_map(|result| {
            let (key, _) = result.ok()?;
            let key = String::from_utf8(key.to_vec()).ok()?;
            key.starts_with(prefix).then_some(key)
        })
        .collect()
}

/// Adopt every collection column family that has no registry entry.
///
/// Idempotent, and the reason the registry can be trusted for listing: a
/// column family created by an older binary, or by a run that crashed between
/// `create_cf` and [`record`], is picked up here rather than staying
/// invisible. Returns how many entries were written.
pub fn backfill(db: &DB, is_pending: impl Fn(&str) -> bool) -> usize {
    let Some(meta_cf) = db.cf_handle(META_CF) else {
        return 0;
    };

    let known: std::collections::HashSet<String> = list_all(db).into_iter().collect();
    let mut batch = WriteBatch::default();
    let mut adopted = 0usize;

    for cf_name in db.cf_names() {
        if cf_name == "default" || cf_name == META_CF {
            continue;
        }
        // Collection CFs are named "<database>:<collection>".
        if !cf_name.contains(':') || known.contains(&cf_name) || is_pending(&cf_name) {
            continue;
        }

        // The type lives in the column family itself on a pre-registry
        // instance; read it back so the adopted entry is not a guess.
        let type_ = db
            .cf_handle(&cf_name)
            .and_then(|cf| db.get_cf(&cf, b"_stats:type").ok().flatten())
            .map(|bytes| String::from_utf8_lossy(&bytes).to_string())
            .unwrap_or_else(default_type);

        if let Ok(value) = serde_json::to_vec(&CollectionRecord {
            type_,
            created_ms: 0, // unknown: this column family predates the registry
            ks: None,
            migrating_to: None,
        }) {
            batch.put_cf(&meta_cf, entry_key(&cf_name).as_bytes(), value);
            adopted += 1;
        }
    }

    if adopted > 0 {
        if let Err(e) = db.write(&batch) {
            tracing::warn!("Collection registry backfill failed: {}", e);
            return 0;
        }
        tracing::info!(
            "Adopted {} column families into the collection registry",
            adopted
        );
    }
    adopted
}

// ==================== Shared-layout catalog ====================
//
// A shared-layout collection is a registry record carrying `ks`, plus the
// keys under that eight-byte prefix in `SHARED_CF`. Creating one is a single
// `_meta` + data write; dropping one is a record delete and one range delete.
// Neither touches the column-family map, so neither rewrites OPTIONS.

/// `_meta` key holding a database's id, allocated on its first shared
/// collection. Separate from `db:{name}` so that value stays `"1"`, which a
/// 1.x binary still understands.
const DB_ID_PREFIX: &str = "dbid:";
/// `_meta` counter for database ids.
const NEXT_DB_ID_KEY: &str = "ks:next_db_id";
/// `_meta` per-database collection-id counter prefix.
const NEXT_COLL_ID_PREFIX: &str = "ks:next_coll:";
/// `_meta` markers for dropped keyspace ranges awaiting compaction.
pub(crate) const DEAD_KS_PREFIX: &str = "dead_ks:";

/// Serialises catalog changes (id allocation, create, drop) across every
/// `Database` handle and the engine: both used to guard creation with their
/// own lock, so the two paths were not serialised against each other.
static CATALOG: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

pub(crate) fn catalog_lock() -> parking_lot::MutexGuard<'static, ()> {
    CATALOG.lock()
}

/// Whether new collections go into the shared keyspace. Needs `_meta` and the
/// shared column family (a `Database` over a bare RocksDB handle has
/// neither); `SOLIDB_LEGACY_COLLECTION_CFS=1` forces the 1.x layout.
pub fn shared_layout_enabled(db: &DB) -> bool {
    if std::env::var("SOLIDB_LEGACY_COLLECTION_CFS").is_ok_and(|v| v == "1" || v == "true") {
        return false;
    }
    available(db) && db.cf_handle(SHARED_CF).is_some()
}

/// Whether `full_name` is a shared-layout collection. A legacy column family
/// of the same name awaiting its drop (left by the migration) does not make
/// it deleted.
pub fn is_shared(db: &DB, full_name: &str) -> bool {
    get(db, full_name).is_some_and(|r| r.ks.is_some())
}

/// The registry record of `full_name`, if any.
pub fn get(db: &DB, full_name: &str) -> Option<CollectionRecord> {
    let meta_cf = db.cf_handle(META_CF)?;
    let bytes = db
        .get_cf(&meta_cf, entry_key(full_name).as_bytes())
        .ok()??;
    serde_json::from_slice(&bytes).ok()
}

/// Where the collection `full_name` (`db:coll`, or a bare engine-level name)
/// keeps its keys, or `None` when it does not exist. A shared record wins;
/// otherwise a live (not `is_pending`) column family of that name is a legacy
/// collection.
pub fn keyspace_of(
    db: &Arc<DB>,
    full_name: &str,
    is_pending: impl Fn(&str) -> bool,
) -> Option<Keyspace> {
    if let Some(ks) = get(db, full_name).and_then(|r| r.ks) {
        return Some(Keyspace::shared(db, ks));
    }
    if is_pending(full_name) {
        return None;
    }
    db.cf_handle(full_name)
        .map(|_| Keyspace::legacy(db, full_name))
}

fn read_u32(db: &DB, meta_cf: &Arc<rust_rocksdb::BoundColumnFamily<'_>>, key: &str) -> Option<u32> {
    let bytes = db.get_cf(meta_cf, key.as_bytes()).ok()??;
    std::str::from_utf8(&bytes).ok()?.parse().ok()
}

/// The database part of a collection name: `Some("db")` for `db:coll`,
/// `None` for a bare engine-level name.
fn database_of(full_name: &str) -> Option<&str> {
    full_name.split_once(':').map(|(db, _)| db)
}

/// Id of `db_name`, allocating (into `batch`) when it has none yet.
fn database_id(
    db: &DB,
    meta_cf: &Arc<rust_rocksdb::BoundColumnFamily<'_>>,
    db_name: Option<&str>,
    batch: &mut WriteBatch,
) -> DbResult<u32> {
    let Some(name) = db_name else {
        return Ok(BARE_DB_ID);
    };
    let key = format!("{}{}", DB_ID_PREFIX, name);
    if let Some(id) = read_u32(db, meta_cf, &key) {
        return Ok(id);
    }
    let id = read_u32(db, meta_cf, NEXT_DB_ID_KEY).unwrap_or(1).max(1);
    if id == RESERVED_DB_ID {
        return Err(DbError::InternalError("database ids exhausted".to_string()));
    }
    batch.put_cf(meta_cf, NEXT_DB_ID_KEY.as_bytes(), (id + 1).to_string());
    batch.put_cf(meta_cf, key.as_bytes(), id.to_string());
    Ok(id)
}

/// The id of an existing database, if it has one.
pub fn existing_database_id(db: &DB, db_name: &str) -> Option<u32> {
    let meta_cf = db.cf_handle(META_CF)?;
    read_u32(db, &meta_cf, &format!("{}{}", DB_ID_PREFIX, db_name))
}

/// Allocate a keyspace for `full_name` and add its counters to `batch`.
/// Caller holds [`catalog_lock`].
pub(crate) fn allocate_keyspace(
    db: &DB,
    full_name: &str,
    batch: &mut WriteBatch,
) -> DbResult<KsNum> {
    let meta_cf = db
        .cf_handle(META_CF)
        .ok_or_else(|| DbError::InternalError("_meta column family missing".to_string()))?;
    let db_id = database_id(db, &meta_cf, database_of(full_name), batch)?;
    let counter = format!("{}{}", NEXT_COLL_ID_PREFIX, db_id);
    let coll_id = read_u32(db, &meta_cf, &counter).unwrap_or(0);
    if coll_id == u32::MAX {
        return Err(DbError::InternalError(format!(
            "collection ids exhausted for database id {}",
            db_id
        )));
    }
    batch.put_cf(&meta_cf, counter.as_bytes(), (coll_id + 1).to_string());
    Ok(keyspace::ks_num(db_id, coll_id))
}

/// Create `full_name` in a fresh shared keyspace: record, id counters and
/// its `_stats:type`, in one write. Caller holds [`catalog_lock`] and has
/// checked that the name is free.
pub(crate) fn create_shared(db: &Arc<DB>, full_name: &str, type_: &str) -> DbResult<Keyspace> {
    let meta_cf = db
        .cf_handle(META_CF)
        .ok_or_else(|| DbError::InternalError("_meta column family missing".to_string()))?;
    let mut batch = WriteBatch::default();
    let ks = allocate_keyspace(db, full_name, &mut batch)?;
    let record = CollectionRecord {
        type_: type_.to_string(),
        created_ms: now_ms(),
        ks: Some(ks),
        migrating_to: None,
    };
    let value = serde_json::to_vec(&record)
        .map_err(|e| DbError::InternalError(format!("Failed to encode collection record: {e}")))?;
    batch.put_cf(&meta_cf, entry_key(full_name).as_bytes(), value);

    let keyspace = Keyspace::shared(db, ks);
    let data = keyspace.live(db, full_name)?;
    use super::keyspace::KsBatchExt;
    batch.put_ks(&data, "_stats:type".as_bytes(), type_.as_bytes());

    db.write(&batch)
        .map_err(|e| DbError::InternalError(format!("Failed to create collection: {e}")))?;
    super::cf_ops::record_keyspace_create();
    Ok(keyspace)
}

/// Queue `[lo, hi)` of the shared column family for background compaction,
/// in `batch`.
fn mark_range_dead(
    meta_cf: &Arc<rust_rocksdb::BoundColumnFamily<'_>>,
    batch: &mut WriteBatch,
    lo: &[u8],
    hi: &[u8],
) {
    let key = format!("{}{}", DEAD_KS_PREFIX, hex::encode(lo));
    batch.put_cf(meta_cf, key.as_bytes(), hi);
}

/// Drop the shared-layout collection `full_name` living in `ks`: record,
/// data and a compaction marker in one write. Every handle of the keyspace
/// reports `CollectionNotFound` from here on.
pub(crate) fn drop_shared(db: &Arc<DB>, full_name: &str, ks: KsNum) -> DbResult<()> {
    let meta_cf = db
        .cf_handle(META_CF)
        .ok_or_else(|| DbError::InternalError("_meta column family missing".to_string()))?;
    let shared = db
        .cf_handle(SHARED_CF)
        .ok_or_else(|| DbError::InternalError("shared column family missing".to_string()))?;
    let prefix = keyspace::KsPrefix::shared(ks);
    let lo = prefix.as_bytes().to_vec();
    let hi = prefix.upper().expect("shared prefix has an upper bound");

    let mut batch = WriteBatch::default();
    batch.delete_cf(&meta_cf, entry_key(full_name).as_bytes());
    batch.delete_range_cf(&shared, &lo, &hi);
    mark_range_dead(&meta_cf, &mut batch, &lo, &hi);
    db.write(&batch)
        .map_err(|e| DbError::InternalError(format!("Failed to delete collection: {e}")))?;

    keyspace::mark_dead(db, &keyspace::KsId::Shared(ks));
    super::cf_ops::record_keyspace_drop();
    super::keyspace_gc::wake();
    Ok(())
}

/// Erase every shared keyspace of database `db_name` (one range delete) and
/// forget its id. Returns the id, so the caller can mark live handles dead.
pub(crate) fn drop_database_keyspaces(db: &Arc<DB>, db_name: &str) -> DbResult<Option<u32>> {
    let (Some(meta_cf), Some(shared)) = (db.cf_handle(META_CF), db.cf_handle(SHARED_CF)) else {
        return Ok(None);
    };
    let key = format!("{}{}", DB_ID_PREFIX, db_name);
    let Some(db_id) = read_u32(db, &meta_cf, &key) else {
        return Ok(None);
    };
    let lo = keyspace::ks_num(db_id, 0).to_be_bytes().to_vec();
    let hi = keyspace::ks_num(db_id + 1, 0).to_be_bytes().to_vec();

    let mut batch = WriteBatch::default();
    // Its collections' records: the legacy drop path only removes those of
    // the column families it schedules, which shared collections have none of.
    forget_database_in_batch(db, &mut batch, db_name);
    batch.delete_cf(&meta_cf, key.as_bytes());
    batch.delete_cf(
        &meta_cf,
        format!("{}{}", NEXT_COLL_ID_PREFIX, db_id).as_bytes(),
    );
    batch.delete_range_cf(&shared, &lo, &hi);
    mark_range_dead(&meta_cf, &mut batch, &lo, &hi);
    db.write(&batch)
        .map_err(|e| DbError::InternalError(format!("Failed to delete database data: {e}")))?;
    keyspace::mark_database_dead(db, db_id);
    super::keyspace_gc::wake();
    Ok(Some(db_id))
}

/// Pending `dead_ks:` markers: `(marker key, lo, hi)`.
pub(crate) fn dead_ranges(db: &DB) -> Vec<(Vec<u8>, Vec<u8>, Vec<u8>)> {
    let Some(meta_cf) = db.cf_handle(META_CF) else {
        return Vec::new();
    };
    let prefix = DEAD_KS_PREFIX.as_bytes();
    let mut out = Vec::new();
    for item in db.prefix_iterator_cf(&meta_cf, prefix) {
        let Ok((key, hi)) = item else { break };
        if !key.starts_with(prefix) {
            break;
        }
        let Ok(lo) = hex::decode(&key[prefix.len()..]) else {
            continue;
        };
        out.push((key.to_vec(), lo, hi.to_vec()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_rocksdb::Options;
    use tempfile::TempDir;

    fn open() -> (DB, TempDir) {
        let dir = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db = DB::open_cf(&opts, dir.path(), [META_CF]).unwrap();
        (db, dir)
    }

    #[test]
    fn record_then_list_round_trips() {
        let (db, _dir) = open();
        record(&db, "shop:orders", "document").unwrap();
        record(&db, "shop:edges", "edge").unwrap();
        record(&db, "other:orders", "document").unwrap();

        let mut listed = list(&db, "shop");
        listed.sort();
        assert_eq!(listed, vec!["edges".to_string(), "orders".to_string()]);

        forget(&db, "shop:edges").unwrap();
        assert_eq!(list(&db, "shop"), vec!["orders".to_string()]);
    }

    /// A database's prefix must not catch a differently-named one that shares
    /// its leading characters.
    #[test]
    fn list_does_not_leak_across_similar_database_names() {
        let (db, _dir) = open();
        record(&db, "app:things", "document").unwrap();
        record(&db, "app_test:things", "document").unwrap();

        assert_eq!(list(&db, "app"), vec!["things".to_string()]);
        assert_eq!(list(&db, "app_test"), vec!["things".to_string()]);
    }

    /// The backfill is what lets listing trust the registry: a column family
    /// with no entry — from a pre-registry binary, or a crash between
    /// `create_cf` and `record` — must be adopted, with its real type.
    #[test]
    fn backfill_adopts_unregistered_column_families() {
        let (db, _dir) = open();
        db.create_cf("shop:orders", &Options::default()).unwrap();
        db.create_cf("shop:edges", &Options::default()).unwrap();

        // The type lives in the column family on a pre-registry instance.
        let cf = db.cf_handle("shop:edges").unwrap();
        db.put_cf(&cf, b"_stats:type", b"edge").unwrap();

        assert!(list(&db, "shop").is_empty());
        assert_eq!(backfill(&db, |_| false), 2);

        let mut listed = list(&db, "shop");
        listed.sort();
        assert_eq!(listed, vec!["edges".to_string(), "orders".to_string()]);

        // Idempotent: a second pass adopts nothing.
        assert_eq!(backfill(&db, |_| false), 0);
    }

    /// A column family already scheduled for drop must not be adopted back
    /// into existence.
    #[test]
    fn backfill_skips_doomed_column_families() {
        let (db, _dir) = open();
        db.create_cf("shop:doomed", &Options::default()).unwrap();

        assert_eq!(backfill(&db, |cf| cf == "shop:doomed"), 0);
        assert!(list(&db, "shop").is_empty());
    }
}
