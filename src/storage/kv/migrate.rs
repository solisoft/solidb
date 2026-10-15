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
use super::{IterMode, Kv, KvBatch, KvError};

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

/// Copy one column family in key order, in batches of about 4 MiB.
fn copy(from: &Kv, to: &Kv, name: &str) -> Result<Digest, KvError> {
    let Some(src) = from.cf_handle(name) else {
        return Ok(Digest::default());
    };
    let dst = to
        .cf_handle(name)
        .ok_or_else(|| KvError::new(format!("target has no column family {name}")))?;
    let mut d = Digest::default();
    let mut hasher = DefaultHasher::new();
    let mut batch = KvBatch::default();
    for item in from.iterator_cf(&src, IterMode::Start) {
        let (k, v) = item?;
        d.add(&mut hasher, &k, &v);
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
}

fn sibling(dir: &Path, suffix: &str) -> PathBuf {
    let mut name = dir.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    dir.with_file_name(name)
}

fn io(what: &str, path: &Path, e: std::io::Error) -> KvError {
    KvError::new(format!("{what} {}: {e}", path.display()))
}

/// Migrate the RocksDB data directory `dir` to solistore, in place. On
/// success, `dir` is a solistore directory and the RocksDB one is kept at
/// `Report::backup`. On failure, `dir` is untouched.
pub fn to_solistore(dir: &Path, progress: &dyn Fn(&str)) -> Result<Report, KvError> {
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
        let unknown: Vec<String> = source
            .cf_names()
            .into_iter()
            .filter(|n| !FAMILIES.contains(&n.as_str()))
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
        let copy_of = super::solistore::open(&target, &FAMILIES[1..])?;
        for name in FAMILIES {
            progress(&format!("copying {name}"));
            let copied = copy(&source, &copy_of, name)?;
            report.families.push((name.to_string(), copied));
        }
        copy_of.flush()?;
        progress("verifying");
        for (name, copied) in &report.families {
            let got = digest(&copy_of, name)?;
            if got != *copied {
                return Err(KvError::new(format!(
                    "verification failed for {name}: copied {copied:?}, read back {got:?}"
                )));
            }
        }
    }

    // The replication log: its own database in `sync_log/`.
    let log_dir = dir.join("sync_log");
    if log_dir.exists() {
        progress("copying sync_log");
        let source = super::rocks::open_sync_log(&log_dir)?;
        let target_log = target.join("sync_log");
        let copy_of = super::solistore::open_sync_log(&target_log)?;
        let copied = copy(&source, &copy_of, "default")?;
        copy_of.flush()?;
        let got = digest(&copy_of, "default")?;
        if got != copied {
            return Err(KvError::new(format!(
                "verification failed for sync_log: copied {copied:?}, read back {got:?}"
            )));
        }
        record(&target_log, EngineKind::Solistore)?;
        report.families.push(("sync_log".to_string(), copied));
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
}
