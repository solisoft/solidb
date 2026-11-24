//! Which engine a data directory is opened with.
//!
//! A directory is always opened with the engine that created it, recorded
//! in its `ENGINE` file. A directory without one that holds RocksDB's
//! `OPTIONS-*` files predates the file and is RocksDB's; one that holds a
//! database of neither kind is refused rather than guessed at (both engines
//! write a `CURRENT`). A new directory gets the engine named by
//! `SOLIDB_ENGINE` (`solistore` or `rocksdb`), and solistore when it names
//! none; a binary built without the `engine-solistore` feature has only
//! RocksDB to give. The file is written once the open has succeeded.
//! Checkpoints carry it too.
//!
//! Only a new directory is concerned: one that holds a database keeps its
//! engine until `solidb-migrate-engine` is run on it.

use std::fmt;
use std::path::Path;

use super::{Kv, KvError};

const ENGINE_FILE: &str = "ENGINE";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineKind {
    Rocksdb,
    Solistore,
}

impl EngineKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EngineKind::Rocksdb => "rocksdb",
            EngineKind::Solistore => "solistore",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "rocksdb" => Some(EngineKind::Rocksdb),
            "solistore" => Some(EngineKind::Solistore),
            _ => None,
        }
    }
}

impl fmt::Display for EngineKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl EngineKind {
    /// The engine of a new directory when `SOLIDB_ENGINE` names none:
    /// solistore, in a binary that has it.
    pub fn default_for_new() -> Self {
        if cfg!(feature = "engine-solistore") {
            EngineKind::Solistore
        } else {
            EngineKind::Rocksdb
        }
    }
}

/// The engine for a new directory.
fn configured() -> Result<EngineKind, KvError> {
    named(std::env::var("SOLIDB_ENGINE").ok().as_deref())
}

/// The engine `SOLIDB_ENGINE` asks for, given its value.
fn named(variable: Option<&str>) -> Result<EngineKind, KvError> {
    match variable {
        Some(name) if !name.trim().is_empty() => EngineKind::parse(name).ok_or_else(|| {
            KvError::new(format!(
                "SOLIDB_ENGINE={name}: expected \"solistore\" or \"rocksdb\""
            ))
        }),
        _ => Ok(EngineKind::default_for_new()),
    }
}

/// The engine recorded for `path`, if the directory already holds a database.
pub(super) fn recorded(path: &Path) -> Result<Option<EngineKind>, KvError> {
    match std::fs::read_to_string(path.join(ENGINE_FILE)) {
        // An empty file is one that was created and never reached the disk:
        // through 2.14.0 the marker was written without an fsync, and a
        // power loss soon after a directory's creation left it there
        // (card #863). It says nothing, as if it were absent.
        Ok(name) if name.trim().is_empty() => unmarked(path),
        Ok(name) => EngineKind::parse(&name).map(Some).ok_or_else(|| {
            KvError::new(format!(
                "{}: unknown storage engine {:?}",
                path.join(ENGINE_FILE).display(),
                name.trim()
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => unmarked(path),
        Err(e) => Err(KvError::new(format!(
            "{}: {e}",
            path.join(ENGINE_FILE).display()
        ))),
    }
}

/// What a directory without a marker holds: RocksDB's when its `OPTIONS-*`
/// files are there, nothing when it holds no database, and otherwise a
/// database this cannot name.
fn unmarked(path: &Path) -> Result<Option<EngineKind>, KvError> {
    let rocks_options = std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("OPTIONS-"))
        })
        .unwrap_or(false);
    if rocks_options {
        Ok(Some(EngineKind::Rocksdb))
    } else if path.join("CURRENT").exists() {
        Err(KvError::new(format!(
            "{} holds a database but no {ENGINE_FILE} file naming its engine",
            path.display()
        )))
    } else {
        Ok(None)
    }
}

/// Write the marker so that it is either absent or whole after a power
/// loss: to a file of its own, fsynced, renamed into place, and the
/// directory fsynced.
pub(super) fn record(path: &Path, kind: EngineKind) -> Result<(), KvError> {
    let file = path.join(ENGINE_FILE);
    if std::fs::read_to_string(&file).is_ok_and(|name| !name.trim().is_empty()) {
        return Ok(());
    }
    let io = |at: &Path, e: std::io::Error| KvError::new(format!("{}: {e}", at.display()));
    std::fs::create_dir_all(path).map_err(|e| io(path, e))?;
    let tmp = path.join(format!("{ENGINE_FILE}.tmp"));
    {
        use std::io::Write;
        let mut out = std::fs::File::create(&tmp).map_err(|e| io(&tmp, e))?;
        out.write_all(format!("{kind}\n").as_bytes())
            .and_then(|()| out.sync_all())
            .map_err(|e| io(&tmp, e))?;
    }
    std::fs::rename(&tmp, &file).map_err(|e| io(&file, e))?;
    // The new name is in the directory: that has to reach the disk too. A
    // directory cannot be opened as a file on Windows.
    #[cfg(unix)]
    std::fs::File::open(path)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| io(path, e))?;
    Ok(())
}

