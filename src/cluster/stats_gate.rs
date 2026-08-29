//! Change gate for the per-collection background sweeps.
//!
//! The cluster stats collector and the heartbeat both walk *every* collection
//! in *every* database every 5 seconds. On an instance with a few thousand
//! collections that is the dominant background cost even when nothing is
//! happening: measured on a dev box with 89 databases and 1718 collections, the
//! collector alone rewrote ~3400 documents per cycle (~690 writes/s, 177 KB/s
//! of WAL) and the pair kept 50-95% of a core busy plus 14 MB/s of allocator
//! churn, on a server with no client traffic at all.
//!
//! Nearly all of that work reproduces a result identical to the previous
//! cycle's. This gate remembers, per collection, the cheap in-memory counters
//! (document and blob-chunk counts, both plain atomics) and skips the
//! expensive part while they are unchanged. Anything that does not move those
//! counters — a compaction changing on-disk size, an update that rewrites a
//! document in place — is still picked up, just at the slower `full_refresh`
//! cadence rather than every 5 seconds.

use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

/// The cheap change signal: `(document_count, chunk_count)`, both read from
/// atomics on a cached collection handle.
pub type Counts = (usize, usize);

struct Entry<T> {
    counts: Counts,
    refreshed_at: Instant,
    value: T,
}

/// Per-collection gate deciding whether a sweep needs to redo its expensive
/// work, keyed by whatever string the caller uses to identify a collection.
///
/// `T` is whatever the caller wants to carry across cycles: the stats
/// collector stores a hash of the document it last wrote (so an unchanged
/// document is not rewritten), the heartbeat stores the summed figures (so it
/// can keep totalling without re-reading RocksDB).
pub struct StatsGate<T> {
    entries: Mutex<HashMap<String, Entry<T>>>,
    full_refresh: Duration,
}

impl<T: Clone> StatsGate<T> {
    /// `full_refresh` bounds how stale a gated entry can get: a collection
    /// whose counters never move is still recomputed this often.
    pub fn new(full_refresh: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            full_refresh,
        }
    }

    /// Whether `key` must be recomputed this cycle — true when it has never
    /// been seen, when its counters moved, or when its periodic refresh is due.
    pub fn needs_refresh(&self, key: &str, counts: Counts) -> bool {
        match self.entries.lock().get(key) {
            Some(entry) => {
                entry.counts != counts || entry.refreshed_at.elapsed() >= self.full_refresh
            }
            None => true,
        }
    }

    /// The value stored by the last refresh, if any.
    pub fn cached(&self, key: &str) -> Option<T> {
        self.entries.lock().get(key).map(|e| e.value.clone())
    }

    /// Record the result of a refresh, restarting this key's refresh clock.
    pub fn record(&self, key: &str, counts: Counts, value: T) {
        self.entries.lock().insert(
            key.to_string(),
            Entry {
                counts,
                refreshed_at: Instant::now(),
                value,
            },
        );
    }

    /// Forget collections that no longer exist, so a long-lived sweep does not
    /// accumulate entries for dropped collections.
    pub fn retain(&self, live: &HashSet<String>) {
        self.entries.lock().retain(|key, _| live.contains(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unseen_key_needs_refresh() {
        let gate: StatsGate<u64> = StatsGate::new(Duration::from_secs(300));
        assert!(gate.needs_refresh("a", (0, 0)));
        assert_eq!(gate.cached("a"), None);
    }

    #[test]
    fn unchanged_counts_are_gated() {
        let gate = StatsGate::new(Duration::from_secs(300));
        gate.record("a", (5, 0), 42u64);
        assert!(!gate.needs_refresh("a", (5, 0)));
        assert_eq!(gate.cached("a"), Some(42));
    }

    #[test]
    fn moved_counts_reopen_the_gate() {
        let gate = StatsGate::new(Duration::from_secs(300));
        gate.record("a", (5, 0), 42u64);
        assert!(gate.needs_refresh("a", (6, 0)));
        // A blob chunk landing without a document change also counts.
        assert!(gate.needs_refresh("a", (5, 1)));
    }

    #[test]
    fn periodic_refresh_falls_due() {
        // Zero interval: every check is past due, so nothing is ever gated.
        let gate = StatsGate::new(Duration::from_secs(0));
        gate.record("a", (5, 0), 42u64);
        assert!(gate.needs_refresh("a", (5, 0)));
    }

    #[test]
    fn retain_drops_vanished_collections() {
        let gate = StatsGate::new(Duration::from_secs(300));
        gate.record("gone", (1, 0), 1u64);
        gate.record("here", (1, 0), 2u64);

        let live: HashSet<String> = ["here".to_string()].into_iter().collect();
        gate.retain(&live);

        assert_eq!(gate.cached("gone"), None);
        assert_eq!(gate.cached("here"), Some(2));
    }
}
