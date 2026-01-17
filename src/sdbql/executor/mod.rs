//! SDBQL Query Executor
//!
//! This module provides the query execution engine for SDBQL.

use std::collections::HashMap;

use super::ast::*;
use crate::sharding::ShardCoordinator;
use crate::storage::StorageEngine;
use crate::sync::log::SyncLog;

mod aggregation;
mod builtins;
mod evaluate;
mod data_source;
mod execution;
mod explain;
mod expression;
pub mod functions;
mod helpers;
mod index_opt;
mod materialized_views;
pub mod types;
pub mod utils;
mod window;

pub use helpers::{
    compare_values, evaluate_binary_op, evaluate_unary_op, get_field_value, to_bool, values_equal,
};
pub use types::*;
pub use utils::*;
pub use window::{contains_window_functions, extract_window_functions, generate_window_key};

/// Query executor for SDBQL
pub struct QueryExecutor<'a> {
    pub(super) storage: &'a StorageEngine,
    pub(super) bind_vars: BindVars,
    pub(super) database: Option<String>,
    pub(super) replication: Option<&'a SyncLog>,
    pub(super) shard_coordinator: Option<std::sync::Arc<ShardCoordinator>>,
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
}
