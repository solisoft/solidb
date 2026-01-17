//! Query execution modules for SDBQL executor.
//!
//! This module organizes execution code into submodules:
//! - entry: Main execution entry points (execute, execute_with_stats)
//! - streaming: Bulk insert and mutation logging
//! - clauses: Body clause processing (FOR, FILTER, JOIN, etc.)
//! - subquery: Correlated subquery execution

mod clauses;
mod entry;
mod streaming;
mod subquery;

// All functionality is provided via impl blocks on QueryExecutor
// in the submodules, so no re-exports needed.
