//! Moving a data directory from RocksDB to solistore (integration step S3).
//!
//! The copy is built next to the data directory and verified before
//! anything is swapped. The RocksDB directory is kept, renamed, as the
//! rollback. Opening RocksDB takes its lock, so a running server makes the
//! migration fail rather than copy a moving target.

use std::collections::hash_map::DefaultHasher;
use std::hash::Hasher;
use std::path::{Path, PathBuf};

use super::select::{record, EngineKind};
use super::{Direction, IterMode, Kv, KvBatch, KvCf, KvError};
use crate::storage::pending_drops::MARKER_PREFIX;

/// Column families a 2.x data directory holds. Anything else is a 1.x
/// per-collection family that the server's own migration has not moved
/// yet: solistore does not take the 1.x layout.
const FAMILIES: [&str; 3] = ["default", "_meta", "__keyspaces__"];

/// RocksDB's own files in a data directory; everything else is SoliDB's
/// (the transaction WAL, the sync log) and moves with the data.
fn is_rocksdb_file(name: &str) -> bool {
    name.ends_with(".sst")
        || name.ends_with(".log")
        || name.starts_with("MANIFEST-")
        || name.starts_with("OPTIONS-")
        || name.starts_with("LOG")
        || matches!(name, "CURRENT" | "IDENTITY" | "LOCK" | "ENGINE")
}

/// Entries and a hash of every key and value of one column family, in key
/// order: what the copy must reproduce.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Digest {
    pub entries: u64,
    pub bytes: u64,
    pub hash: u64,
}

impl Digest {
    fn add(&mut self, hasher: &mut DefaultHasher, key: &[u8], value: &[u8]) {
        hasher.write_usize(key.len());
        hasher.write(key);
        hasher.write_usize(value.len());
        hasher.write(value);
        self.entries += 1;
        self.bytes += (key.len() + value.len()) as u64;
    }
}

fn digest(kv: &Kv, name: &str) -> Result<Digest, KvError> {
    let Some(cf) = kv.cf_handle(name) else {
        return Ok(Digest::default());
    };
    let mut d = Digest::default();
    let mut hasher = DefaultHasher::new();
    for item in kv.iterator_cf(&cf, IterMode::Start) {
        let (k, v) = item?;
        d.add(&mut hasher, &k, &v);
    }
    d.hash = hasher.finish();
    Ok(d)
}

/// Entries between two progress lines of one copy.
const PROGRESS_EVERY: u64 = 1 << 20;

/// A progress line every [`PROGRESS_EVERY`] entries of one copy, so a
/// copy of tens of GB does not look stalled.
struct Ticker<'a> {
    name: &'a str,
    started: std::time::Instant,
    next: u64,
    progress: &'a (dyn Fn(&str) + Sync),
}

impl<'a> Ticker<'a> {
    fn new(name: &'a str, progress: &'a (dyn Fn(&str) + Sync)) -> Self {
        Ticker {
            name,
            started: std::time::Instant::now(),
            next: PROGRESS_EVERY,
            progress,
        }
    }

    fn tick(&mut self, d: &Digest) {
        if d.entries < self.next {
            return;
        }
        self.next += PROGRESS_EVERY;
        let secs = self.started.elapsed().as_secs_f64().max(1e-3);
        (self.progress)(&format!(
            "copying {}: {:.1}M entries, {:.2} GB, {:.0} MB/s",
            self.name,
            d.entries as f64 / 1e6,
            d.bytes as f64 / 1e9,
            d.bytes as f64 / 1e6 / secs
        ));
    }
}

