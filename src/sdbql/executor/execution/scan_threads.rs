//! How many threads a streamed `COLLECT` may scan its collection with.
//!
//! A full-scan aggregation is split into key ranges read on several threads
//! (`scan_collect`) when the engine can give them one consistent view (a
//! snapshot) and the collection is large enough to be worth it. Two limits
//! apply:
//!
//! - per scan, `SOLIDB_SCAN_THREADS` threads (the query's own included);
//!   unset, the CPUs this process may use: the cgroup v2 quota in
//!   `/sys/fs/cgroup/cpu.max` when there is one, else the scheduler's view
//!   (`available_parallelism`, which also honours CPU affinity). `1` turns
//!   parallel scans off;
//! - across the process, scans together never run more helper threads (the
//!   threads beyond each query's own) than that number less one: a second
//!   large aggregation started while one holds the CPUs gets what is left —
//!   possibly none, and then it reads sequentially — instead of doubling the
//!   load. Other queries are never made to wait for a scan thread.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

/// Settings of one executor, for tests and benchmarks (see
/// `QueryExecutor::with_typed_collect` and `with_scan_threads`).
#[derive(Debug, Clone, Copy)]
pub(in crate::sdbql::executor) struct ScanSettings {
    /// Whether keys, arguments and FILTERs may be read from stored bytes.
    pub(in crate::sdbql::executor) typed: bool,
    /// Threads per scan; `None` for the process's setting.
    pub(in crate::sdbql::executor) threads: Option<usize>,
    /// Fewest documents per thread: a collection of fewer than twice this
    /// is read on one.
    pub(in crate::sdbql::executor) min_docs_per_thread: usize,
}

impl Default for ScanSettings {
    fn default() -> Self {
        Self {
            typed: true,
            threads: None,
            min_docs_per_thread: DEFAULT_MIN_DOCS_PER_THREAD,
        }
    }
}

/// Fewer documents than this per thread are not worth a thread: reading
/// 32k documents takes a thread about ten milliseconds, against well under
/// one to start it and merge what it found — and a smaller collection is
/// answered in a few milliseconds anyway.
const DEFAULT_MIN_DOCS_PER_THREAD: usize = 32_768;

impl ScanSettings {
    /// Threads a scan of `docs` documents would use, before the process-wide
    /// budget: 1 when it is not worth splitting.
    pub(in crate::sdbql::executor) fn wanted(&self, docs: usize) -> usize {
        let threads = self.threads.unwrap_or_else(configured_threads);
        threads.min(docs / self.min_docs_per_thread).max(1)
    }
}

/// `SOLIDB_SCAN_THREADS`, else the CPUs available; at least 1.
pub(crate) fn configured_threads() -> usize {
    static THREADS: OnceLock<usize> = OnceLock::new();
    *THREADS.get_or_init(|| {
        std::env::var("SOLIDB_SCAN_THREADS")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|&n| n > 0)
            .unwrap_or_else(available_cpus)
    })
}

/// CPUs this process may use: the cgroup v2 quota rounded up, capped by
/// what the scheduler offers.
fn available_cpus() -> usize {
    let scheduler = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    match std::fs::read_to_string("/sys/fs/cgroup/cpu.max")
        .ok()
        .and_then(|s| cgroup_quota(&s))
    {
        Some(quota) => quota.min(scheduler).max(1),
        None => scheduler,
    }
}

/// The CPUs a cgroup v2 `cpu.max` (`"<quota> <period>"` or `"max <period>"`)
/// allows, rounded up; `None` without a limit.
fn cgroup_quota(cpu_max: &str) -> Option<usize> {
    let mut parts = cpu_max.split_whitespace();
    let quota: u64 = parts.next()?.parse().ok()?;
    let period: u64 = parts.next()?.parse().ok().filter(|&p| p > 0)?;
    Some(quota.div_ceil(period).max(1) as usize)
}

/// Helper threads running for scans, process-wide.
static HELPERS: AtomicUsize = AtomicUsize::new(0);

/// Helper threads a scan holds; returned when dropped.
pub(in crate::sdbql::executor) struct Helpers(usize);

impl Helpers {
    /// Up to `wanted` helpers, as many as the process-wide budget has left
    /// (none at all is an answer: the scan then runs on its own thread).
    pub(in crate::sdbql::executor) fn take(wanted: usize) -> Helpers {
        let budget = configured_threads().saturating_sub(1);
        let mut current = HELPERS.load(Ordering::Relaxed);
        loop {
            let granted = wanted.min(budget.saturating_sub(current));
            if granted == 0 {
                return Helpers(0);
            }
            match HELPERS.compare_exchange_weak(
                current,
                current + granted,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Helpers(granted),
                Err(actual) => current = actual,
            }
        }
    }

    pub(in crate::sdbql::executor) fn count(&self) -> usize {
        self.0
    }
}

impl Drop for Helpers {
    fn drop(&mut self) {
        if self.0 > 0 {
            HELPERS.fetch_sub(self.0, Ordering::AcqRel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgroup_quotas() {
        assert_eq!(cgroup_quota("max 100000"), None);
        assert_eq!(cgroup_quota("400000 100000"), Some(4));
        assert_eq!(cgroup_quota("250000 100000"), Some(3));
        assert_eq!(cgroup_quota("50000 100000"), Some(1));
        assert_eq!(cgroup_quota("garbage"), None);
        assert_eq!(cgroup_quota("100 0"), None);
    }

    #[test]
    fn small_collections_stay_on_one_thread() {
        let s = ScanSettings {
            typed: true,
            threads: Some(8),
            min_docs_per_thread: 1000,
        };
        assert_eq!(s.wanted(0), 1);
        assert_eq!(s.wanted(1999), 1);
        assert_eq!(s.wanted(2000), 2);
        assert_eq!(s.wanted(1_000_000), 8);
        let one = ScanSettings {
            threads: Some(1),
            ..s
        };
        assert_eq!(one.wanted(1_000_000), 1);
    }
}
