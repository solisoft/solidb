//! Which engine a data directory is opened with.
//!
//! A directory is always opened with the engine that created it, recorded
//! in its `ENGINE` file. A directory without one that holds RocksDB's
//! `OPTIONS-*` files predates the file and is RocksDB's; one that holds a
//! database of neither kind is refused rather than guessed at (both engines
//! write a `CURRENT`). A new directory gets the engine named by
//! `SOLIDB_ENGINE` (`rocksdb`, the default, or `solistore`), and the file is
//! written once the open has succeeded. Checkpoints carry it too.

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

/// The engine for a new directory.
fn configured() -> Result<EngineKind, KvError> {
    match std::env::var("SOLIDB_ENGINE") {
        Ok(name) if !name.trim().is_empty() => EngineKind::parse(&name).ok_or_else(|| {
            KvError::new(format!(
                "SOLIDB_ENGINE={name}: expected \"rocksdb\" or \"solistore\""
            ))
        }),
        _ => Ok(EngineKind::Rocksdb),
    }
}

/// The engine recorded for `path`, if the directory already holds a database.
pub(super) fn recorded(path: &Path) -> Result<Option<EngineKind>, KvError> {
    match std::fs::read_to_string(path.join(ENGINE_FILE)) {
        Ok(name) => EngineKind::parse(&name).map(Some).ok_or_else(|| {
            KvError::new(format!(
                "{}: unknown storage engine {:?}",
                path.join(ENGINE_FILE).display(),
                name.trim()
            ))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
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
        Err(e) => Err(KvError::new(format!(
            "{}: {e}",
            path.join(ENGINE_FILE).display()
        ))),
    }
}

pub(super) fn record(path: &Path, kind: EngineKind) -> Result<(), KvError> {
    let file = path.join(ENGINE_FILE);
    if file.exists() {
        return Ok(());
    }
    std::fs::write(&file, format!("{kind}\n"))
        .map_err(|e| KvError::new(format!("{}: {e}", file.display())))
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
    let kv = match kind {
        EngineKind::Rocksdb => rocks()?,
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
        || super::rocks::open(path, required),
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
        || super::rocks::open_sync_log(path),
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

    fn write_engine(dir: &Path, name: &str) {
        std::fs::write(dir.join(ENGINE_FILE), format!("{name}\n")).unwrap();
    }

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
