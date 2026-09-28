//! Background compaction of dropped keyspaces.
//!
//! Dropping a shared-layout collection (or a whole database) is one range
//! tombstone plus a `dead_ks:` marker in `_meta`, written atomically with the
//! registry change. The tombstone makes the data invisible at once; the disk
//! space comes back when compaction drops the covered keys. This thread
//! compacts each marked range and then clears its marker, so the space is
//! reclaimed soon after a drop instead of whenever compaction happens to
//! reach those files. Markers survive a crash and are resumed at startup.
//!
//! One thread, one range at a time, after a short delay so a burst of drops
//! (a test suite tearing down its databases) is handled in one pass.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::Duration;

use super::collection_registry::dead_ranges;
use super::engine::META_CF;
use super::keyspace::SHARED_CF;
use super::RocksDb as DB;

/// Seconds between a drop and the compaction of its range.
fn delay() -> Duration {
    let secs = std::env::var("SOLIDB_KS_GC_DELAY_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(10);
    Duration::from_secs(secs)
}

static COMPACTIONS: AtomicU64 = AtomicU64::new(0);

/// Dead keyspace ranges compacted since process start.
pub fn compactions() -> u64 {
    COMPACTIONS.load(Ordering::Relaxed)
}

struct Signal {
    pending: Mutex<bool>,
    cv: Condvar,
}

static SIGNAL: Signal = Signal {
    pending: Mutex::new(false),
    cv: Condvar::new(),
};

/// Tell the collector there is work. Cheap; safe to call without a running
/// collector.
pub fn wake() {
    if let Ok(mut p) = SIGNAL.pending.lock() {
        *p = true;
        SIGNAL.cv.notify_one();
    }
}

/// Compact every marked range now and clear the markers. Returns how many
/// ranges were processed.
pub fn run_once(db: &DB) -> usize {
    let (Some(meta_cf), Some(shared)) = (db.cf_handle(META_CF), db.cf_handle(SHARED_CF)) else {
        return 0;
    };
    let mut done = 0;
    for (marker, lo, hi) in dead_ranges(db) {
        db.compact_range_cf(&shared, Some(&lo), Some(&hi));
        if let Err(e) = db.delete_cf(&meta_cf, &marker) {
            tracing::warn!("keyspace GC: failed to clear marker: {}", e);
            continue;
        }
        COMPACTIONS.fetch_add(1, Ordering::Relaxed);
        done += 1;
    }
    done
}

static STARTED: std::sync::Once = std::sync::Once::new();

/// Start the collector (once per process). Holds only a weak handle, so it
/// never keeps a dropped engine's RocksDB open; it exits when the handle dies.
pub fn ensure_started(db: &Arc<DB>) {
    let weak: Weak<DB> = Arc::downgrade(db);
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new()
            .name("solidb-keyspace-gc".into())
            .spawn(move || loop {
                {
                    let Ok(mut pending) = SIGNAL.pending.lock() else {
                        return;
                    };
                    // Resume leftovers at startup, then wait for a drop.
                    while !*pending {
                        let (guard, _) = SIGNAL
                            .cv
                            .wait_timeout(pending, Duration::from_secs(60))
                            .unwrap_or_else(|e| e.into_inner());
                        pending = guard;
                        if weak.strong_count() == 0 {
                            return;
                        }
                        if !*pending {
                            break; // periodic sweep
                        }
                    }
                    *pending = false;
                }
                std::thread::sleep(delay());
                let Some(db) = weak.upgrade() else {
                    return;
                };
                run_once(&db);
            });
    });
    wake();
}