/// Copy one column family in key order, leaving out the keys `skip` names.
fn copy(
    from: &Kv,
    to: &Kv,
    name: &str,
    skip: &dyn Fn(&[u8]) -> bool,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Digest, KvError> {
    let Some(src) = from.cf_handle(name) else {
        return Ok(Digest::default());
    };
    let dst = to
        .cf_handle(name)
        .ok_or_else(|| KvError::new(format!("target has no column family {name}")))?;
    let mut d = Digest::default();
    let mut hasher = DefaultHasher::new();
    let mut ticker = Ticker::new(name, progress);
    // On solistore, the column family is built as bottom-level tables in
    // parallel (`Db::ingest`): each byte written once, where batches went
    // through the WAL, a flush and every compaction down the tree. The dev
    // server's 39 GB took 8 min 45 s that way.
    if let (Some(db), KvCf::Solistore(cf)) = (to.as_solistore(), &dst) {
        let entries = from
            .iterator_cf(&src, IterMode::Start)
            .filter_map(|item| match item {
                Ok((k, _)) if skip(&k) => None,
                Ok((k, v)) => {
                    d.add(&mut hasher, &k, &v);
                    ticker.tick(&d);
                    Some(Ok((k, v)))
                }
                Err(e) => Some(Err(e)),
            });
        db.ingest(cf, entries, ingest_threads())
            .map_err(|e| KvError::new(format!("loading {name}: {e}")))?;
        d.hash = hasher.finish();
        return Ok(d);
    }
    let mut batch = KvBatch::default();
    for item in from.iterator_cf(&src, IterMode::Start) {
        let (k, v) = item?;
        if skip(&k) {
            continue;
        }
        d.add(&mut hasher, &k, &v);
        ticker.tick(&d);
        batch.put_cf(&dst, &k, &v);
        if batch.size_in_bytes() >= 4 << 20 {
            to.write(std::mem::take(&mut batch))?;
        }
    }
    to.write(batch)?;
    d.hash = hasher.finish();
    Ok(d)
}

#[derive(Debug, Default)]
pub struct Report {
    /// Per column family (the sync log's as `sync_log`): what was copied.
    pub families: Vec<(String, Digest)>,
    /// Where the RocksDB directory now is.
    pub backup: PathBuf,
    /// 1.x column families left behind, as the server's own 2.x migration
    /// leaves them: those of dropped collections whose deletion was still
    /// pending (not copied, nor their markers), and the remains of an
    /// interrupted drop whose database is gone. They stay in the backup.
    pub skipped: Vec<String>,
}

/// Keys of `_meta` under `prefix`, the prefix stripped.
fn meta_keys(kv: &Kv, prefix: &[u8]) -> Result<Vec<String>, KvError> {
    let Some(meta) = kv.cf_handle("_meta") else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for item in kv.iterator_cf(&meta, IterMode::From(prefix, Direction::Forward)) {
        let (k, _) = item?;
        let Some(rest) = k.strip_prefix(prefix) else {
            break;
        };
        out.push(String::from_utf8_lossy(rest).into_owned());
    }
    Ok(out)
}

/// The 1.x column families the server's 2.x migration passes over
/// (`storage::keyspace_migration::run`): a pending drop, or a `db:coll`
/// whose database no longer exists.
fn left_behind(kv: &Kv, cfs: &[String]) -> Result<Vec<String>, KvError> {
    let pending = meta_keys(kv, MARKER_PREFIX.as_bytes())?;
    let databases = meta_keys(kv, b"db:")?;
    Ok(cfs
        .iter()
        .filter(|cf| !FAMILIES.contains(&cf.as_str()))
        .filter(|cf| {
            pending.contains(cf)
                || cf
                    .split_once(':')
                    .is_some_and(|(db, _)| !databases.iter().any(|d| d == db))
        })
        .cloned()
        .collect())
}

/// Threads building tables: the cores, up to eight (each holds ~200 MB of
/// entries while it works).
fn ingest_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8)
}

fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    dir.with_file_name(name)
}

fn io(what: &str, path: &Path, e: std::io::Error) -> KvError {
    KvError::new(format!("{what} {}: {e}", path.display()))
}

/// Read back every copied column family of `copy_of`, one thread each, and
/// compare with what was copied. `what` names the database in an error
/// (empty: the column family's own name).
fn verify(copy_of: &Kv, families: &[(String, Digest)], what: &str) -> Result<(), KvError> {
    std::thread::scope(|s| {
        let reads: Vec<_> = families
            .iter()
            .map(|(name, copied)| (name, copied, s.spawn(|| digest(copy_of, name))))
            .collect();
        for (name, copied, read) in reads {
            let got = read
                .join()
                .unwrap_or_else(|p| std::panic::resume_unwind(p))?;
            if got != *copied {
                let name = if what.is_empty() { name.as_str() } else { what };
                return Err(KvError::new(format!(
                    "verification failed for {name}: copied {copied:?}, read back {got:?}"
                )));
            }
        }
        Ok(())
    })
}

/// The replication log: its own database in `sync_log/`, copied and
/// verified on its own.
fn copy_sync_log(
    log_dir: &Path,
    target_log: &Path,
    progress: &(dyn Fn(&str) + Sync),
) -> Result<Digest, KvError> {
    progress("copying sync_log");
    let source = super::rocks::open_sync_log(log_dir)?;
    let copy_of = super::solistore::open_sync_log(target_log)?;
    let copied = copy(&source, &copy_of, "default", &|_| false, progress)?;
    copy_of.flush()?;
    verify(&copy_of, &[("default".to_string(), copied)], "sync_log")?;
    record(target_log, EngineKind::Solistore)?;
    progress("sync_log copied and verified");
    Ok(copied)
}