fn open_with(
    path: &Path,
    rocks: impl FnOnce() -> Result<Kv, KvError>,
    #[allow(unused_variables)] solistore: impl FnOnce() -> Result<Kv, KvError>,
) -> Result<Kv, KvError> {
    let kind = match recorded(path)? {
        Some(kind) => kind,
        None => configured()?,
    };
    // Before the engine creates anything: a directory that holds an
    // engine's files then always says whose they are. Recorded afterwards,
    // a power loss between the two left a database no start could name.
    // Not for an engine this binary cannot open, which is refused below.
    let can_open = match kind {
        EngineKind::Rocksdb => cfg!(feature = "engine-rocksdb"),
        EngineKind::Solistore => cfg!(feature = "engine-solistore"),
    };
    if can_open {
        record(path, kind)?;
    }
    let kv = match kind {
        #[cfg(feature = "engine-rocksdb")]
        EngineKind::Rocksdb => rocks()?,
        #[cfg(not(feature = "engine-rocksdb"))]
        EngineKind::Rocksdb => {
            let _ = rocks;
            return Err(KvError::new(format!(
                "{} holds a RocksDB database, which SoliDB 3.0 no longer opens. \
                 Stop the server, run `solidb-migrate-engine {}` from SoliDB 2.16 \
                 (it converts the directory to solistore in place), then start 3.0; \
                 or build 3.0 with `--features engine-rocksdb`",
                path.display(),
                path.display()
            )));
        }
        #[cfg(feature = "engine-solistore")]
        EngineKind::Solistore => solistore()?,
        #[cfg(not(feature = "engine-solistore"))]
        EngineKind::Solistore => {
            return Err(KvError::new(format!(
                "{} uses the solistore engine; this binary was built without the \
                 engine-solistore feature",
                path.display()
            )))
        }
    };
    record(path, kind)?;
    Ok(kv)
}

/// Open (or create) the main database at `path` with every existing column
/// family, creating those in `required` that are missing.
pub fn open(path: &Path, required: &[&str]) -> Result<Kv, KvError> {
    open_with(
        path,
        || {
            #[cfg(feature = "engine-rocksdb")]
            return super::rocks::open(path, required);
            #[cfg(not(feature = "engine-rocksdb"))]
            unreachable!()
        },
        || {
            #[cfg(feature = "engine-solistore")]
            return super::solistore::open(path, required);
            #[cfg(not(feature = "engine-solistore"))]
            unreachable!()
        },
    )
}

