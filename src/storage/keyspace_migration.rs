//! Startup migration of 1.x collections into the shared keyspace.
//!
//! A 1.x instance keeps every collection in its own column family. The first
//! start of 2.x moves each one into [`SHARED_CF`] under a fresh keyspace
//! prefix, so that creating and dropping collections stops rewriting the
//! OPTIONS file. It runs inside `StorageEngine::initialize`, before anything
//! is served, one collection at a time:
//!
//! 1. record `migrating_to = ks` in the collection's registry entry (synced);
//! 2. copy every key into `ks`, counting keys and hashing key+value;
//! 3. re-read the copy and compare count and hash;
//! 4. flip the entry to `ks` (synced) and schedule the old column family for
//!    a background drop;
//! 5. free the old column family's files at once, so the extra disk used at
//!    any moment is about one collection, not the whole data set.
//!
//! Every step is resumable. A crash during 2–3 leaves `migrating_to`: the
//! target range is wiped and the copy redone. A crash after 4 leaves a shared
//! entry whose old column family still exists: it is scheduled again. A
//! collection that fails (verification mismatch, I/O error) stays legacy —
//! still served from its own column family — and is retried at the next
//! start. When nothing is left to move, `_meta storage_format` becomes `2`
//! and later starts skip all of this.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_rocksdb::{IteratorMode, WriteBatch, WriteOptions};

use super::collection_registry::{self, CollectionRecord};
use super::engine::META_CF;
use super::keyspace::{KsNum, KsPrefix, SHARED_CF};
use super::pending_drops::PendingCfDrops;
use super::RocksDb as DB;
use crate::error::{DbError, DbResult};

/// `_meta` key: storage format of this instance. Absent on 1.x.
pub const STORAGE_FORMAT_KEY: &str = "storage_format";
pub const STORAGE_FORMAT_V2: &str = "2";
/// `_meta` key: JSON report of the last migration run.
const REPORT_KEY: &str = "migration:v2:report";

/// Flush a copy batch at this many bytes or keys.
const BATCH_BYTES: usize = 4 * 1024 * 1024;
const BATCH_KEYS: usize = 10_000;

#[derive(Debug, Default, serde::Serialize)]
pub struct MigrationReport {
    pub migrated: usize,
    pub skipped: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub keys_copied: u64,
    pub bytes_copied: u64,
    pub seconds: f64,
}

/// FNV-1a over (key, value) pairs, order-sensitive: the copy and the re-read
/// walk the same sorted order, so equal digests mean equal contents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Digest(u64, u64);

