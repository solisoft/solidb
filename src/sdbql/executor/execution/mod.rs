//! Query execution modules for SDBQL executor.
//!
//! This module organizes execution code into submodules:
//! - entry: Main execution entry points (execute, execute_with_stats)
//! - streaming: Bulk insert and mutation logging
//! - clauses: Body clause processing (FOR, FILTER, JOIN, etc.)
//! - collect: COLLECT grouping and streaming aggregation
//! - subquery: Correlated subquery execution
//! - mutations: per-row INSERT / UPDATE / REPLACE / REMOVE with OPTIONS, OLD / NEW
//! - array_ops: array comparison operators and inline array expressions

mod array_ops;
mod clauses;
mod collect;
mod entry;
mod graph;
mod graph_rag;
mod mutations;
mod paths;
mod streaming;
mod subquery;

// All functionality is provided via impl blocks on QueryExecutor
// in the submodules, so no re-exports needed.
