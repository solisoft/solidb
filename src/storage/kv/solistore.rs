//! solistore under [`Kv`]: how SoliDB configures and opens it.

use std::path::Path;
use std::time::Duration;

use ::solistore::{Db, Durability, Options};

use super::{Kv, KvCf, KvError, KvMemory};
use crate::storage::engine::profile;

/// Durability of ordinary writes. `SOLIDB_DURABILITY=full` fsyncs every
/// commit (concurrent commits share the fsync); the default, eventual,
/// fsyncs in the background every `SOLIDB_DURABILITY_INTERVAL_MS` (100), so
/// a process crash loses nothing and a power loss at most that interval.
/// Transaction commits and other `sync` writes wait for their own fsync in
/// either mode.
fn durability() -> Durability {
    match std::env::var("SOLIDB_DURABILITY").as_deref() {
        Ok("full") => Durability::Full,
        _ => {
            let ms = std::env::var("SOLIDB_DURABILITY_INTERVAL_MS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|&ms| ms > 0)
                .unwrap_or(100);
            Durability::Eventual {
                sync_interval: Duration::from_millis(ms),
            }
        }
    }
}

/// Whether every write is fsynced before it returns
/// (`SOLIDB_DURABILITY=full`): a write then has nothing left to sync.
pub(super) fn syncs_every_write() -> bool {
    matches!(durability(), Durability::Full)
}

/// The process-wide [`crate::storage::engine::EngineProfile`] mapped onto
/// solistore's options. Everything shares one LSM, so the per-column-family
/// RocksDB knobs (arena block, global write-buffer budget) have no
/// counterpart.
fn options() -> Options {
    let p = profile();
    Options {
        durability: durability(),
        block_cache_size: p.block_cache_bytes as u64,
        write_buffer_size: p.write_buffer_size,
        max_immutable_memtables: (p.max_write_buffer_number.max(2) - 1) as usize,
        max_background_jobs: p.max_background_jobs.max(2) as usize,
        recycle_wal_files: recycled_wals(p.write_buffer_size),
        ..Options::default()
    }
}

/// How many dead WAL files solistore keeps to write over. A WAL grows to
/// about the size of the write buffer and a kept one stays that long on
/// disk: with a 2 GB buffer, 1.5 GB of dead log per file, measured (3.1 GB
/// of log beside 1.7 GB of tables once everything was flushed). Reusing a
/// file saves creating one per buffer filled, which matters when buffers
/// are small and fill often, and not when one holds gigabytes.
fn recycled_wals(write_buffer_size: usize) -> usize {
    const LARGE_BUFFER: usize = 256 << 20;
    if write_buffer_size > LARGE_BUFFER {
        0
    } else {
        Options::default().recycle_wal_files
    }
}

/// Open (or create) the main database at `path`, creating the column
/// families in `required` that are missing.
pub(crate) fn open(path: &Path, required: &[&str]) -> Result<Kv, KvError> {
    let db = Db::open(path, options())?;
    for name in required {
        if db.cf_handle(name).is_none() {
            db.create_cf(name)?;
        }
    }
    Ok(Kv::from(db))
}

/// Open (or create) the replication log's database: default column family
/// only.
pub(crate) fn open_sync_log(path: &Path) -> Result<Kv, KvError> {
    let opts = Options {
        write_buffer_size: 64 << 20,
        ..options()
    };
    Ok(Kv::from(Db::open(path, opts)?))
}

/// `multi_get` reads one column family at one snapshot; keys spanning
/// several are read one by one.
pub(super) fn multi_get<'c, 'h: 'c, K: AsRef<[u8]>>(
    db: &Db,
    keys: impl IntoIterator<Item = (&'c KvCf<'h>, K)>,
) -> Vec<Result<Option<Vec<u8>>, KvError>> {
    let keys: Vec<(&::solistore::ColumnFamily, K)> = keys
        .into_iter()
        .map(|(cf, k)| match cf {
            KvCf::Solistore(h) => (h, k),
            #[cfg(feature = "engine-rocksdb")]
            KvCf::Rocks(_) => super::mixed_engines(),
            #[cfg(not(feature = "engine-rocksdb"))]
            #[allow(unreachable_patterns)]
            _ => unreachable!("built by no path without RocksDB"),
        })
        .collect();
    let Some((first, _)) = keys.first() else {
        return Vec::new();
    };
    let one_cf = keys.iter().all(|(cf, _)| cf.name() == first.name());
    if one_cf {
        let only: Vec<&K> = keys.iter().map(|(_, k)| k).collect();
        let only: Vec<&[u8]> = only.iter().map(|k| k.as_ref()).collect();
        // The values are lent, and copied once, into the vectors returned.
        let mut out: Vec<Result<Option<Vec<u8>>, KvError>> = Vec::with_capacity(only.len());
        out.resize_with(only.len(), || Ok(None));
        return match db.multi_get_with(first, &only, |i, v| out[i] = Ok(v.map(<[u8]>::to_vec))) {
            Ok(()) => out,
            Err(e) => {
                let e = KvError::from(e);
                keys.iter().map(|_| Err(e.clone())).collect()
            }
        };
    }
    keys.iter()
        .map(|(cf, k)| {
            db.get(cf, k)
                .map(|v| v.map(|v| v.to_vec()))
                .map_err(KvError::from)
        })
        .collect()
}

pub(super) fn memory(db: &Db) -> KvMemory {
    let memtables = db.memtable_bytes() as u64;
    let readers = db.table_readers_bytes() as u64;
    KvMemory {
        memtable_bytes: memtables,
        memtable_total_bytes: memtables,
        table_readers_bytes: readers,
        table_files: db.files_per_level().iter().sum::<usize>() as u64,
        block_cache_bytes: db.block_cache_usage(),
        // Index and filter blocks are charged to the cache and never evicted.
        block_cache_pinned_bytes: readers,
    }
}
