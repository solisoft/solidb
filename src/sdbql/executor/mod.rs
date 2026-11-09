//! SDBQL Query Executor
//!
//! This module provides the query execution engine for SDBQL.

use std::collections::HashMap;

use crate::error::{DbError, DbResult};
use crate::sharding::ShardCoordinator;
use crate::storage::StorageEngine;
use crate::sync::log::SyncLog;

mod aggregation;
pub mod builtins;
mod catalog;
mod const_fold;
mod data_source;
mod datum;
mod evaluate;
mod exact_sum;
mod execution;
mod explain;
mod expression;
mod geo_opt;
mod helpers;
mod index_opt;
mod materialized_views;
pub mod phonetic;
mod projection;
mod subquery_memo;
pub mod types;
pub mod utils;
mod window;

pub(crate) use execution::update_guard;
pub use execution::SmallRaw;
pub use helpers::{
    compare_key_rows, compare_values, evaluate_binary_op, evaluate_unary_op, get_field_ref,
    get_field_value, hash_value, to_bool, values_equal, ValueSet,
};
pub use types::*;
pub use utils::*;
pub use window::{contains_window_functions, extract_window_functions, generate_window_key};

/// Default ceiling on intermediate rows a single query may materialise.
///
/// Nested `FOR` clauses build the cartesian product into a `Vec<Context>`
/// before any `LIMIT` applies. Each source is individually capped (`RANGE` at
/// 10M rows), but the *product* was not: `FOR x IN 1..10000000 FOR y IN
/// 1..10000000 RETURN 1` grows past any available memory and the OOM killer
/// takes the whole process, not just the request.
///
/// Override with `SOLIDB_MAX_INTERMEDIATE_ROWS`.
pub const DEFAULT_MAX_INTERMEDIATE_ROWS: usize = 5_000_000;

fn default_max_intermediate_rows() -> usize {
    static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("SOLIDB_MAX_INTERMEDIATE_ROWS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_MAX_INTERMEDIATE_ROWS)
    })
}

/// Query executor for SDBQL
pub struct QueryExecutor<'a> {
    pub(super) storage: &'a StorageEngine,
    pub(super) bind_vars: BindVars,
    pub(super) database: Option<String>,
    pub(super) replication: Option<&'a SyncLog>,
    pub(super) shard_coordinator: Option<std::sync::Arc<ShardCoordinator>>,
    pub(super) principal: Option<QueryPrincipal>,
    /// When set, the query's INSERT / UPDATE / REMOVE / UPSERT are staged on
    /// this transaction instead of being applied (they land at commit).
    pub(super) tx_writer: Option<std::sync::Arc<TxWriter>>,
    /// Wall-clock point after which the query gives up.
    ///
    /// The HTTP and driver layers wrap execution in `tokio::time::timeout`,
    /// but that only decides when the *client* gets a 408: the work runs
    /// inside `spawn_blocking`, which cannot be cancelled, so an expensive
    /// query kept burning a blocking-pool thread and heap until it finished.
    /// Enough of them (512 by default) and no further query — HTTP or driver
    /// — can be served at all. The executor has to check a deadline itself.
    pub(super) deadline: Option<std::time::Instant>,
    /// Ceiling on rows held in the intermediate pipeline.
    pub(super) max_intermediate_rows: usize,
    /// Whether a read-only query may reuse work inside itself: a subquery's
    /// rows while the bindings it can read are unchanged, and a collection
    /// read once and hashed for a correlated FILTER no index serves. On by
    /// default; tests turn it off to compare results against plain per-row
    /// execution.
    pub(super) reuse_within_query: bool,
    /// Whether a full-scan `COLLECT` folds each document into its group as
    /// it is read (`execution::scan_collect`) instead of materialising the
    /// scan first. On by default; tests turn it off to compare the two.
    pub(super) stream_collect: bool,
    /// How a streamed `COLLECT` reads the collection: typed reads, threads.
    pub(in crate::sdbql::executor) scan: execution::ScanSettings,
    /// Per-executor memo tables (row-policy gates, IN sets, window keys...).
    caches: ExecCaches,
}

/// A transaction a query stages its writes on.
pub struct TxWriter {
    pub tx: std::sync::Arc<std::sync::RwLock<crate::transaction::Transaction>>,
    pub wal: std::sync::Arc<crate::transaction::wal::WalWriter>,
    pub locks: std::sync::Arc<crate::transaction::lock_manager::LockManager>,
}

/// A recorded scan projection: the `FOR` clause's collection name, and the
/// fields its scan decodes.
type ScanProjection = (String, std::sync::Arc<[String]>);

