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
use super::RocksDb as DB;
use crate::error::{DbError, DbResult};

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
