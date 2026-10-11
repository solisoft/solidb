//! Client-query latency for `/metrics`.
//!
//! Two fixed series, no per-query labels: `read` and `write` executions on
//! `POST /_api/database/{db}/cursor` and on the driver's query command. A
//! result-cache hit executes nothing and is not in the histogram — a 0 s
//! sample per hit would drag every quantile and mean that does not filter on
//! `kind` towards zero. Hits are `solidb_query_cache_hits_total`. Lua
//! scripts, triggers and the queue worker execute through other doors and are
//! not in these series: mixing them in would put background refresh into the
//! client p99.
//!
//! Every execution is measured by an [`Observation`] around the executor
//! call, on the thread that runs it, so failures, panics and runs past the
//! client's timeout are counted too (the last when the executor stops, with
//! their real duration). A query at or above [`SLOW_QUERY_THRESHOLD_MS`] also
//! increments `solidb_slow_queries_total`. The `_slow_queries` collection
//! still receives only finished HTTP queries.
//!
//! Counters live on the thread that ran the query ([`PerThread`]). A sample
//! increments one bucket; the cumulative `le` values are computed while
//! rendering.

use crate::observability::per_thread::{PerThread, Registration, Slot};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Queries at or above this are slow. The HTTP slow-query log uses the same
/// constant, so the counter and the collection agree on the threshold.
pub const SLOW_QUERY_THRESHOLD_MS: f64 = 100.0;