/// Memo tables that live as long as one executor (one query).
///
/// Mutex rather than RefCell so the executor stays `Sync`; every lock is
/// uncontended in practice (a query runs on one thread).
#[derive(Default)]
struct ExecCaches {
    /// Parsed row-policy gate per collection binding name (audit P10).
    /// `None` = no policy binds this principal on that collection.
    row_policy_gates:
        parking_lot::Mutex<HashMap<String, Option<std::sync::Arc<data_source::RowPolicyGate>>>>,
    /// Hash sets for `x IN @bindVar`, keyed by bind variable name (audit P9).
    in_sets: parking_lot::Mutex<HashMap<String, std::sync::Arc<ValueSet>>>,
    /// Window-function context keys, keyed by the address of the
    /// `WindowFunctionCall` node; the node is kept alongside and compared on
    /// every hit, so a different AST reusing the address cannot alias.
    window_keys:
        parking_lot::Mutex<HashMap<usize, (crate::sdbql::ast::Expression, std::sync::Arc<str>)>>,
    /// Results of whole-collection graph analytics (PAGERANK,
    /// DEGREE_CENTRALITY) by function and arguments: they do not depend on
    /// the row, and ran once per row inside a FOR (audit S2).
    graph_results: parking_lot::Mutex<HashMap<String, serde_json::Value>>,
    /// Last BM25 query string and its tokens: BM25 runs once per row with
    /// the same query.
    bm25_query: parking_lot::Mutex<Option<(String, std::sync::Arc<Vec<String>>)>>,
    /// Fields a plain collection scan needs to decode, keyed by the address
    /// of its `FOR` clause (see [`projection`]). The collection name is kept
    /// and compared on lookup, so another AST reusing the address cannot
    /// pick up the wrong projection.
    scan_projections: parking_lot::Mutex<HashMap<usize, ScanProjection>>,
    /// The last result of each subquery node and the bindings it was
    /// computed under, keyed by the node's address (see [`subquery_memo`]).
    subqueries: parking_lot::Mutex<HashMap<usize, subquery_memo::SubqueryMemo>>,
    /// Whether the query may reuse work inside itself (memoised subqueries,
    /// the hash probe):
    /// set by `execute_with_stats` and `explain` for a query that writes
    /// nothing; off by default, so anything that bypasses them runs per row.
    reuse: std::sync::atomic::AtomicBool,
    /// Subqueries actually executed (memo hits excluded).
    subquery_runs: std::sync::atomic::AtomicU64,
    /// Whole-collection reads from storage by a `FOR` (a hash-probe build
    /// included).
    collection_scans: std::sync::atomic::AtomicU64,
    /// Documents the hash probe re-checked against a row's FILTERs.
    probe_candidates: std::sync::atomic::AtomicU64,
    /// Collections read whole for the hash probe, their buckets, and the
    /// FOR clauses that scanned without it (see [`execution::hash_probe`]).
    hash_probe: parking_lot::Mutex<execution::hash_probe::ProbeCache>,
    /// Whether a collection name is a columnar collection, per name (an
    /// executor serves one database). Every FOR over a collection asked
    /// storage (a registry read, 0.52 µs); with the auto-index flag below,
    /// that was ~10% of an 11 µs correlated subquery run.
    columnar: parking_lot::Mutex<HashMap<String, bool>>,
    /// `Collection::auto_index_enabled` per collection: a storage read
    /// (0.56 µs) that every index-path FOR made, once per run of a
    /// correlated subquery.
    auto_index: parking_lot::Mutex<HashMap<String, bool>>,
}

