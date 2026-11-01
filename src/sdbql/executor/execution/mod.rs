//! Query execution modules for SDBQL executor.
//!
//! This module organizes execution code into submodules:
//! - entry: Main execution entry points (execute, execute_with_stats)
//! - streaming: Bulk insert and mutation logging
//! - clauses: Body clause processing (FOR, FILTER, JOIN, etc.)
//! - collect: COLLECT grouping and streaming aggregation
//! - index_count: `FILTER v.f == x COLLECT WITH COUNT` read from an index
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
mod mutations;
mod paths;
mod raw_scan;
mod return_moves;
mod streaming;
mod subquery;

// All functionality is provided via impl blocks on QueryExecutor
// in the submodules; the one re-export is a constant the index planner's
// row-building loop shares.
pub(super) use clauses::BUDGET_CHECK_INTERVAL;
