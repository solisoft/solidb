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

static CF_OP_COUNT: AtomicU64 = AtomicU64::new(0);
static CF_OP_NANOS: AtomicU64 = AtomicU64::new(0);

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_accumulates_count_and_duration() {
        let before = snapshot();
        timed(|| std::thread::sleep(std::time::Duration::from_millis(5)));
        timed(|| ());
        let after = snapshot();

        assert_eq!(before.ops_since(&after), 2);
        assert!(before.ms_since(&after) >= 5.0);
    }
}