impl ExecCaches {
    fn reuse_enabled(&self) -> bool {
        self.reuse.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn enable_reuse(&self, on: bool) {
        self.reuse.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    fn count_subquery_run(&self) {
        self.subquery_runs
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn count_collection_scan(&self) {
        self.collection_scans
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn count_probe_candidates(&self, n: u64) {
        self.probe_candidates
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
    }

    /// Forget everything memoised. Call between two top-level `execute`s on
    /// the same executor.
    fn clear(&self) {
        self.subqueries.lock().clear();
        self.enable_reuse(false);
        self.subquery_runs
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.collection_scans
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.probe_candidates
            .store(0, std::sync::atomic::Ordering::Relaxed);
        *self.hash_probe.lock() = Default::default();
        self.row_policy_gates.lock().clear();
        self.in_sets.lock().clear();
        self.window_keys.lock().clear();
        self.graph_results.lock().clear();
        *self.bm25_query.lock() = None;
        self.scan_projections.lock().clear();
        self.columnar.lock().clear();
        self.auto_index.lock().clear();
    }
}

impl<'a> QueryExecutor<'a> {
    /// Create a new executor with storage reference
    pub fn new(storage: &'a StorageEngine) -> Self {
        Self {
            storage,
            bind_vars: HashMap::new(),
            database: None,
            replication: None,
            shard_coordinator: None,
            tx_writer: None,
            principal: None,
            deadline: None,
            max_intermediate_rows: default_max_intermediate_rows(),
            reuse_within_query: true,
            stream_collect: true,
            scan: execution::ScanSettings::default(),
            caches: ExecCaches::default(),
        }
    }

    /// Create executor with bind variables for parameterized queries
    pub fn with_bind_vars(storage: &'a StorageEngine, bind_vars: BindVars) -> Self {
        Self {
            storage,
            bind_vars,
            database: None,
            replication: None,
            shard_coordinator: None,
            tx_writer: None,
            principal: None,
            deadline: None,
            max_intermediate_rows: default_max_intermediate_rows(),
            reuse_within_query: true,
            stream_collect: true,
            scan: execution::ScanSettings::default(),
            caches: ExecCaches::default(),
        }
    }

    /// Create executor with database context
    pub fn with_database(storage: &'a StorageEngine, database: String) -> Self {
        Self {
            storage,
            bind_vars: HashMap::new(),
            database: Some(database),
            replication: None,
            shard_coordinator: None,
            tx_writer: None,
            principal: None,
            deadline: None,
            max_intermediate_rows: default_max_intermediate_rows(),
            reuse_within_query: true,
            stream_collect: true,
            scan: execution::ScanSettings::default(),
            caches: ExecCaches::default(),
        }
    }

    /// Create executor with both database context and bind variables
    pub fn with_database_and_bind_vars(
        storage: &'a StorageEngine,
        database: String,
        bind_vars: BindVars,
    ) -> Self {
        Self {
            storage,
            bind_vars,
            database: Some(database),
            replication: None,
            shard_coordinator: None,
            tx_writer: None,
            principal: None,
            deadline: None,
            max_intermediate_rows: default_max_intermediate_rows(),
            reuse_within_query: true,
            stream_collect: true,
            scan: execution::ScanSettings::default(),
            caches: ExecCaches::default(),
        }
    }

    /// Set sync log for logging mutations
    pub fn with_replication(mut self, replication: &'a SyncLog) -> Self {
        self.replication = Some(replication);
        self
    }

    /// Set shard coordinator for scatter-gather queries on sharded collections
    /// Stage this query's writes on `tx` rather than applying them. Reads still
    /// see committed data only.
    pub fn with_transaction(mut self, tx: TxWriter) -> Self {
        self.tx_writer = Some(std::sync::Arc::new(tx));
        self
    }

    pub fn with_shard_coordinator(mut self, coordinator: std::sync::Arc<ShardCoordinator>) -> Self {
        self.shard_coordinator = Some(coordinator);
        self
    }

    pub fn with_principal(mut self, principal: QueryPrincipal) -> Self {
        self.principal = Some(principal);
        self
    }

    /// Give the query a wall-clock budget it enforces itself.
    ///
    /// Pass the same duration the caller's `tokio::time::timeout` uses, so the
    /// work actually stops when the client is told it timed out.
    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.deadline = Some(std::time::Instant::now() + timeout);
        self
    }

    /// Override the intermediate row ceiling for this executor.
    ///
    /// The process-wide default comes from `SOLIDB_MAX_INTERMEDIATE_ROWS`;
    /// this exists so a test can exercise the limit on a small collection.
    pub fn with_max_intermediate_rows(mut self, rows: usize) -> Self {
        self.max_intermediate_rows = rows.max(1);
        self
    }

    /// Allow or forbid reuse of work inside a read-only query (see
    /// `reuse_within_query`). Results are the same either way; only the
    /// amount of work differs.
    pub fn with_reuse_within_query(mut self, on: bool) -> Self {
        self.reuse_within_query = on;
        self
    }

    /// Allow or forbid folding a full-scan `COLLECT` into the scan (see
    /// `stream_collect`). Results are the same either way; only the memory
    /// held and the time taken differ.
    pub fn with_streaming_collect(mut self, on: bool) -> Self {
        self.stream_collect = on;
        self
    }

    /// Allow or forbid the typed reads of a streamed `COLLECT` (group keys,
    /// aggregate arguments and FILTER operands read from the stored bytes
    /// instead of a decoded document). Results are the same either way; for
    /// tests and benchmarks that compare the two.
    pub fn with_typed_collect(mut self, on: bool) -> Self {
        self.scan.typed = on;
        self
    }

    /// The most threads a streamed `COLLECT` may scan with (1: one), instead
    /// of `SOLIDB_SCAN_THREADS` or the CPUs available; and the fewest
    /// documents worth a thread of their own (default 32,768). For tests and
    /// benchmarks; the process-wide cap on scan threads still applies.
    pub fn with_scan_threads(mut self, threads: usize, min_docs_per_thread: usize) -> Self {
        self.scan.threads = Some(threads.max(1));
        self.scan.min_docs_per_thread = min_docs_per_thread.max(1);
        self
    }

    /// Subqueries executed (memo hits excluded) in the last query this
    /// executor ran; for tests and benchmarks.
    pub fn subquery_runs(&self) -> u64 {
        self.caches
            .subquery_runs
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whole-collection reads from storage by a `FOR` in the last query this
    /// executor ran; for tests and benchmarks.
    pub fn collection_scans(&self) -> u64 {
        self.caches
            .collection_scans
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Documents the hash probe re-checked against a row's FILTERs in the
    /// last query this executor ran; for tests and benchmarks.
    pub fn probe_candidates(&self) -> u64 {
        self.caches
            .probe_candidates
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Whether `name` is a columnar collection of `database`, read from
    /// storage once per query.
    pub(super) fn is_columnar(&self, database: &crate::storage::Database, name: &str) -> bool {
        if let Some(&known) = self.caches.columnar.lock().get(name) {
            return known;
        }
        let columnar = database.is_columnar_collection(name);
        self.caches
            .columnar
            .lock()
            .insert(name.to_string(), columnar);
        columnar
    }

    /// [`crate::storage::Collection::auto_index_enabled`], read from storage
    /// once per query.
    pub(super) fn auto_index_enabled(&self, collection: &crate::storage::Collection) -> bool {
        if let Some(&known) = self.caches.auto_index.lock().get(&collection.name) {
            return known;
        }
        let enabled = collection.auto_index_enabled();
        self.caches
            .auto_index
            .lock()
            .insert(collection.name.clone(), enabled);
        enabled
    }

    /// Drop every per-query memo (row-policy gates, IN sets, window keys,
    /// graph analytics results). Call at the start of each top-level
    /// `execute` so a reused executor never serves results memoised for a
    /// previous query, or a policy that changed in between.
    pub fn reset_query_caches(&self) {
        self.caches.clear();
    }

    /// The row ceiling this executor enforces.
    pub fn max_intermediate_rows(&self) -> usize {
        self.max_intermediate_rows
    }

    /// The scan limit to hand a storage read whose result will be held in
    /// memory: one past the ceiling, so the budget check that follows fails
    /// exactly as it would have on the full scan without first holding it.
    pub fn scan_cap(&self) -> Option<usize> {
        Some(self.max_intermediate_rows.saturating_add(1))
    }

    /// Read a whole collection into memory, bounded by the row ceiling.
    ///
    /// Every whole-collection read on the query path — JOIN right sides,
    /// edge sets for path search and graph analytics — used `scan(None)` and
    /// only met the ceiling, if at all, once the rows built from it were
    /// counted. That is the wrong side of the allocation.
    pub fn scan_bounded(
        &self,
        collection: &crate::storage::collection::Collection,
    ) -> DbResult<Vec<serde_json::Value>> {
        let docs = collection.scan_values(self.scan_cap());
        self.check_budget(docs.len())?;
        Ok(docs)
    }

    /// Stop the query if it has run past its deadline or grown past its row
    /// budget.
    ///
    /// Called at each pipeline stage boundary — the points where the row set
    /// is replaced — which is where both a runaway cartesian product and a
    /// long scan become visible.
    pub fn check_budget(&self, rows: usize) -> DbResult<()> {
        if rows > self.max_intermediate_rows {
            return Err(DbError::ExecutionError(format!(
                "Query exceeded the intermediate row limit ({} > {}). Add a \
                 FILTER or LIMIT, or raise SOLIDB_MAX_INTERMEDIATE_ROWS.",
                rows, self.max_intermediate_rows
            )));
        }
        if let Some(deadline) = self.deadline {
            if std::time::Instant::now() >= deadline {
                return Err(DbError::ExecutionError(
                    "Query exceeded its time limit".to_string(),
                ));
            }
        }
        Ok(())
    }
}