impl Digest {
    fn new() -> Self {
        Digest(0xcbf2_9ce4_8422_2325, 0)
    }
    fn add(&mut self, key: &[u8], value: &[u8]) {
        let mut h = self.0;
        for part in [key, &[0xff][..], value, &[0xfe][..]] {
            for b in part {
                h ^= *b as u64;
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
        }
        self.0 = h;
        self.1 += 1;
    }
}

fn is_v2(db: &DB) -> bool {
    let Some(meta) = db.cf_handle(META_CF) else {
        return false;
    };
    db.get_cf(&meta, STORAGE_FORMAT_KEY.as_bytes())
        .ok()
        .flatten()
        .is_some_and(|v| v == STORAGE_FORMAT_V2.as_bytes())
}

fn synced() -> WriteOptions {
    let mut w = WriteOptions::default();
    w.set_sync(true);
    w
}

fn put_record(db: &DB, name: &str, record: &CollectionRecord) -> DbResult<()> {
    let meta = db
        .cf_handle(META_CF)
        .ok_or_else(|| DbError::InternalError("_meta missing".into()))?;
    let value = serde_json::to_vec(record).map_err(|e| DbError::InternalError(e.to_string()))?;
    db.put_cf_opt(
        &meta,
        collection_registry::entry_key(name).as_bytes(),
        value,
        &synced(),
    )
    .map_err(|e| DbError::InternalError(format!("registry write failed: {}", e)))
}

fn sst_size(db: &DB, cf_name: &str) -> u64 {
    db.cf_handle(cf_name)
        .and_then(|cf| {
            db.property_int_value_cf(&cf, "rocksdb.total-sst-files-size")
                .ok()
                .flatten()
        })
        .unwrap_or(0)
}

/// Free bytes on the filesystem holding `path`.
#[cfg(unix)]
fn free_space(path: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` a valid out-pointer.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

/// Not measured off Unix: the migration then skips the disk-space check.
#[cfg(not(unix))]
fn free_space(_path: &std::path::Path) -> Option<u64> {
    None
}

/// Delete everything under keyspace `ks` in the shared column family.
fn wipe_target(db: &DB, ks: KsNum) -> DbResult<()> {
    let shared = db
        .cf_handle(SHARED_CF)
        .ok_or_else(|| DbError::InternalError("shared column family missing".into()))?;
    let p = KsPrefix::shared(ks);
    let lo = p.as_bytes().to_vec();
    let hi = p.upper().expect("shared prefix");
    let mut batch = WriteBatch::default();
    batch.delete_range_cf(&shared, &lo, &hi);
    db.write(&batch)
        .map_err(|e| DbError::InternalError(e.to_string()))
}

/// Copy the legacy column family `cf_name` into keyspace `ks`.
fn copy(db: &DB, cf_name: &str, ks: KsNum, report: &mut MigrationReport) -> DbResult<Digest> {
    let legacy = db
        .cf_handle(cf_name)
        .ok_or_else(|| DbError::CollectionNotFound(cf_name.to_string()))?;
    let shared = db
        .cf_handle(SHARED_CF)
        .ok_or_else(|| DbError::InternalError("shared column family missing".into()))?;
    let prefix = KsPrefix::shared(ks);
    let mut digest = Digest::new();
    let mut batch = WriteBatch::default();
    let (mut batch_bytes, mut batch_keys) = (0usize, 0usize);
    for item in db.iterator_cf(&legacy, IteratorMode::Start) {
        let (key, value) = item.map_err(|e| DbError::InternalError(e.to_string()))?;
        digest.add(&key, &value);
        batch_bytes += key.len() + value.len() + 8;
        batch_keys += 1;
        report.bytes_copied += (key.len() + value.len()) as u64;
        batch.put_cf(&shared, prefix.key(&key), &value);
        if batch_bytes >= BATCH_BYTES || batch_keys >= BATCH_KEYS {
            db.write(&batch)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
            batch = WriteBatch::default();
            batch_bytes = 0;
            batch_keys = 0;
        }
    }
    if batch_keys > 0 {
        db.write(&batch)
            .map_err(|e| DbError::InternalError(e.to_string()))?;
    }
    report.keys_copied += digest.1;
    Ok(digest)
}

/// Re-read keyspace `ks` and hash it the same way as [`copy`].
fn digest_of(db: &DB, ks: KsNum) -> DbResult<Digest> {
    let shared = db
        .cf_handle(SHARED_CF)
        .ok_or_else(|| DbError::InternalError("shared column family missing".into()))?;
    let p = KsPrefix::shared(ks);
    let mut opts = rust_rocksdb::ReadOptions::default();
    opts.set_iterate_lower_bound(p.as_bytes().to_vec());
    opts.set_iterate_upper_bound(p.upper().expect("shared prefix"));
    let mut digest = Digest::new();
    for item in db.iterator_cf_opt(&shared, opts, IteratorMode::Start) {
        let (key, value) = item.map_err(|e| DbError::InternalError(e.to_string()))?;
        digest.add(&key[8..], &value);
    }
    Ok(digest)
}

/// Free the old column family's files and range-delete what is left, so its
/// disk space comes back now rather than when the background drop runs.
fn free_legacy(db: &DB, cf_name: &str) {
    let Some(cf) = db.cf_handle(cf_name) else {
        return;
    };
    let first = db.iterator_cf(&cf, IteratorMode::Start).next();
    let last = db.iterator_cf(&cf, IteratorMode::End).next();
    let (Some(Ok((lo, _))), Some(Ok((hi, _)))) = (first, last) else {
        return;
    };
    let lo = lo.to_vec();
    let mut hi = hi.to_vec();
    hi.push(0);
    if let Err(e) = db.delete_file_in_range_cf(&cf, &lo, &hi) {
        tracing::debug!("migration: delete_file_in_range on {}: {}", cf_name, e);
    }
    let mut batch = WriteBatch::default();
    batch.delete_range_cf(&cf, &lo, &hi);
    let _ = db.write(&batch);
}

fn type_of_legacy(db: &DB, cf_name: &str) -> String {
    db.cf_handle(cf_name)
        .and_then(|cf| db.get_cf(&cf, b"_stats:type").ok().flatten())
        .map(|b| String::from_utf8_lossy(&b).to_string())
        .unwrap_or_else(|| "document".to_string())
}

/// Move one legacy collection. `Ok(false)` when it is skipped on purpose.
fn migrate_one(
    db: &Arc<DB>,
    pending: &Arc<PendingCfDrops>,
    cf_name: &str,
    report: &mut MigrationReport,
) -> DbResult<bool> {
    let existing = collection_registry::get(db, cf_name);

    // Flipped already, old column family not yet scheduled (crash after 4).
    if let Some(record) = existing.as_ref().filter(|r| r.ks.is_some()) {
        let _ = record;
        free_legacy(db, cf_name);
        pending.schedule_one(db, cf_name)?;
        return Ok(true);
    }

    let type_ = existing
        .as_ref()
        .map(|r| r.type_.clone())
        .unwrap_or_else(|| type_of_legacy(db, cf_name));

    // Allocate, or resume the half-done copy with the same id.
    let ks = match existing.as_ref().and_then(|r| r.migrating_to) {
        Some(ks) => {
            wipe_target(db, ks)?;
            ks
        }
        None => {
            let _catalog = collection_registry::catalog_lock();
            let mut batch = WriteBatch::default();
            let ks = collection_registry::allocate_keyspace(db, cf_name, &mut batch)?;
            db.write_opt(&batch, &synced())
                .map_err(|e| DbError::InternalError(e.to_string()))?;
            ks
        }
    };
    let created_ms = existing.as_ref().map(|r| r.created_ms).unwrap_or(0);
    put_record(
        db,
        cf_name,
        &CollectionRecord {
            type_: type_.clone(),
            created_ms,
            ks: None,
            migrating_to: Some(ks),
        },
    )?;

    let copied = copy(db, cf_name, ks, report)?;
    let reread = digest_of(db, ks)?;
    if copied != reread {
        wipe_target(db, ks)?;
        return Err(DbError::InternalError(format!(
            "verification failed: copied {} keys, read back {}",
            copied.1, reread.1
        )));
    }

    // The flip. Synced: once it is durable, every earlier copy write is too
    // (the WAL is sequential).
    put_record(
        db,
        cf_name,
        &CollectionRecord {
            type_,
            created_ms,
            ks: Some(ks),
            migrating_to: None,
        },
    )?;
    free_legacy(db, cf_name);
    pending.schedule_one(db, cf_name)?;
    Ok(true)
}

/// Run the migration if this instance still has 1.x collections. Returns
/// `None` when there was nothing to do.
pub fn run(
    db: &Arc<DB>,
    pending: &Arc<PendingCfDrops>,
    data_dir: &std::path::Path,
) -> DbResult<Option<MigrationReport>> {
    if !collection_registry::shared_layout_enabled(db) || is_v2(db) {
        return Ok(None);
    }
    let meta = db
        .cf_handle(META_CF)
        .ok_or_else(|| DbError::InternalError("_meta missing".into()))?;

    // Make sure every legacy column family has a registry entry to flip.
    collection_registry::backfill(db, |cf| pending.contains(cf));

    let databases: std::collections::HashSet<String> = {
        let mut out = std::collections::HashSet::new();
        for item in db.prefix_iterator_cf(&meta, b"db:") {
            let Ok((k, _)) = item else { break };
            if !k.starts_with(b"db:") {
                break;
            }
            out.insert(String::from_utf8_lossy(&k[3..]).to_string());
        }
        out
    };

    let mut report = MigrationReport::default();
    let mut work: Vec<(u64, String)> = Vec::new();
    for cf in db.cf_names() {
        if cf == "default" || cf == META_CF || cf == SHARED_CF || pending.contains(&cf) {
            continue;
        }
        // A `db:coll` column family whose database is gone is the remains of
        // an interrupted 1.x drop: neither resurrect nor delete it here.
        if let Some((db_name, _)) = cf.split_once(':') {
            if !databases.contains(db_name) {
                report.skipped.push(cf);
                continue;
            }
        }
        work.push((sst_size(db, &cf), cf));
    }

    if work.is_empty() {
        db.put_cf_opt(&meta, STORAGE_FORMAT_KEY, STORAGE_FORMAT_V2, &synced())
            .map_err(|e| DbError::InternalError(e.to_string()))?;
        if report.skipped.is_empty() {
            return Ok(None);
        }
        return Ok(Some(report));
    }

    // Smallest first: most collections are done quickly and progress shows.
    work.sort();
    let largest = work.last().map(|(s, _)| *s).unwrap_or(0);
    if std::env::var("SOLIDB_MIGRATION_SKIP_SPACE_CHECK").as_deref() != Ok("1") {
        if let Some(free) = free_space(data_dir) {
            let need = largest + largest / 5 + (1 << 30);
            if free < need {
                return Err(DbError::InternalError(format!(
                    "storage migration needs about {} MB free for its largest collection \
                     ({} MB); {} MB available. Free space, or set \
                     SOLIDB_MIGRATION_SKIP_SPACE_CHECK=1",
                    need >> 20,
                    largest >> 20,
                    free >> 20
                )));
            }
        }
    }

    let total = work.len();
    let total_bytes: u64 = work.iter().map(|(s, _)| *s).sum();
    tracing::info!(
        "Storage migration to the shared keyspace: {} collections, ~{} MB",
        total,
        total_bytes >> 20
    );
    let started = Instant::now();
    let mut last_log = Instant::now();
    for (i, (_, cf)) in work.iter().enumerate() {
        match migrate_one(db, pending, cf, &mut report) {
            Ok(true) => report.migrated += 1,
            Ok(false) => report.skipped.push(cf.clone()),
            Err(e) => {
                tracing::warn!("Storage migration: '{}' stays legacy for now: {}", cf, e);
                report.failed.push((cf.clone(), e.to_string()));
            }
        }
        if last_log.elapsed() >= Duration::from_secs(5) || i + 1 == total {
            let secs = started.elapsed().as_secs_f64();
            let rate = report.bytes_copied as f64 / secs.max(0.001);
            tracing::info!(
                "Storage migration: {}/{} collections, {} MB copied ({:.1} MB/s)",
                i + 1,
                total,
                report.bytes_copied >> 20,
                rate / 1_048_576.0
            );
            last_log = Instant::now();
        }
    }
    report.seconds = started.elapsed().as_secs_f64();

    // Only a run that moved everything movable marks the instance as done;
    // failures are retried at the next start.
    let mut batch = WriteBatch::default();
    if report.failed.is_empty() {
        batch.put_cf(&meta, STORAGE_FORMAT_KEY, STORAGE_FORMAT_V2);
    }
    if let Ok(json) = serde_json::to_vec(&report) {
        batch.put_cf(&meta, REPORT_KEY, json);
    }
    db.write_opt(&batch, &synced())
        .map_err(|e| DbError::InternalError(e.to_string()))?;
    tracing::info!(
        "Storage migration done: {} migrated, {} skipped, {} failed, {:.1}s",
        report.migrated,
        report.skipped.len(),
        report.failed.len(),
        report.seconds
    );
    Ok(Some(report))
}
