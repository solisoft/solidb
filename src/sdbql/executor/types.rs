use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::super::ast::BinaryOperator;
use crate::storage::StorageEngine;
use crate::sync::log::SyncLog;

/// Execution context holding variable bindings
pub type Context = HashMap<String, Value>;

/// Statistics about mutation operations performed during query execution
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MutationStats {
    pub documents_inserted: usize,
    pub documents_updated: usize,
    pub documents_removed: usize,
}

impl MutationStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn total(&self) -> usize {
        self.documents_inserted + self.documents_updated + self.documents_removed
    }

    pub fn has_mutations(&self) -> bool {
        self.total() > 0
    }
}

/// Result of query execution including results and mutation statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExecutionResult {
    pub results: Vec<Value>,
    pub mutations: MutationStats,
}

/// Bind variables for parameterized queries (prevents SDBQL injection)
pub type BindVars = HashMap<String, Value>;

/// Query execution plan with timing information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryExplain {
    /// Collections accessed by the query
    pub collections: Vec<CollectionAccess>,
    /// LET clause bindings
    pub let_bindings: Vec<LetBinding>,
    /// Filter conditions analyzed
    pub filters: Vec<FilterInfo>,
    /// Sort information
    pub sort: Option<SortInfo>,
    /// Limit information
    pub limit: Option<LimitInfo>,
    /// Execution timing for each step (in microseconds)
    pub timing: ExecutionTiming,
    /// Total documents scanned
    pub documents_scanned: usize,
    /// Total documents returned
    pub documents_returned: usize,
    /// Warnings or suggestions
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionAccess {
    pub name: String,
    pub variable: String,
    pub access_type: String, // "full_scan" or "index_lookup"
    pub index_used: Option<String>,
    pub index_type: Option<String>,
    pub documents_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LetBinding {
    pub variable: String,
    pub is_subquery: bool,
    pub time_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterInfo {
    pub expression: String,
    pub index_candidate: Option<String>,
    pub can_use_index: bool,
    pub documents_before: usize,
    pub documents_after: usize,
    pub time_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortInfo {
    pub field: String,
    pub direction: String,
    pub time_us: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LimitInfo {
    pub offset: usize,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTiming {
    pub total_us: u64,
    pub let_clauses_us: u64,
    pub collection_scan_us: u64,
    pub filter_us: u64,
    pub sort_us: u64,
    pub limit_us: u64,
    pub return_projection_us: u64,
}

pub struct QueryExecutor<'a> {
    pub(crate) storage: &'a StorageEngine,
    pub(crate) bind_vars: BindVars,
    pub(crate) database: Option<String>,
    pub(crate) replication: Option<&'a SyncLog>,
    // Flag to indicate if we should defer mutations for transactional execution

    // Shard coordinator for scatter-gather queries on sharded collections
    pub(crate) shard_coordinator: Option<std::sync::Arc<crate::sharding::ShardCoordinator>>,
}

/// Extracted filter condition for index optimization
#[derive(Debug)]
pub struct IndexableCondition {
    pub field: String,
    pub op: BinaryOperator,
    pub value: Value,
}