/// Open (or create) the replication log's database at `path`.
pub fn open_sync_log(path: &Path) -> Result<Kv, KvError> {
    open_with(
        path,
        || {
            #[cfg(feature = "engine-rocksdb")]
            return super::rocks::open_sync_log(path);
            #[cfg(not(feature = "engine-rocksdb"))]
            unreachable!()
        },
        || {
            #[cfg(feature = "engine-solistore")]
            return super::solistore::open_sync_log(path);
            #[cfg(not(feature = "engine-solistore"))]
            unreachable!()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 3.0 (card #992): a RocksDB directory, marked or not, is refused with
    /// what to do about it, and nothing of it is changed.
    #[cfg(not(feature = "engine-rocksdb"))]
    #[test]
    fn a_rocksdb_directory_is_refused_with_the_way_to_migrate() {
        for marked in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            if marked {
                write_engine(dir.path(), "rocksdb");
            } else {
                std::fs::write(dir.path().join("OPTIONS-000007"), b"").unwrap();
            }
            let err = open(dir.path(), &[]).err().expect("refused");
            assert!(err.to_string().contains("solidb-migrate-engine"), "{err}");
            assert!(!dir.path().join("CURRENT").exists());
        }
    }

    fn write_engine(dir: &Path, name: &str) {
        std::fs::write(dir.join(ENGINE_FILE), format!("{name}\n")).unwrap();
    }

    /// The marker a power loss left empty (written without an fsync
    /// through 2.14.0) says nothing: an empty directory is a new one, a
    /// RocksDB directory is RocksDB's, and the marker is written again,
    /// whole.
    #[cfg(feature = "engine-rocksdb")]
    #[test]
    fn an_empty_marker_is_read_as_no_marker_and_written_again() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(ENGINE_FILE), b"").unwrap();
        assert_eq!(recorded(dir.path()).unwrap(), None);
        record(dir.path(), EngineKind::Rocksdb).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join(ENGINE_FILE)).unwrap(),
            "rocksdb\n"
        );
        assert!(!dir.path().join("ENGINE.tmp").exists());
        assert_eq!(recorded(dir.path()).unwrap(), Some(EngineKind::Rocksdb));
        // A marker in place is left alone.
        record(dir.path(), EngineKind::Solistore).unwrap();
        assert_eq!(recorded(dir.path()).unwrap(), Some(EngineKind::Rocksdb));

        let rocks = tempfile::tempdir().unwrap();
        drop(super::super::rocks::open(rocks.path(), &["x"]).unwrap());
        std::fs::write(rocks.path().join(ENGINE_FILE), b"").unwrap();
        assert_eq!(recorded(rocks.path()).unwrap(), Some(EngineKind::Rocksdb));
    }

    /// The marker is there before the engine has created anything, in a
    /// directory that does not exist yet too.
    #[test]
    fn a_new_directory_is_marked_before_its_engine_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh");
        let kind = configured().unwrap();
        let seen = std::cell::Cell::new(None);
        let look = || {
            seen.set(Some(recorded(&path).unwrap()));
            Err(KvError::new("stop here".to_string()))
        };
        assert!(open_with(&path, look, look).is_err());
        assert_eq!(seen.get(), Some(Some(kind)));
    }

    #[cfg(feature = "engine-solistore")]
    #[test]
    fn a_new_directory_is_solistore_unless_the_variable_names_rocksdb() {
        assert_eq!(named(None).unwrap(), EngineKind::Solistore);
        assert_eq!(named(Some("  ")).unwrap(), EngineKind::Solistore);
        assert_eq!(named(Some("solistore")).unwrap(), EngineKind::Solistore);
        assert_eq!(named(Some("rocksdb")).unwrap(), EngineKind::Rocksdb);
        assert!(named(Some("leveldb")).is_err());
    }

    #[cfg(not(feature = "engine-solistore"))]
    #[test]
    fn a_new_directory_is_rocksdb_in_a_binary_without_solistore() {
        assert_eq!(named(None).unwrap(), EngineKind::Rocksdb);
        assert_eq!(named(Some("rocksdb")).unwrap(), EngineKind::Rocksdb);
    }

    /// The default does not reach a directory that holds a database: one
    /// made by RocksDB before the `ENGINE` file existed stays RocksDB's.
    #[cfg(feature = "engine-rocksdb")]
    #[test]
    fn a_directory_that_holds_a_rocksdb_database_keeps_it_whatever_the_default() {
        let dir = tempfile::tempdir().unwrap();
        {
            let kv = super::super::rocks::open(dir.path(), &["x"]).unwrap();
            kv.put(b"k", b"v").unwrap();
        }
        for _ in 0..2 {
            let kv = open(dir.path(), &["x"]).unwrap();
            assert_eq!(kv.engine_kind(), EngineKind::Rocksdb);
            assert_eq!(kv.get(b"k").unwrap().as_deref(), Some(&b"v"[..]));
        }
    }

    #[cfg(feature = "engine-rocksdb")]
    #[test]
    fn a_directory_is_opened_with_the_engine_it_records() {
        let dir = tempfile::tempdir().unwrap();
        write_engine(dir.path(), "rocksdb");
        let kv = open(dir.path(), &["x"]).unwrap();
        assert_eq!(kv.engine_kind(), EngineKind::Rocksdb);
        assert!(kv.cf_handle("x").is_some());
    }

    #[cfg(feature = "engine-solistore")]
    #[test]
    fn a_solistore_directory_opens_as_solistore_and_reopens_with_its_data() {
        let dir = tempfile::tempdir().unwrap();
        write_engine(dir.path(), "solistore");
        {
            let kv = open(dir.path(), &["x"]).unwrap();
            assert_eq!(kv.engine_kind(), EngineKind::Solistore);
            let cf = kv.cf_handle("x").unwrap();
            let mut batch = crate::storage::kv::KvBatch::default();
            batch.put_cf(&cf, b"k", b"v");
            kv.write_opt(batch, &crate::storage::kv::KvWriteOptions::synced())
                .unwrap();
            kv.put(b"meta", b"1").unwrap();
        }
        let kv = open(dir.path(), &["x"]).unwrap();
        let cf = kv.cf_handle("x").unwrap();
        assert_eq!(kv.get_cf(&cf, b"k").unwrap().as_deref(), Some(&b"v"[..]));
        assert_eq!(kv.get(b"meta").unwrap().as_deref(), Some(&b"1"[..]));
        let target = dir.path().join("checkpoint");
        kv.checkpoint(&target).unwrap();
        assert_eq!(recorded(&target).unwrap(), Some(EngineKind::Solistore));
    }

    #[cfg(not(feature = "engine-solistore"))]
    #[test]
    fn a_solistore_directory_is_refused_without_the_feature() {
        let dir = tempfile::tempdir().unwrap();
        write_engine(dir.path(), "solistore");
        let err = open(dir.path(), &[]).err().expect("refused");
        assert!(err.to_string().contains("engine-solistore"), "{err}");
    }

    #[cfg(feature = "engine-rocksdb")]
    #[test]
    fn a_rocksdb_directory_from_before_the_engine_file_is_recognised() {
        let dir = tempfile::tempdir().unwrap();
        drop(super::super::rocks::open(dir.path(), &[]).unwrap());
        assert!(!dir.path().join(ENGINE_FILE).exists());
        assert_eq!(recorded(dir.path()).unwrap(), Some(EngineKind::Rocksdb));
        let kv = open(dir.path(), &[]).unwrap();
        assert_eq!(kv.engine_kind(), EngineKind::Rocksdb);
        assert!(dir.path().join(ENGINE_FILE).exists());
        let target = dir.path().join("checkpoint");
        kv.checkpoint(&target).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.join(ENGINE_FILE)).unwrap(),
            "rocksdb\n"
        );
    }

    #[test]
    fn an_unmarked_database_of_unknown_kind_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CURRENT"), "MANIFEST-000001\n").unwrap();
        assert!(recorded(dir.path()).is_err());
        assert!(open(dir.path(), &[]).is_err());
    }

    #[test]
    fn an_unknown_engine_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_engine(dir.path(), "leveldb");
        assert!(open(dir.path(), &[]).is_err());
    }
}
