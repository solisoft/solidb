//! Query execution modules for SDBQL executor.
//!
//! This module organizes execution code into submodules:
//! - entry: Main execution entry points (execute, execute_with_stats)
//! - streaming: Bulk insert and mutation logging
//! - clauses: Body clause processing (FOR, FILTER, JOIN, etc.)
//! - collect: COLLECT grouping and streaming aggregation
//! - scan_collect: a full-scan COLLECT folded while the collection is read
//! - typed_scan: its keys, arguments and FILTERs read from the stored bytes
//! - scan_threads: how many threads such a scan may use
//! - index_count: `FILTER v.f == x COLLECT WITH COUNT` read from an index
//! - index_sort: `FILTER v.a == x SORT v.b LIMIT n` (or a range on `v.b`) read in
//!   SORT order from an index, stopping at the LIMIT
//! - hash_probe: a correlated FILTER no index serves, probed in a per-query hash
//! - subquery: Correlated subquery execution
//! - mutations: per-row INSERT / UPDATE / REPLACE / REMOVE with OPTIONS, OLD / NEW
//! - array_ops: array comparison operators and inline array expressions

mod array_ops;
mod clauses;
mod collect;
mod entry;
mod graph;
mod graph_rag;
pub(super) mod hash_probe;
mod index_count;
mod index_sort;
mod mutations;
mod paths;
mod raw_scan;
mod return_moves;
mod scan_collect;
mod scan_threads;
mod streaming;
mod subquery;
mod typed_scan;
mod update_guard;

// All functionality is provided via impl blocks on QueryExecutor
// in the submodules; the one re-export is a constant the index planner's
// row-building loop shares.
pub(super) use clauses::BUDGET_CHECK_INTERVAL;
pub(super) use entry::has_window_functions;
pub use raw_scan::SmallRaw;
pub(super) use scan_collect::has_collect;
pub(super) use scan_threads::ScanSettings;
pub(crate) use update_guard::update_guard;