/// Migrate the RocksDB data directory `dir` to solistore, in place. On
/// success, `dir` is a solistore directory and the RocksDB one is kept at
/// `Report::backup`. On failure, `dir` is untouched.
pub fn to_solistore(dir: &Path, progress: &(dyn Fn(&str) + Sync)) -> Result<Report, KvError> {
    match super::select::recorded(dir)? {
        Some(EngineKind::Rocksdb) => {}
        Some(EngineKind::Solistore) => {
            return Err(KvError::new(format!(
                "{} is already on solistore",
                dir.display()
            )))
        }
        None => return Err(KvError::new(format!("{} holds no database", dir.display()))),
    }
    let target = sibling(dir, ".solistore-tmp");
    if target.exists() {
        // What an interrupted run left: never anything to keep.
        std::fs::remove_dir_all(&target).map_err(|e| io("remove", &target, e))?;
    }
    std::fs::create_dir_all(&target).map_err(|e| io("create", &target, e))?;

    let mut report = Report::default();
    {
        let source = super::rocks::open(dir, &[])?;
        let cfs = source.cf_names();
        report.skipped = left_behind(&source, &cfs)?;
        let unknown: Vec<String> = cfs
            .into_iter()
            .filter(|n| !FAMILIES.contains(&n.as_str()) && !report.skipped.contains(n))
            .collect();
        if !unknown.is_empty() {
            return Err(KvError::new(format!(
                "{} still has 1.x column families ({} of them, e.g. {}); start a 2.x \
                 server on it once to finish its layout migration, then retry",
                dir.display(),
                unknown.len(),
                unknown[0]
            )));
        }
        // The sync log is a database of its own: it is copied alongside.
        let log_dir = dir.join("sync_log");
        let target_log = target.join("sync_log");
        let (main, log) = std::thread::scope(|s| {
            let log = log_dir
                .exists()
                .then(|| s.spawn(|| copy_sync_log(&log_dir, &target_log, progress)));
            let main = (|| {
                let copy_of = super::solistore::open(&target, &FAMILIES[1..])?;
                // The markers go with the column families they name.
                let markers = |k: &[u8]| k.starts_with(MARKER_PREFIX.as_bytes());
                let mut families = Vec::new();
                for name in FAMILIES {
                    progress(&format!("copying {name}"));
                    let copied = copy(&source, &copy_of, name, &markers, progress)?;
                    families.push((name.to_string(), copied));
                }
                copy_of.flush()?;
                progress("verifying");
                verify(&copy_of, &families, "")?;
                Ok::<_, KvError>(families)
            })();
            let log = log.map(|h| h.join().unwrap_or_else(|p| std::panic::resume_unwind(p)));
            (main, log)
        });
        report.families = main?;
        if let Some(log) = log {
            report.families.push(("sync_log".to_string(), log?));
        }
    }

    // SoliDB's own files (the transaction WAL and the like) move as they are.
    for entry in std::fs::read_dir(dir).map_err(|e| io("list", dir, e))? {
        let entry = entry.map_err(|e| io("list", dir, e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_file = entry.file_type().map(|t| t.is_file()).unwrap_or(false);
        if is_file && !is_rocksdb_file(&name) {
            std::fs::copy(entry.path(), target.join(&name))
                .map_err(|e| io("copy", &entry.path(), e))?;
        }
    }
    record(&target, EngineKind::Solistore)?;

    // Swap: the RocksDB directory becomes the backup, the copy takes its place.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    report.backup = sibling(dir, &format!(".rocksdb-backup-{stamp}"));
    std::fs::rename(dir, &report.backup).map_err(|e| io("rename", dir, e))?;
    if let Err(e) = std::fs::rename(&target, dir) {
        // Put the original back rather than leave no data directory.
        let _ = std::fs::rename(&report.backup, dir);
        return Err(io("rename", &target, e));
    }
    #[cfg(unix)]
    if let Some(parent) = dir.parent() {
        if let Ok(d) = std::fs::File::open(parent) {
            let _ = d.sync_all();
        }
    }
    progress("done");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rocksdb_directory_moves_to_solistore_with_its_data() {
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("data");
        std::fs::create_dir(&dir).unwrap();
        {
            let kv = super::super::rocks::open(&dir, &FAMILIES[1..]).unwrap();
            let shared = kv.cf_handle("__keyspaces__").unwrap();
            let meta = kv.cf_handle("_meta").unwrap();
            let mut batch = KvBatch::default();
            for i in 0..5000u32 {
                batch.put_cf(&shared, i.to_be_bytes(), format!("doc {i}"));
            }
            batch.put_cf(&meta, b"storage_format", b"2");
            kv.write(batch).unwrap();
            kv.put(b"default-key", b"v").unwrap();
            let log = super::super::rocks::open_sync_log(&dir.join("sync_log")).unwrap();
            log.put(b"sync_log:1", b"entry").unwrap();
        }
        std::fs::write(dir.join("transaction.wal"), b"wal bytes").unwrap();
        std::fs::write(dir.join("ENGINE"), b"rocksdb\n").unwrap();

        let report = to_solistore(&dir, &|_| {}).unwrap();
        assert_eq!(report.families[2].1.entries, 5000);
        assert!(report.backup.exists());

        let kv = super::super::open(&dir, &[]).unwrap();
        assert_eq!(kv.engine_kind(), EngineKind::Solistore);
        let shared = kv.cf_handle("__keyspaces__").unwrap();
        assert_eq!(
            kv.get_cf(&shared, 4999u32.to_be_bytes())
                .unwrap()
                .as_deref(),
            Some(&b"doc 4999"[..])
        );
        assert_eq!(kv.get(b"default-key").unwrap().as_deref(), Some(&b"v"[..]));
        assert_eq!(
            std::fs::read(dir.join("transaction.wal")).unwrap(),
            b"wal bytes"
        );
        let log = super::super::open_sync_log(&dir.join("sync_log")).unwrap();
        assert_eq!(log.engine_kind(), EngineKind::Solistore);
        assert_eq!(
            log.get(b"sync_log:1").unwrap().as_deref(),
            Some(&b"entry"[..])
        );
        // Migrating again is refused.
        assert!(to_solistore(&dir, &|_| {}).is_err());
    }

    /// A RocksDB data directory at `dir` with `entries` documents of
    /// `value_len` bytes in `__keyspaces__`, under two keyspace prefixes,
    /// and a sync log of `entries / 10`.
    fn rocksdb_dir(dir: &Path, entries: u32, value_len: usize) {
        std::fs::create_dir(dir).unwrap();
        let kv = super::super::rocks::open(dir, &FAMILIES[1..]).unwrap();
        let shared = kv.cf_handle("__keyspaces__").unwrap();
        let meta = kv.cf_handle("_meta").unwrap();
        // Bytes that do not compress to nothing, as documents do not.
        let mut x = 0x9E37_79B9_7F4A_7C15u64;
        let mut value = vec![0u8; value_len];
        let mut batch = KvBatch::default();
        for i in 0..entries {
            for b in value.iter_mut() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                *b = (x as u8) & 0x3f | 0x40;
            }
            let mut key = Vec::with_capacity(20);
            key.extend_from_slice(&(i % 2 + 1).to_be_bytes());
            key.extend_from_slice(&7u32.to_be_bytes());
            key.extend_from_slice(b"doc:");
            key.extend_from_slice(&i.to_be_bytes());
            batch.put_cf(&shared, &key, &value);
            if batch.size_in_bytes() >= 8 << 20 {
                kv.write(std::mem::take(&mut batch)).unwrap();
            }
        }
        batch.put_cf(&meta, b"storage_format", b"2");
        kv.write(batch).unwrap();
        {
            let log = super::super::rocks::open_sync_log(&dir.join("sync_log")).unwrap();
            let log_cf = log.cf_handle("default").unwrap();
            let mut batch = KvBatch::default();
            for i in 0..entries / 10 {
                batch.put_cf(
                    &log_cf,
                    format!("sync_log:{i:012}"),
                    &value[..value_len.min(200)],
                );
            }
            log.write(batch).unwrap();
        }
        drop(shared);
        drop(meta);
        drop(kv);
        std::fs::write(dir.join("ENGINE"), b"rocksdb\n").unwrap();
    }

    /// Enough data to need several tables; every document arrives, and the
    /// read-back matches the copy.
    #[test]
    fn a_large_copy_arrives_whole() {
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("data");
        // 6000 x 4 KiB: about 24 MiB.
        rocksdb_dir(&dir, 6000, 4096);
        let report = to_solistore(&dir, &|_| {}).unwrap();
        let shared = report
            .families
            .iter()
            .find(|(n, _)| n == "__keyspaces__")
            .unwrap()
            .1;
        assert_eq!(shared.entries, 6000);
        let log = report
            .families
            .iter()
            .find(|(n, _)| n == "sync_log")
            .unwrap()
            .1;
        assert_eq!(log.entries, 600);

        let kv = super::super::open(&dir, &[]).unwrap();
        let cf = kv.cf_handle("__keyspaces__").unwrap();
        let mut seen = 0u32;
        let mut hasher = DefaultHasher::new();
        let mut d = Digest::default();
        for item in kv.iterator_cf(&cf, IterMode::Start) {
            let (k, v) = item.unwrap();
            assert_eq!(v.len(), 4096);
            d.add(&mut hasher, &k, &v);
            seen += 1;
        }
        d.hash = hasher.finish();
        assert_eq!(seen, 6000);
        assert_eq!(d, shared);
    }

    /// Throughput on a generated directory of documents of ~1.1 KB, in
    /// `SOLIDB_MIGRATE_BENCH_DIR` (default: the current directory):
    /// `SOLIDB_MIGRATE_BENCH_ENTRIES=20000000 cargo test --profile ci
    /// --features engine-solistore --lib migration_throughput -- --ignored
    /// --nocapture`. Read the `migrated` line.
    #[test]
    #[ignore]
    fn migration_throughput() {
        let entries: u32 = std::env::var("SOLIDB_MIGRATE_BENCH_ENTRIES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500_000);
        let parent = tempfile::tempdir_in(
            std::env::var("SOLIDB_MIGRATE_BENCH_DIR").unwrap_or_else(|_| ".".into()),
        )
        .unwrap();
        let dir = parent.path().join("data");
        let t = std::time::Instant::now();
        rocksdb_dir(&dir, entries, 1100);
        eprintln!(
            "generated {entries} entries in {:.1}s",
            t.elapsed().as_secs_f64()
        );
        let started = std::time::Instant::now();
        let report = to_solistore(&dir, &|step| {
            eprintln!("[{:>7.1}s] {step}", started.elapsed().as_secs_f64())
        })
        .unwrap();
        let secs = started.elapsed().as_secs_f64();
        let bytes: u64 = report.families.iter().map(|(_, d)| d.bytes).sum();
        eprintln!(
            "migrated {:.2} GB in {secs:.1}s ({:.0} MB/s, {} threads)",
            bytes as f64 / 1e9,
            bytes as f64 / 1e6 / secs,
            ingest_threads()
        );
    }

    /// A dropped collection's column family still waiting for its deletion,
    /// and one whose database is gone, are left behind (the first with its
    /// marker); a live 1.x one stops the migration.
    #[test]
    fn pending_drops_are_left_out_and_other_legacy_families_refused() {
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("data");
        std::fs::create_dir(&dir).unwrap();
        {
            let kv = super::super::rocks::open(
                &dir,
                &["_meta", "__keyspaces__", "shop:gone", "dropped_db:_jobs"],
            )
            .unwrap();
            let meta = kv.cf_handle("_meta").unwrap();
            let gone = kv.cf_handle("shop:gone").unwrap();
            let orphan = kv.cf_handle("dropped_db:_jobs").unwrap();
            let mut batch = KvBatch::default();
            batch.put_cf(&gone, b"doc:1", b"old");
            batch.put_cf(&orphan, b"doc:1", b"old");
            batch.put_cf(&meta, b"db:shop", b"1");
            batch.put_cf(&meta, b"pending_drop:shop:gone", b"");
            batch.put_cf(&meta, b"storage_format", b"2");
            kv.write(batch).unwrap();
        }
        std::fs::write(dir.join("ENGINE"), b"rocksdb\n").unwrap();
        let report = to_solistore(&dir, &|_| {}).unwrap();
        let mut skipped = report.skipped.clone();
        skipped.sort();
        assert_eq!(skipped, vec!["dropped_db:_jobs", "shop:gone"]);
        let kv = super::super::open(&dir, &[]).unwrap();
        assert!(kv.cf_handle("shop:gone").is_none());
        assert!(kv.cf_handle("dropped_db:_jobs").is_none());
        let meta = kv.cf_handle("_meta").unwrap();
        assert!(kv
            .get_cf(&meta, b"pending_drop:shop:gone")
            .unwrap()
            .is_none());
        assert!(kv.get_cf(&meta, b"storage_format").unwrap().is_some());

        let other = parent.path().join("other");
        std::fs::create_dir(&other).unwrap();
        {
            let kv = super::super::rocks::open(&other, &["_meta", "shop:live"]).unwrap();
            let live = kv.cf_handle("shop:live").unwrap();
            let meta = kv.cf_handle("_meta").unwrap();
            kv.put_cf(&live, b"doc:1", b"kept").unwrap();
            kv.put_cf(&meta, b"db:shop", b"1").unwrap();
        }
        std::fs::write(other.join("ENGINE"), b"rocksdb\n").unwrap();
        let err = to_solistore(&other, &|_| {}).unwrap_err().to_string();
        assert!(err.contains("1.x column families"), "{err}");
    }
}
