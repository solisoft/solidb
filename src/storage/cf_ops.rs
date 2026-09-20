//! Global column-family operation tracker.
//!
//! Every `create_cf`/`drop_cf` rewrites + fsyncs the entire OPTIONS file
//! (one section per CF) under the RocksDB DB mutex — hundreds of
//! milliseconds on instances with many CFs (see `pending_drops.rs`). While
//! one holds the mutex, writes in *every* database stall behind it, so an
//! innocent query can show up in `_slow_queries` purely as a contention
//! victim of CF churn elsewhere (e.g. a test suite creating/dropping spec
//! databases).
//!
//! This module keeps process-wide counters of CF-op count and wall time.
//! The slow-query logger snapshots them around query execution and stamps
//! each entry with the CF activity that overlapped it — distinguishing
//! "this query is slow" from "this query queued behind CF operations".

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use rust_rocksdb::{IteratorMode, WriteBatch};

use super::RocksDb as DB;
use crate::error::{DbError, DbResult};

static CF_OP_COUNT: AtomicU64 = AtomicU64::new(0);
static CF_OP_NANOS: AtomicU64 = AtomicU64::new(0);
static CF_REUSE_COUNT: AtomicU64 = AtomicU64::new(0);
static AUTO_CREATE_COUNT: AtomicU64 = AtomicU64::new(0);

/// Cumulative CF-op activity since process start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CfOpSnapshot {
    pub ops: u64,
    pub nanos: u64,
}

/// Snapshot the cumulative counters. Take one before and one after a query
/// and subtract to get the CF activity that overlapped its execution.
pub fn snapshot() -> CfOpSnapshot {
    CfOpSnapshot {
        ops: CF_OP_COUNT.load(Ordering::Relaxed),
        nanos: CF_OP_NANOS.load(Ordering::Relaxed),
    }
}

impl CfOpSnapshot {
    /// CF ops that completed between `self` (before) and `later` (after).
    pub fn ops_since(&self, later: &CfOpSnapshot) -> u64 {
        later.ops.saturating_sub(self.ops)
    }

    /// Milliseconds spent inside CF ops between `self` and `later`.
    pub fn ms_since(&self, later: &CfOpSnapshot) -> f64 {
        later.nanos.saturating_sub(self.nanos) as f64 / 1_000_000.0
    }
}

/// Run a `create_cf`/`drop_cf` call, adding its duration to the global
/// counters. Wrap every CF op call site with this.
pub fn timed<R>(op: impl FnOnce() -> R) -> R {
    let start = Instant::now();
    let result = op();
    CF_OP_COUNT.fetch_add(1, Ordering::Relaxed);
    CF_OP_NANOS.fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
    result
}

/// How many times a doomed column family was handed to a new collection
/// instead of being dropped and recreated. Each one avoids *two* OPTIONS
/// rewrites.
///
/// Counted by the caller that actually reuses, not by [`wipe_cf`] — wiping
/// also happens on deletion, where nothing is reused.
pub fn reuses() -> u64 {
    CF_REUSE_COUNT.load(Ordering::Relaxed)
}

/// Record that a create claimed and reused an existing column family.
pub fn record_reuse() {
    CF_REUSE_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Collections brought into existence by a write to a name that did not
/// exist yet.
///
/// Each one is a `create_cf`, so this is the rate at which ordinary traffic
/// grows the OPTIONS file. Measured on a dev instance: 11 241 auto-creations
/// across 321 distinct names in eighteen days, one name 358 times.
pub fn autocreates() -> u64 {
    AUTO_CREATE_COUNT.load(Ordering::Relaxed)
}

/// Record that a write created a collection that did not exist.
pub fn record_autocreate() {
    AUTO_CREATE_COUNT.fetch_add(1, Ordering::Relaxed);
}

/// Whether a write to an unknown collection may create it.
///
/// On by default — applications rely on it — but an instance whose schema is
/// managed elsewhere can turn it off and get a `CollectionNotFound` instead of
/// silently growing the column-family count on a typo.
pub fn auto_create_enabled() -> bool {
    !matches!(
        std::env::var("SOLIDB_AUTO_CREATE_COLLECTIONS")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "0" | "false" | "off" | "no"
    )
}

/// Erase every key in a column family so it can be handed to a fresh
/// collection in place of a `drop_cf` + `create_cf` pair.
///
/// The pair is what a create-after-delete of the same name used to cost: two
/// full OPTIONS rewrites, each proportional to the *total* CF count of the
/// instance. Wiping is a pair of seeks and one range tombstone, and every CF
/// is built from the same `tuned_cf_options()`, so a reused one is
/// indistinguishable from a new one.
///
/// The bounds are taken from the column family's own first and last key
/// rather than from a synthetic `\xff` sentinel, so this is correct whatever
/// bytes the keys contain — index entries are hex-encoded and blob chunk keys
/// embed user-chosen document keys.
pub fn wipe_cf(db: &DB, cf_name: &str) -> DbResult<()> {
    let cf = db
        .cf_handle(cf_name)
        .ok_or_else(|| DbError::InternalError(format!("column family '{cf_name}' missing")))?;

    let first = db
        .iterator_cf(&cf, IteratorMode::Start)
        .next()
        .and_then(|r| r.ok())
        .map(|(k, _)| k);
    let last = db
        .iterator_cf(&cf, IteratorMode::End)
        .next()
        .and_then(|r| r.ok())
        .map(|(k, _)| k);

    if let (Some(first), Some(last)) = (first, last) {
        // `delete_range` is [start, end), so the end bound must be strictly
        // greater than the last key. Appending a zero byte is the shortest
        // key that is.
        let mut end = last.to_vec();
        end.push(0);

        let mut batch = WriteBatch::default();
        // Both bounds as `&[u8]`: `delete_range_cf` takes one type for the
        // pair, and the iterator hands back a `Box<[u8]>`.
        batch.delete_range_cf(&cf, first.as_ref(), end.as_slice());
        db.write(&batch).map_err(|e| {
            DbError::InternalError(format!("Failed to wipe column family '{cf_name}': {e}"))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_accumulates_count_and_duration() {
        let before = snapshot();
        timed(|| std::thread::sleep(std::time::Duration::from_millis(5)));
        timed(|| ());
        let after = snapshot();

        // Counters are process-wide statics, so other tests running in
        // parallel may also bump them. Assert at least our two calls landed.
        assert!(before.ops_since(&after) >= 2);
        assert!(before.ms_since(&after) >= 5.0);
    }
}