/// Upper bounds of the histogram buckets, in nanoseconds, with the `le` label
/// Prometheus prints. `+Inf` is rendered separately. Bounds are inclusive.
const BUCKETS: [(u64, &str); 9] = [
    (1_000_000, "0.001"),
    (5_000_000, "0.005"),
    (10_000_000, "0.01"),
    (25_000_000, "0.025"),
    (50_000_000, "0.05"),
    (100_000_000, "0.1"),
    (250_000_000, "0.25"),
    (1_000_000_000, "1"),
    (5_000_000_000, "5"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueryKind {
    Read,
    Write,
}

impl QueryKind {
    pub fn of(mutates: bool) -> Self {
        if mutates {
            Self::Write
        } else {
            Self::Read
        }
    }
}

/// Counts one execution: the only way to record one. Start it immediately
/// before the executor call and [`finish`](Self::finish) it after, or bind it
/// to a name (`let _observed`; `let _ = …` drops it at once) and let it drop.
///
/// Records on drop too, so a `?` or a panic inside the measured region still
/// counts. Do not hold it across an `.await` that is not part of the query.
#[must_use = "the observation is recorded when this value drops"]
pub struct Observation {
    kind: QueryKind,
    start: Instant,
    recorded: bool,
}

impl Observation {
    pub fn start(kind: QueryKind) -> Self {
        Self {
            kind,
            start: Instant::now(),
            recorded: false,
        }
    }

    /// Record now and return the measured duration.
    pub fn finish(mut self) -> Duration {
        let elapsed = self.start.elapsed();
        record(self.kind, elapsed);
        self.recorded = true;
        elapsed
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        if !self.recorded {
            record(self.kind, self.start.elapsed());
        }
    }
}

struct Histogram {
    /// Non-cumulative. Index `i` counts samples that fall in bucket `i` and
    /// not in a smaller one. Samples above the last bound stay out of every
    /// cell and are counted only in `count` (`+Inf` at render time).
    buckets: [AtomicU64; BUCKETS.len()],
    sum_nanos: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    const fn new() -> Self {
        Self {
            buckets: [const { AtomicU64::new(0) }; BUCKETS.len()],
            sum_nanos: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    /// Two or three relaxed adds on a line this thread owns. No shared line.
    fn observe(&self, nanos: u64) {
        self.count.fetch_add(1, Ordering::Relaxed);
        if nanos != 0 {
            self.sum_nanos.fetch_add(nanos, Ordering::Relaxed);
        }
        if let Some(i) = bucket_index(nanos) {
            self.buckets[i].fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// One writer's counters. Scrape reads them; only this thread writes them,
/// so the line stays in its cache.
struct ThreadSeries {
    read: Histogram,
    write: Histogram,
    slow: AtomicU64,
}

impl ThreadSeries {
    const fn new() -> Self {
        Self {
            read: Histogram::new(),
            write: Histogram::new(),
            slow: AtomicU64::new(0),
        }
    }

    fn histogram(&self, kind: QueryKind) -> &Histogram {
        match kind {
            QueryKind::Read => &self.read,
            QueryKind::Write => &self.write,
        }
    }
}

fn add(into: &AtomicU64, from: &AtomicU64) {
    into.fetch_add(from.load(Ordering::Relaxed), Ordering::Relaxed);
}

impl Histogram {
    fn retire_into(&self, retired: &Self) {
        for (from, into) in self.buckets.iter().zip(retired.buckets.iter()) {
            add(into, from);
        }
        add(&retired.sum_nanos, &self.sum_nanos);
        add(&retired.count, &self.count);
    }
}

impl Slot for ThreadSeries {
    fn retire_into(&self, retired: &Self) {
        self.read.retire_into(&retired.read);
        self.write.retire_into(&retired.write);
        add(&retired.slow, &self.slow);
    }
}

static SERIES_REGISTRY: PerThread<ThreadSeries> = PerThread::new(ThreadSeries::new());

thread_local! {
    static SERIES: Registration<ThreadSeries> = SERIES_REGISTRY.register(ThreadSeries::new());
}

fn bucket_index(nanos: u64) -> Option<usize> {
    BUCKETS.iter().position(|(bound, _)| nanos <= *bound)
}

pub fn is_slow(elapsed: Duration) -> bool {
    elapsed >= Duration::from_millis(SLOW_QUERY_THRESHOLD_MS as u64)
}

/// Record one executed query. Not called on a cache hit: that path must not
/// read the clock or touch a counter beyond the hit tally in the cache.
#[inline]
fn record(kind: QueryKind, elapsed: Duration) {
    let nanos = u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX);
    // `try_with`: an observation dropped while the thread's locals are being
    // destroyed is lost rather than a panic.
    let _ = SERIES.try_with(|series| {
        series.histogram(kind).observe(nanos);
        if is_slow(elapsed) {
            series.slow.fetch_add(1, Ordering::Relaxed);
        }
    });
}

#[derive(Clone, Copy, Default)]
struct Merged {
    buckets: [u64; BUCKETS.len()],
    sum_nanos: u64,
    count: u64,
}

impl Merged {
    fn add(&mut self, hist: &Histogram) {
        for (into, bucket) in self.buckets.iter_mut().zip(hist.buckets.iter()) {
            *into += bucket.load(Ordering::Relaxed);
        }
        self.sum_nanos += hist.sum_nanos.load(Ordering::Relaxed);
        self.count += hist.count.load(Ordering::Relaxed);
    }
}

fn merged() -> ([Merged; 2], u64) {
    let mut out = [Merged::default(), Merged::default()];
    let mut slow = 0u64;
    SERIES_REGISTRY.for_each(|series| {
        out[0].add(&series.read);
        out[1].add(&series.write);
        slow += series.slow.load(Ordering::Relaxed);
    });
    (out, slow)
}

fn write_histogram(out: &mut String, kind: &str, hist: &Merged) {
    // Stored buckets are exclusive. Prometheus `le` is cumulative.
    let mut cumulative = 0u64;
    for (i, (_, label)) in BUCKETS.iter().enumerate() {
        cumulative += hist.buckets[i];
        out.push_str(&format!(
            "solidb_query_duration_seconds_bucket{{kind=\"{kind}\",le=\"{label}\"}} {cumulative}\n"
        ));
    }
    out.push_str(&format!(
        "solidb_query_duration_seconds_bucket{{kind=\"{kind}\",le=\"+Inf\"}} {}\n",
        hist.count
    ));
    let sum = hist.sum_nanos as f64 / 1e9;
    out.push_str(&format!(
        "solidb_query_duration_seconds_sum{{kind=\"{kind}\"}} {sum:.6}\n"
    ));
    out.push_str(&format!(
        "solidb_query_duration_seconds_count{{kind=\"{kind}\"}} {}\n",
        hist.count
    ));
}

/// Append the query series. Series are emitted at zero so a fresh process
/// already has them.
pub fn write_prometheus(out: &mut String) {
    out.push_str(
        "# HELP solidb_query_duration_seconds SDBQL execution time in seconds. \
         The label kind is read or write. Result-cache hits execute nothing and \
         are counted in solidb_query_cache_hits_total. Covers HTTP /cursor and the driver.\n",
    );
    out.push_str("# TYPE solidb_query_duration_seconds histogram\n");
    let (hits, misses, entries) = crate::storage::query_cache::metrics_snapshot();
    let (merged, slow) = merged();
    for (kind, hist) in [("read", &merged[0]), ("write", &merged[1])] {
        write_histogram(out, kind, hist);
    }
    out.push('\n');

    out.push_str(&format!(
        "# HELP solidb_slow_queries_total SDBQL executions at or above {SLOW_QUERY_THRESHOLD_MS}ms \
         on HTTP /cursor and the driver, including failures and timeouts\n"
    ));
    out.push_str("# TYPE solidb_slow_queries_total counter\n");
    out.push_str(&format!("solidb_slow_queries_total {slow}\n\n"));

    out.push_str(
        "# HELP solidb_query_cache_hits_total Result-cache lookups that returned a stored result\n",
    );
    out.push_str("# TYPE solidb_query_cache_hits_total counter\n");
    out.push_str(&format!("solidb_query_cache_hits_total {hits}\n\n"));
    out.push_str(
        "# HELP solidb_query_cache_misses_total Result-cache lookups that missed or found an expired entry\n",
    );
    out.push_str("# TYPE solidb_query_cache_misses_total counter\n");
    out.push_str(&format!("solidb_query_cache_misses_total {misses}\n\n"));
    out.push_str("# HELP solidb_query_cache_entries Result-cache entries currently held\n");
    out.push_str("# TYPE solidb_query_cache_entries gauge\n");
    out.push_str(&format!("solidb_query_cache_entries {entries}\n\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_increments_only_the_matching_bucket() {
        let hist = Histogram::new();
        hist.observe(3_000_000);
        hist.observe(100_000_000);
        hist.observe(6_000_000_000);

        assert_eq!(hist.buckets[0].load(Ordering::Relaxed), 0); // 1ms: nothing that small
        assert_eq!(hist.buckets[1].load(Ordering::Relaxed), 1); // 5ms: the 3ms sample
        assert_eq!(hist.buckets[2].load(Ordering::Relaxed), 0);
        assert_eq!(hist.buckets[5].load(Ordering::Relaxed), 1); // 100ms exactly
        assert_eq!(hist.buckets[8].load(Ordering::Relaxed), 0); // 6s is above 5s
        assert_eq!(hist.count.load(Ordering::Relaxed), 3);
        assert_eq!(
            hist.sum_nanos.load(Ordering::Relaxed),
            3_000_000 + 100_000_000 + 6_000_000_000
        );
    }

    #[test]
    fn rendered_buckets_are_cumulative() {
        let mut hist = Merged::default();
        hist.buckets[1] = 1;
        hist.buckets[5] = 1;
        hist.count = 3;
        hist.sum_nanos = 103_000_000;
        let mut out = String::new();
        write_histogram(&mut out, "read", &hist);
        assert!(out.contains("kind=\"read\",le=\"0.001\"} 0\n"));
        assert!(out.contains("kind=\"read\",le=\"0.005\"} 1\n"));
        assert!(out.contains("kind=\"read\",le=\"0.1\"} 2\n"));
        assert!(out.contains("kind=\"read\",le=\"5\"} 2\n"));
        assert!(out.contains("kind=\"read\",le=\"+Inf\"} 3\n"));
    }

    fn thread_counts() -> (u64, u64) {
        SERIES.with(|series| {
            (
                series.read.count.load(Ordering::Relaxed),
                series.write.count.load(Ordering::Relaxed),
            )
        })
    }

    #[test]
    fn observation_records_once_on_finish_or_drop() {
        let (reads, writes) = thread_counts();
        Observation::start(QueryKind::Read).finish();
        drop(Observation::start(QueryKind::of(true)));
        assert_eq!(thread_counts(), (reads + 1, writes + 1));
    }

    #[test]
    fn observation_records_a_panicking_query() {
        let (reads, _) = thread_counts();
        let outcome = std::panic::catch_unwind(|| {
            let _observed = Observation::start(QueryKind::Read);
            panic!("executor bug");
        });
        assert!(outcome.is_err());
        assert_eq!(thread_counts().0, reads + 1);
    }

    #[test]
    fn exited_threads_stay_in_the_totals() {
        let (before, _) = merged();
        std::thread::spawn(|| Observation::start(QueryKind::Read).finish())
            .join()
            .unwrap();
        let (after, _) = merged();
        assert!(after[0].count > before[0].count);
    }

    #[test]
    fn slow_boundary_is_one_hundred_milliseconds() {
        assert!(!is_slow(Duration::from_millis(99)));
        assert!(is_slow(Duration::from_millis(100)));
        assert!(is_slow(Duration::from_secs(30)));
    }

    #[test]
    fn render_names_every_series() {
        let mut out = String::new();
        write_prometheus(&mut out);
        assert!(out.contains("# TYPE solidb_query_duration_seconds histogram\n"));
        assert!(out.contains("solidb_query_duration_seconds_bucket{kind=\"read\",le=\"0.001\"}"));
        assert!(out.contains("solidb_query_duration_seconds_bucket{kind=\"write\",le=\"+Inf\"}"));
        assert!(!out.contains("kind=\"cached\""));
        assert!(
            out.contains("# HELP solidb_slow_queries_total SDBQL executions at or above 100ms ")
        );
        assert!(out.contains("solidb_query_duration_seconds_sum{kind=\"read\"}"));
        assert!(out.contains("solidb_query_duration_seconds_count{kind=\"read\"}"));
        assert!(out.contains("# TYPE solidb_slow_queries_total counter\n"));
        assert!(out.contains("# TYPE solidb_query_cache_hits_total counter\n"));
        assert!(out.contains("# TYPE solidb_query_cache_misses_total counter\n"));
        assert!(out.contains("# TYPE solidb_query_cache_entries gauge\n"));
    }
}
