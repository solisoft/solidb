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
mod data_source;
mod evaluate;
mod execution;
mod explain;
mod expression;
mod helpers;
mod index_opt;
mod materialized_views;
pub mod phonetic;
pub mod types;
pub mod utils;
mod window;

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
            principal: None,
            deadline: None,
            max_intermediate_rows: default_max_intermediate_rows(),
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
            principal: None,
            deadline: None,
            max_intermediate_rows: default_max_intermediate_rows(),
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
            principal: None,
            deadline: None,
            max_intermediate_rows: default_max_intermediate_rows(),
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
            principal: None,
            deadline: None,
            max_intermediate_rows: default_max_intermediate_rows(),
        }
    }

    /// Set sync log for logging mutations
    pub fn with_replication(mut self, replication: &'a SyncLog) -> Self {
        self.replication = Some(replication);
        self
    }

    /// Set shard coordinator for scatter-gather queries on sharded collections
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
