use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

use super::ast::*;
use crate::sync::log::SyncLog;
use crate::sync::protocol::Operation;
use crate::sync::log::LogEntry;
use crate::error::{DbError, DbResult};
use crate::storage::{distance_meters, Collection, GeoPoint, StorageEngine};

/// Convert f64 to serde_json::Number, returning 0 for NaN/Infinity instead of panicking
fn number_from_f64(f: f64) -> serde_json::Number {
    serde_json::Number::from_f64(f).unwrap_or_else(|| serde_json::Number::from(0))
}

/// Parse a date value (timestamp or ISO string) into DateTime<Utc>
fn parse_datetime(value: &Value) -> DbResult<chrono::DateTime<Utc>> {
    use chrono::{DateTime, TimeZone};
    
    match value {
        Value::Number(n) => {
            let timestamp_ms = if let Some(i) = n.as_i64() {
                i
            } else if let Some(f) = n.as_f64() {
                f as i64
            } else {
                return Err(DbError::ExecutionError("Invalid timestamp".to_string()));
            };
            let secs = timestamp_ms / 1000;
            let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
            match Utc.timestamp_opt(secs, nanos) {
                chrono::LocalResult::Single(dt) => Ok(dt),
                _ => Err(DbError::ExecutionError(format!("Invalid timestamp: {}", timestamp_ms))),
            }
        }
        Value::String(s) => {
            DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| DbError::ExecutionError(format!("Invalid ISO 8601 date '{}': {}", s, e)))
        }
        _ => Err(DbError::ExecutionError("Date must be a timestamp or ISO 8601 string".to_string())),
    }
}

/// Execution context holding variable bindings
type Context = HashMap<String, Value>;

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
    storage: &'a StorageEngine,
    bind_vars: BindVars,
    database: Option<String>,
    replication: Option<&'a SyncLog>,
    // Flag to indicate if we should defer mutations for transactional execution

    // Shard coordinator for scatter-gather queries on sharded collections
    shard_coordinator: Option<std::sync::Arc<crate::sharding::ShardCoordinator>>,
}

/// Extracted filter condition for index optimization
#[derive(Debug)]
struct IndexableCondition {
    field: String,
    op: BinaryOperator,
    value: Value,
}

/// Format an Expression as a human-readable string
fn format_expression(expr: &Expression) -> String {
    match expr {
        Expression::Variable(name) => name.clone(),
        Expression::BindVariable(name) => format!("@{}", name),
        Expression::FieldAccess(base, field) => {
            format!("{}.{}", format_expression(base), field)
        }
        Expression::DynamicFieldAccess(base, field_expr) => {
            format!("{}[{}]", format_expression(base), format_expression(field_expr))
        }
        Expression::ArrayAccess(base, index) => {
            format!("{}[{}]", format_expression(base), format_expression(index))
        }
        Expression::Literal(value) => format!("{}", value),
        Expression::FunctionCall { name, args } => {
            let args_str = args
                .iter()
                .map(|a| format_expression(a))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({})", name, args_str)
        }
        _ => format!("{:?}", expr), // Fallback to debug for complex expressions
    }
}

impl<'a> QueryExecutor<'a> {
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
    pub fn with_shard_coordinator(mut self, coordinator: std::sync::Arc<crate::sharding::ShardCoordinator>) -> Self {
        self.shard_coordinator = Some(coordinator);
        self
    }

    /// Try to execute a streaming bulk INSERT optimization
    /// Returns Some((results, insert_count)) if the pattern matches, None otherwise
    /// Pattern: FOR i IN start..end INSERT {...} INTO collection [RETURN ...]
    fn try_streaming_bulk_insert(
        &self,
        query: &Query,
        initial_bindings: &Context,
    ) -> DbResult<Option<(Vec<Value>, usize)>> {
        // Check pattern: exactly 2 body clauses (FOR + INSERT), no sort/limit/filter
        if query.body_clauses.len() != 2
            || query.sort_clause.is_some()
            || query.limit_clause.is_some()
        {
            return Ok(None);
        }

        // First clause must be FOR with range expression
        let for_clause = match &query.body_clauses[0] {
            BodyClause::For(fc) => fc,
            _ => return Ok(None),
        };

        // Second clause must be INSERT
        let insert_clause = match &query.body_clauses[1] {
            BodyClause::Insert(ic) => ic,
            _ => return Ok(None),
        };

        // FOR must have a range expression
        let range_expr = match &for_clause.source_expression {
            Some(Expression::Range(start, end)) => (start, end),
            _ => return Ok(None),
        };

        // Evaluate range bounds
        let start_val = self.evaluate_expr_with_context(range_expr.0, initial_bindings)?;
        let end_val = self.evaluate_expr_with_context(range_expr.1, initial_bindings)?;

        let start = match &start_val {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            _ => None,
        };
        let end = match &end_val {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            _ => None,
        };

        let (start, end) = match (start, end) {
            (Some(s), Some(e)) => (s, e),
            _ => return Ok(None),
        };

        // Only use streaming for large ranges (>5000 items)
        const STREAMING_THRESHOLD: i64 = 5_000;
        const BATCH_SIZE: i64 = 5_000;

        let total_count = (end - start + 1).max(0);
        if total_count < STREAMING_THRESHOLD {
            return Ok(None); // Use normal path for small ranges
        }

        tracing::info!(
            "STREAMING INSERT: Processing {} documents in batches of {}",
            total_count,
            BATCH_SIZE
        );

        // Get collection once
        let collection = self.get_collection(&insert_clause.collection)?;
        
        // Disable streaming bulk insert for sharded collections (fall back to generic path for routing)
        if let Some(config) = collection.get_shard_config() {
            if config.num_shards > 0 {
                tracing::debug!("Streaming insert disabled for sharded collection: {}", insert_clause.collection);
                return Ok(None);
            }
        }
        
        let has_indexes = !collection.list_indexes().is_empty();

        let var_name = &for_clause.variable;
        let mut all_results: Vec<Value> = Vec::new();
        let mut current = start;
        let total_start = std::time::Instant::now();

        while current <= end {
            let batch_end = (current + BATCH_SIZE - 1).min(end);
            let batch_size = (batch_end - current + 1) as usize;

            // Build documents for this batch
            let mut documents = Vec::with_capacity(batch_size);
            for i in current..=batch_end {
                let mut ctx = initial_bindings.clone();
                ctx.insert(
                    var_name.clone(),
                    Value::Number(serde_json::Number::from(i)),
                );
                let doc_value = self.evaluate_expr_with_context(&insert_clause.document, &ctx)?;
                documents.push(doc_value);
            }

            // Batch insert
            let inserted_docs = collection.insert_batch(documents)?;

            // Handle RETURN clause if present
            if query.return_clause.is_some() {
                for i in current..=batch_end {
                    all_results.push(Value::Number(serde_json::Number::from(i)));
                }
            }

            // Log to replication asynchronously
            self.log_mutations_async(&insert_clause.collection, Operation::Insert, &inserted_docs);

            // Index documents if needed
            if has_indexes && !inserted_docs.is_empty() {
                let _ = collection.index_documents(&inserted_docs);
            }

            current = batch_end + 1;

            // Throttled flush of stats (max 1 per second)
            collection.flush_stats_throttled();

            // Log progress for very large inserts
            if total_count > 100_000 && (current - start) % 100_000 == 0 {
                tracing::info!(
                    "STREAMING INSERT: Processed {}/{} documents",
                    current - start,
                    total_count
                );
            }
        }

        let elapsed = total_start.elapsed();
        tracing::info!(
            "STREAMING INSERT: Completed {} documents in {:?} ({:.0} docs/sec)",
            total_count,
            elapsed,
            total_count as f64 / elapsed.as_secs_f64()
        );

        // Final flush to ensure count is persisted
        collection.flush_stats();

        Ok(Some((all_results, total_count as usize)))
    }

    /// Log a mutation to the replication service
    fn log_mutation(
        &self,
        collection: &str,
        operation: Operation,
        key: &str,
        data: Option<&Value>,
    ) {
        if let (Some(repl), Some(ref db)) = (&self.replication, &self.database) {
            let entry = LogEntry {
                sequence: 0,
                node_id: "".to_string(),
                database: db.clone(),
                collection: collection.to_string(),
                operation,
                key: key.to_string(),
                data: data.and_then(|v| serde_json::to_vec(v).ok()),
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            };
            let _ = repl.append(entry);
        }
    }

    /// Log multiple mutations asynchronously in a background thread
    /// Used for bulk INSERT operations to avoid blocking the response
    /// Log multiple mutations asynchronously in a background thread
    /// Used for bulk INSERT operations to avoid blocking the response
    fn log_mutations_async(
        &self,
        collection: &str,
        operation: Operation,
        docs: &[crate::storage::Document],
    ) {
        // Clone the replication service if available
        let repl_clone = self.replication.map(|r| r.clone());
        let db_clone = self.database.clone();

        if let (Some(repl), Some(db)) = (repl_clone, db_clone) {
            let collection = collection.to_string();
            
            // Serialize documents upfront
            let entries: Vec<LogEntry> = docs
                .iter()
                .map(|doc| LogEntry {
                    sequence: 0,
                    node_id: "".to_string(),
                    database: db.clone(),
                    collection: collection.clone(),
                    operation: operation.clone(), // Operation must be Clone (it's Copy usually?)
                    key: doc.key.clone(),
                    data: serde_json::to_vec(&doc.to_value()).ok(),
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                    origin_sequence: None,
                })
                .collect();

            let count = entries.len();
            tracing::debug!(
                "INSERT: Starting async replication logging for {} docs",
                count
            );

            // Execute replication logging synchronously (RocksDB is fast)
            // We use the cloned ReplicationLog reference which points to the same DB
            let start = std::time::Instant::now();
            let _ = repl.append_batch(entries);
            let elapsed = start.elapsed();
            tracing::debug!(
                "INSERT: Replication logging of {} docs completed in {:?}",
                count,
                elapsed
            );
        }
    }

    /// Get collection with database prefix if set
    /// Uses database.get_collection() to share the same cached Collection instances
    fn get_collection(&self, name: &str) -> DbResult<crate::storage::Collection> {
        // If we have a database context, get collection through the database
        // This ensures we use the same cached Collection instances as the handlers
        if let Some(ref db_name) = self.database {
            let database = self.storage.get_database(db_name)?;
            database.get_collection(name)
        } else {
            // No database context - fall back to legacy storage method
            self.storage.get_collection(name)
        }
    }

    /// Execute query and return results only (backwards compatible)
    pub fn execute(&self, query: &Query) -> DbResult<Vec<Value>> {
        let result = self.execute_with_stats(query)?;
        Ok(result.results)
    }

    /// Execute query and return full results with mutation statistics
    pub fn execute_with_stats(&self, query: &Query) -> DbResult<QueryExecutionResult> {
        // First, evaluate initial LET clauses (before any FOR) to create initial bindings
        let mut initial_bindings: Context = HashMap::new();

        // Merge bind variables into initial context
        for (key, value) in &self.bind_vars {
            initial_bindings.insert(format!("@{}", key), value.clone());
        }

        for let_clause in &query.let_clauses {
            let value =
                self.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
            initial_bindings.insert(let_clause.variable.clone(), value);
        }

        // Optimization: Streaming bulk INSERT for range-based FOR loops
        // Pattern: FOR i IN start..end INSERT {...} INTO collection [RETURN ...]
        // This avoids materializing millions of row contexts in memory
        if let Some((results, insert_count)) = self.try_streaming_bulk_insert(query, &initial_bindings)? {
            return Ok(QueryExecutionResult {
                results,
                mutations: MutationStats {
                    documents_inserted: insert_count,
                    documents_updated: 0,
                    documents_removed: 0,
                },
            });
        }

        // Optimization: Use index for SORT + LIMIT if available
        // Check if query is: FOR var IN collection SORT var.field LIMIT n RETURN ...
        if let (Some(sort), Some(limit)) = (&query.sort_clause, &query.limit_clause) {
            // Check if we have a simple FOR loop on a collection
            // Only optimize single field sort for now
            if query.body_clauses.len() == 1 && sort.fields.len() == 1 {
                if let Some(BodyClause::For(for_clause)) = query.body_clauses.first() {
                    let (sort_expr, sort_asc) = &sort.fields[0];
                    
                    // Evaluate limit expressions
                    let limit_offset = self.evaluate_expr_with_context(&limit.offset, &initial_bindings)
                        .ok().and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(0);
                    let limit_count = self.evaluate_expr_with_context(&limit.count, &initial_bindings)
                        .ok().and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(0);

                    // Check if the sort field is on the loop variable
                    // Check if sort expression is a simple field access on the loop variable
                    if let Expression::FieldAccess(base, field) = sort_expr {
                        if let Expression::Variable(var) = base.as_ref() {
                            if var == &for_clause.variable {
                                // Try to get collection and check for index
                                if let Ok(collection) = self.get_collection(&for_clause.collection)
                                {
                                    if let Some(docs) = collection.index_sorted(
                                        field,
                                        *sort_asc,
                                        Some(limit_offset + limit_count),
                                    ) {
                                        // Got sorted documents from index! Apply offset and build result
                                        let start = limit_offset.min(docs.len());
                                        let end = (start + limit_count).min(docs.len());
                                        let docs = &docs[start..end];

                                        let results = if let Some(ref return_clause) = query.return_clause {
                                            let results: DbResult<Vec<Value>> = docs
                                                .iter()
                                                .map(|doc| {
                                                    let mut ctx = initial_bindings.clone();
                                                    ctx.insert(
                                                        for_clause.variable.clone(),
                                                        doc.to_value(),
                                                    );
                                                    self.evaluate_expr_with_context(
                                                        &return_clause.expression,
                                                        &ctx,
                                                    )
                                                })
                                                .collect();
                                            results?
                                        } else {
                                            // No RETURN clause - return empty array
                                            vec![]
                                        };
                                        // Index-sorted optimization is read-only, no mutations
                                        return Ok(QueryExecutionResult {
                                            results,
                                            mutations: MutationStats::new(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Optimization: Check if we can push LIMIT down to storage scan
        let scan_limit = if query.sort_clause.is_none() {
            let for_count = query
                .body_clauses
                .iter()
                .filter(|c| matches!(c, BodyClause::For(_)))
                .count();
            let filter_count = query
                .body_clauses
                .iter()
                .filter(|c| matches!(c, BodyClause::Filter(_)))
                .count();

            if for_count == 1 && filter_count == 0 {
                query.limit_clause.as_ref().map(|l| {
                     let offset = self.evaluate_expr_with_context(&l.offset, &initial_bindings)
                        .ok().and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(0);
                     let count = self.evaluate_expr_with_context(&l.count, &initial_bindings)
                        .ok().and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(0);
                     offset + count
                })
            } else {
                None
            }
        } else {
            None
        };

        // Process body_clauses in order (supports correlated subqueries)
        // If body_clauses is empty, fall back to legacy behavior
        let (rows, mutation_stats) = if !query.body_clauses.is_empty() {
            self.execute_body_clauses(&query.body_clauses, &initial_bindings, scan_limit)?
        } else {
            // Legacy path: use for_clauses and filter_clauses separately
            let mut rows =
                self.build_row_combinations_with_context(&query.for_clauses, &initial_bindings)?;
            for filter in &query.filter_clauses {
                rows.retain(|ctx| {
                    self.evaluate_filter_with_context(&filter.expression, ctx)
                        .unwrap_or(false)
                });
            }
            (rows, MutationStats::new())
        };

        let mut rows = rows;

        // Apply SORT
        if let Some(sort) = &query.sort_clause {
            rows.sort_by(|a, b| {
                for (expr, ascending) in &sort.fields {
                    let a_val = self
                        .evaluate_expr_with_context(expr, a)
                        .unwrap_or(Value::Null);
                    let b_val = self
                        .evaluate_expr_with_context(expr, b)
                        .unwrap_or(Value::Null);

                    let cmp = compare_values(&a_val, &b_val);
                    if cmp != std::cmp::Ordering::Equal {
                        return if *ascending {
                            cmp
                        } else {
                            cmp.reverse()
                        };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }

        // Apply LIMIT
        if let Some(limit) = &query.limit_clause {
            let offset = self.evaluate_expr_with_context(&limit.offset, &initial_bindings)
                .ok().and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(0);
            let count = self.evaluate_expr_with_context(&limit.count, &initial_bindings)
                .ok().and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(0);

            let start = offset.min(rows.len());
            let end = (start + count).min(rows.len());
            rows = rows[start..end].to_vec();
        }

        // Apply RETURN projection (if present)
        let results = if let Some(ref return_clause) = query.return_clause {
            let results: DbResult<Vec<Value>> = rows
                .iter()
                .map(|ctx| self.evaluate_expr_with_context(&return_clause.expression, ctx))
                .collect();
            results?
        } else {
            // No RETURN clause - return empty array (mutations don't need to return anything)
            vec![]
        };

        Ok(QueryExecutionResult {
            results,
            mutations: mutation_stats,
        })
    }

    /// Execute body clauses in order, supporting correlated subqueries
    /// LET clauses inside FOR loops are evaluated per-row with access to outer variables
    /// Returns (row_contexts, mutation_stats) - mutation stats track INSERT/UPDATE/REMOVE counts
    fn execute_body_clauses(
        &self,
        clauses: &[BodyClause],
        initial_ctx: &Context,
        scan_limit: Option<usize>,
    ) -> DbResult<(Vec<Context>, MutationStats)> {
        let mut rows: Vec<Context> = vec![initial_ctx.clone()];
        let mut stats = MutationStats::new();

        // Optimization: Check if we can use index for FOR + FILTER pattern
        // Pattern: FOR var IN collection, followed by FILTER on var.field == value
        let mut i = 0;
        while i < clauses.len() {
            match &clauses[i] {
                BodyClause::For(for_clause) => {
                    // Check if next clause is a FILTER that can use an index
                    let use_index = if i + 1 < clauses.len() {
                        if let BodyClause::Filter(filter_clause) = &clauses[i + 1] {
                            // Check if this is a collection (not a LET variable)
                            // source_variable might be None or Some(collection_name)
                            let is_collection = if let Some(src) = &for_clause.source_variable {
                                // If source_variable == collection, it's a collection
                                src == &for_clause.collection
                            } else {
                                // If source_variable is None, it's definitely a collection
                                true
                            };

                            if is_collection {
                                // Try to extract indexable condition
                                self.extract_indexable_condition(
                                    &filter_clause.expression,
                                    &for_clause.variable,
                                )
                                .is_some()
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if use_index {
                        // Try to use index lookup
                        if let BodyClause::Filter(filter_clause) = &clauses[i + 1] {
                            let mut used_index = false;
                            let mut new_rows = Vec::new();

                            for ctx in &rows {
                                if let Ok(collection) = self.get_collection(&for_clause.collection)
                                {
                                    if let Some(condition) = self.extract_indexable_condition(
                                        &filter_clause.expression,
                                        &for_clause.variable,
                                    ) {
                                        if let Some(docs) =
                                            self.use_index_for_condition(&collection, &condition)
                                        {
                                            used_index = true;
                                            if !docs.is_empty() {
                                                // Apply scan_limit to index results
                                                let docs: Vec<_> = if let Some(n) = scan_limit {
                                                    docs.into_iter().take(n).collect()
                                                } else {
                                                    docs
                                                };

                                                for doc in docs {
                                                    let mut new_ctx = ctx.clone();
                                                    new_ctx.insert(
                                                        for_clause.variable.clone(),
                                                        doc.to_value(),
                                                    );
                                                    new_rows.push(new_ctx);
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            // Only use index results if we actually found documents
                            if used_index {
                                rows = new_rows;
                                i += 2; // Skip both FOR and FILTER
                                continue;
                            }
                            // Otherwise fall through to normal FOR processing
                        }
                    }

                    // Normal FOR processing (no index)
                    let mut new_rows = Vec::new();
                    for ctx in &rows {
                        let docs = self.get_for_source_docs(for_clause, ctx, scan_limit)?;
                        for doc in docs {
                            let mut new_ctx = ctx.clone();
                            new_ctx.insert(for_clause.variable.clone(), doc);
                            new_rows.push(new_ctx);
                        }
                    }
                    rows = new_rows;
                }
                BodyClause::Let(let_clause) => {
                    // Evaluate LET expression for EACH row (correlated subquery support)
                    for ctx in &mut rows {
                        let value = self.evaluate_expr_with_context(&let_clause.expression, ctx)?;
                        ctx.insert(let_clause.variable.clone(), value);
                    }
                }
                BodyClause::Filter(filter_clause) => {
                    // Filter out rows that don't match
                    rows.retain(|ctx| {
                        self.evaluate_filter_with_context(&filter_clause.expression, ctx)
                            .unwrap_or(false)
                    });
                }
                 BodyClause::Insert(insert_clause) => {
                    // Get collection once, outside the loop
                    let collection = self.get_collection(&insert_clause.collection)?;

                    // SHARDING SUPPORT - Use batch insert for performance
                    if let (Some(config), Some(coordinator)) = (collection.get_shard_config(), &self.shard_coordinator) {
                        if config.num_shards > 0 {
                             tracing::info!("INSERT: Using ShardCoordinator BATCH for {} documents into {}", rows.len(), insert_clause.collection);
                             
                             // Evaluate all documents first
                             let mut documents = Vec::with_capacity(rows.len());
                             for ctx in &rows {
                                 let doc_value = self.evaluate_expr_with_context(&insert_clause.document, ctx)?;
                                 documents.push(doc_value);
                             }
                             
                             // Use batch insert via coordinator (groups by shard internally)
                             let handle = tokio::runtime::Handle::current();
                             let db_name = self.database.as_deref().unwrap_or("_system").to_string();
                             let coll_name = insert_clause.collection.clone();
                             let config = config.clone();
                             let coord = coordinator.clone();
                             
                             let (tx, rx) = std::sync::mpsc::sync_channel(1);
                             
                             handle.spawn(async move {
                                 let res = coord.insert_batch(&db_name, &coll_name, &config, documents).await;
                                 let _ = tx.send(res);
                             });
                             
                             // Wait for batch result
                             let result = rx.recv().map_err(|_| DbError::InternalError("Sharded batch insert failed".to_string()))??;
                             tracing::debug!("INSERT: Sharded batch completed - {} success, {} failed", result.0, result.1);
                             stats.documents_inserted += result.0;

                             i += 1; // CRITICAL: Advance to next clause before continuing
                             continue; // Skip standard insert logic
                        }
                    }

                    // For bulk inserts (>100 docs), use batch mode for maximum performance
                    let bulk_mode = rows.len() > 100;
                    let has_indexes = !collection.list_indexes().is_empty();

                    tracing::debug!(
                        "INSERT: {} documents, bulk_mode={}, has_indexes={}",
                        rows.len(),
                        bulk_mode,
                        has_indexes
                    );

                    if bulk_mode {
                        // Evaluate all documents first
                        let eval_start = std::time::Instant::now();
                        let mut documents = Vec::with_capacity(rows.len());
                        for ctx in &rows {
                            let doc_value =
                                self.evaluate_expr_with_context(&insert_clause.document, ctx)?;
                            documents.push(doc_value);
                        }
                        let eval_time = eval_start.elapsed();
                        tracing::debug!("INSERT: Document evaluation took {:?}", eval_time);

                        // Batch insert all documents at once (uses RocksDB WriteBatch)
                        let insert_start = std::time::Instant::now();
                        let inserted_docs = collection.insert_batch(documents)?;
                        let insert_time = insert_start.elapsed();
                        stats.documents_inserted += inserted_docs.len();
                        tracing::debug!(
                            "INSERT: Batch insert of {} docs took {:?}",
                            inserted_docs.len(),
                            insert_time
                        );

                        // Log to replication asynchronously for bulk inserts
                        self.log_mutations_async(
                            &insert_clause.collection,
                            Operation::Insert,
                            &inserted_docs,
                        );

                        // Index ONLY the newly inserted documents asynchronously
                        if has_indexes {
                            tracing::debug!(
                                "INSERT: Starting async indexing of {} new docs",
                                inserted_docs.len()
                            );
                            let coll = collection.clone();
                            std::thread::spawn(move || {
                                let index_start = std::time::Instant::now();
                                let result = coll.index_documents(&inserted_docs);
                                let index_time = index_start.elapsed();
                                match result {
                                    Ok(count) => tracing::debug!(
                                        "INSERT: Indexed {} docs in {:?}",
                                        count,
                                        index_time
                                    ),
                                    Err(e) => tracing::error!("INSERT: Indexing failed: {}", e),
                                }
                            });
                        }
                    } else {
                        // Small inserts - use normal path with indexes
                        let insert_start = std::time::Instant::now();
                        let insert_count = rows.len();
                        for ctx in &rows {
                            let doc_value =
                                self.evaluate_expr_with_context(&insert_clause.document, ctx)?;
                            let doc = collection.insert(doc_value)?;
                            // Log to replication
                            self.log_mutation(
                                &insert_clause.collection,
                                Operation::Insert,
                                &doc.key,
                                Some(&doc.to_value()),
                            );
                        }
                        stats.documents_inserted += insert_count;
                        let insert_time = insert_start.elapsed();
                        tracing::debug!(
                            "INSERT: {} docs with indexes took {:?}",
                            rows.len(),
                            insert_time
                        );
                    }
                }
                BodyClause::Update(update_clause) => {
                    // Get collection once, outside the loop
                    let collection = self.get_collection(&update_clause.collection)?;
                    
                    // SHARDING SUPPORT
                    if let (Some(config), Some(coordinator)) = (collection.get_shard_config(), &self.shard_coordinator) {
                        if config.num_shards > 0 {
                             tracing::debug!("UPDATE: Delegating to ShardCoordinator for {}", update_clause.collection);
                             let handle = tokio::runtime::Handle::current();
                             let db_name = self.database.as_deref().unwrap_or("_system").to_string();
                             let coll_name = update_clause.collection.clone();
                             let config = config.clone();
                             
                             for ctx in &mut rows {
                                // Evaluate selector (Duplicated logic)
                                let selector_value = self.evaluate_expr_with_context(&update_clause.selector, ctx)?;
                                let key = match &selector_value {
                                    Value::String(s) => s.clone(),
                                    Value::Object(obj) => obj.get("_key").and_then(|v| v.as_str()).map(|s| s.to_string()).ok_or_else(|| DbError::ExecutionError("UPDATE: missing _key".to_string()))?,
                                    _ => return Err(DbError::ExecutionError("UPDATE: invalid selector".to_string())),
                                };
                                let changes = self.evaluate_expr_with_context(&update_clause.changes, ctx)?;
                                if !changes.is_object() { return Err(DbError::ExecutionError("UPDATE: changes must be object".to_string())); }
                                
                                let coord = coordinator.clone();
                                let db = db_name.clone();
                                let coll = coll_name.clone();
                                let conf = config.clone();
                                let k = key;
                                let doc = changes;
                                
                                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                                handle.spawn(async move {
                                      let res = coord.update(&db, &coll, &conf, &k, doc).await;
                                      let _ = tx.send(res);
                                });
                                let updated_doc = rx.recv().map_err(|_| DbError::InternalError("Sharded update task failed".to_string()))??;
                                stats.documents_updated += 1;

                                // Inject NEW variable
                                ctx.insert("NEW".to_string(), updated_doc.clone());
                             }
                             i += 1; // CRITICAL: Advance to next clause
                             continue;
                        }
                    }

                    // Update documents for each row context
                    for ctx in &mut rows {
                        // Evaluate selector expression to get the document key
                        let selector_value =
                            self.evaluate_expr_with_context(&update_clause.selector, ctx)?;

                        // Extract _key from selector (can be a string key or a document with _key field)
                        let key = match &selector_value {
                            Value::String(s) => s.clone(),
                            Value::Object(obj) => {
                                obj.get("_key")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .ok_or_else(|| DbError::ExecutionError(
                                        "UPDATE: selector object must have a _key field".to_string()
                                    ))?
                            }
                            _ => return Err(DbError::ExecutionError(
                                "UPDATE: selector must be a string key or an object with _key field".to_string()
                            )),
                        };

                        // Evaluate changes expression
                        let changes_value =
                            self.evaluate_expr_with_context(&update_clause.changes, ctx)?;

                        // Ensure changes is an object
                        if !changes_value.is_object() {
                            return Err(DbError::ExecutionError(
                                "UPDATE: changes must be an object".to_string(),
                            ));
                        }

                        // Update the document (collection.update handles merging internally)
                        let doc = collection.update(&key, changes_value)?;
                        stats.documents_updated += 1;

                        // Log to replication
                        self.log_mutation(
                            &update_clause.collection,
                            Operation::Update,
                            &key,
                            Some(&doc.to_value()),
                        );

                        // Inject NEW variable
                        ctx.insert("NEW".to_string(), doc.to_value());
                    }
                }
                BodyClause::Remove(remove_clause) => {
                    // Get collection once, outside the loop
                    let collection = self.get_collection(&remove_clause.collection)?;
                    
                    // SHARDING SUPPORT
                    if let (Some(config), Some(coordinator)) = (collection.get_shard_config(), &self.shard_coordinator) {
                        if config.num_shards > 0 {
                             tracing::debug!("REMOVE: Delegating to ShardCoordinator for {}", remove_clause.collection);
                             let handle = tokio::runtime::Handle::current();
                             let db_name = self.database.as_deref().unwrap_or("_system").to_string();
                             let coll_name = remove_clause.collection.clone();
                             let config = config.clone();
                             
                             for ctx in &rows {
                                // Evaluate selector (Duplicated logic)
                                let selector_value = self.evaluate_expr_with_context(&remove_clause.selector, ctx)?;
                                let key = match &selector_value {
                                    Value::String(s) => s.clone(),
                                    Value::Object(obj) => obj.get("_key").and_then(|v| v.as_str()).map(|s| s.to_string()).ok_or_else(|| DbError::ExecutionError("REMOVE: missing _key".to_string()))?,
                                    _ => return Err(DbError::ExecutionError("REMOVE: invalid selector".to_string())),
                                };
                                
                                let coord = coordinator.clone();
                                let db = db_name.clone();
                                let coll = coll_name.clone();
                                let conf = config.clone();
                                let k = key;
                                
                                let (tx, rx) = std::sync::mpsc::sync_channel(1);
                                handle.spawn(async move {
                                      let res = coord.delete(&db, &coll, &conf, &k).await;
                                      let _ = tx.send(res);
                                });
                                let _ = rx.recv().map_err(|_| DbError::InternalError("Sharded remove task failed".to_string()))??;
                                stats.documents_removed += 1;
                             }
                             i += 1; // CRITICAL: Advance to next clause
                             continue;
                        }
                    }

                    // Remove documents for each row context
                    for ctx in &rows {
                        // Evaluate selector expression to get the document key
                        let selector_value =
                            self.evaluate_expr_with_context(&remove_clause.selector, ctx)?;

                        // Extract _key from selector (can be a string key or a document with _key field)
                        let key = match &selector_value {
                            Value::String(s) => s.clone(),
                            Value::Object(obj) => {
                                obj.get("_key")
                                    .and_then(|v| v.as_str())
                                    .map(|s| s.to_string())
                                    .ok_or_else(|| DbError::ExecutionError(
                                        "REMOVE: selector object must have a _key field".to_string()
                                    ))?
                            }
                            _ => return Err(DbError::ExecutionError(
                                "REMOVE: selector must be a string key or an object with _key field".to_string()
                            )),
                        };

                        // Delete the document
                        collection.delete(&key)?;
                        stats.documents_removed += 1;
                        // Log to replication
                        self.log_mutation(&remove_clause.collection, Operation::Delete, &key, None);
                    }
                }
                BodyClause::Upsert(upsert_clause) => {
                    let collection = self.get_collection(&upsert_clause.collection)?;
                    
                    for ctx in &mut rows {
                        let search_value = self.evaluate_expr_with_context(&upsert_clause.search, ctx)?;
                        
                        let mut found_doc_key: Option<String> = None;
                        
                        if let Some(s) = search_value.as_str() {
                             if collection.get(s).is_ok() {
                                 found_doc_key = Some(s.to_string());
                             }
                        } else if let Some(obj) = search_value.as_object() {
                              if let Some(k) = obj.get("_key").or_else(|| obj.get("_id")) {
                                   if let Some(ks) = k.as_str() {
                                        if collection.get(ks).is_ok() {
                                             found_doc_key = Some(ks.to_string());
                                        }
                                   }
                              }
                        }

                        if let Some(key) = found_doc_key {
                            // Update
                            let update_value = self.evaluate_expr_with_context(&upsert_clause.update, ctx)?;
                            if !update_value.is_object() {
                                return Err(DbError::ExecutionError("UPSERT: update expression must be an object".to_string()));
                            }

                            let doc = collection.update(&key, update_value)?;
                            stats.documents_updated += 1;

                            self.log_mutation(
                                &upsert_clause.collection,
                                Operation::Update,
                                &key,
                                Some(&doc.to_value()),
                            );
                            ctx.insert("NEW".to_string(), doc.to_value());

                        } else {
                             // Insert
                            let insert_value = self.evaluate_expr_with_context(&upsert_clause.insert, ctx)?;
                            let doc = collection.insert(insert_value)?;
                            stats.documents_inserted += 1;

                            self.log_mutation(
                                &upsert_clause.collection,
                                Operation::Insert,
                                &doc.key,
                                Some(&doc.to_value()),
                            );
                            ctx.insert("NEW".to_string(), doc.to_value());
                        }
                    }
                }
                BodyClause::GraphTraversal(gt) => {
                    // Execute graph traversal using BFS
                    let mut new_rows = Vec::new();

                    for ctx in &rows {
                        // Evaluate start vertex
                        let start_value = self.evaluate_expr_with_context(&gt.start_vertex, ctx)?;
                        let start_id = match &start_value {
                            Value::String(s) => s.clone(),
                            _ => return Err(DbError::ExecutionError(
                                "Start vertex must be a string (e.g., 'users/alice')".to_string()
                            )),
                        };

                        // Get edge collection
                        let edge_collection = self.get_collection(&gt.edge_collection)?;

                        // BFS traversal
                        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
                        let mut queue: std::collections::VecDeque<(String, usize, Option<Value>)> = std::collections::VecDeque::new();
                        visited.insert(start_id.clone());
                        queue.push_back((start_id.clone(), 0, None));

                        while let Some((current_id, depth, edge)) = queue.pop_front() {
                            // Add result if within depth range
                            if depth >= gt.min_depth && depth <= gt.max_depth {
                                // Get vertex document
                                if let Some((coll_name, key)) = current_id.split_once('/') {
                                    if let Ok(vertex_coll) = self.get_collection(coll_name) {
                                        if let Ok(vertex_doc) = vertex_coll.get(key) {
                                            let mut new_ctx = ctx.clone();
                                            new_ctx.insert(gt.vertex_var.clone(), vertex_doc.to_value());
                                            if let Some(ref edge_var) = gt.edge_var {
                                                new_ctx.insert(edge_var.clone(), edge.clone().unwrap_or(Value::Null));
                                            }
                                            new_rows.push(new_ctx);
                                        }
                                    }
                                }
                            }

                            // Continue traversal if not at max depth
                            if depth >= gt.max_depth {
                                continue;
                            }

                            // Find connected vertices
                            let edges = edge_collection.scan(None);
                            for edge_doc in edges {
                                let edge_val = edge_doc.to_value();
                                let from = edge_val.get("_from").and_then(|v| v.as_str());
                                let to = edge_val.get("_to").and_then(|v| v.as_str());

                                let next_id = match gt.direction {
                                    EdgeDirection::Outbound => {
                                        if from == Some(current_id.as_str()) {
                                            to.map(|s| s.to_string())
                                        } else { None }
                                    }
                                    EdgeDirection::Inbound => {
                                        if to == Some(current_id.as_str()) {
                                            from.map(|s| s.to_string())
                                        } else { None }
                                    }
                                    EdgeDirection::Any => {
                                        if from == Some(current_id.as_str()) {
                                            to.map(|s| s.to_string())
                                        } else if to == Some(current_id.as_str()) {
                                            from.map(|s| s.to_string())
                                        } else { None }
                                    }
                                };

                                if let Some(next) = next_id {
                                    if !visited.contains(&next) {
                                        visited.insert(next.clone());
                                        queue.push_back((next, depth + 1, Some(edge_val.clone())));
                                    }
                                }
                            }
                        }
                    }
                    rows = new_rows;
                }
                BodyClause::ShortestPath(sp) => {
                    // Execute shortest path using BFS
                    let mut new_rows = Vec::new();

                    for ctx in &rows {
                        let start_value = self.evaluate_expr_with_context(&sp.start_vertex, ctx)?;
                        let start_id = match &start_value {
                            Value::String(s) => s.clone(),
                            _ => return Err(DbError::ExecutionError("Start vertex must be a string".to_string())),
                        };

                        let end_value = self.evaluate_expr_with_context(&sp.end_vertex, ctx)?;
                        let end_id = match &end_value {
                            Value::String(s) => s.clone(),
                            _ => return Err(DbError::ExecutionError("End vertex must be a string".to_string())),
                        };

                        let edge_collection = self.get_collection(&sp.edge_collection)?;

                        // BFS with parent tracking
                        let mut visited: std::collections::HashMap<String, (Option<String>, Option<Value>)> = std::collections::HashMap::new();
                        let mut queue: std::collections::VecDeque<String> = std::collections::VecDeque::new();

                        visited.insert(start_id.clone(), (None, None));
                        queue.push_back(start_id.clone());
                        let mut found = false;

                        while let Some(current_id) = queue.pop_front() {
                            if current_id == end_id { found = true; break; }

                            let edges = edge_collection.scan(None);
                            for edge_doc in edges {
                                let edge_val = edge_doc.to_value();
                                let from = edge_val.get("_from").and_then(|v| v.as_str());
                                let to = edge_val.get("_to").and_then(|v| v.as_str());

                                let next_id = match sp.direction {
                                    EdgeDirection::Outbound => {
                                        if from == Some(current_id.as_str()) { to.map(|s| s.to_string()) } else { None }
                                    }
                                    EdgeDirection::Inbound => {
                                        if to == Some(current_id.as_str()) { from.map(|s| s.to_string()) } else { None }
                                    }
                                    EdgeDirection::Any => {
                                        if from == Some(current_id.as_str()) { to.map(|s| s.to_string()) }
                                        else if to == Some(current_id.as_str()) { from.map(|s| s.to_string()) }
                                        else { None }
                                    }
                                };

                                if let Some(next) = next_id {
                                    if !visited.contains_key(&next) {
                                        visited.insert(next.clone(), (Some(current_id.clone()), Some(edge_val.clone())));
                                        queue.push_back(next);
                                    }
                                }
                            }
                        }

                        // Reconstruct path
                        if found {
                            let mut path: Vec<(String, Option<Value>)> = Vec::new();
                            let mut current = end_id.clone();

                            while let Some((parent, edge)) = visited.get(&current) {
                                path.push((current.clone(), edge.clone()));
                                if let Some(p) = parent { current = p.clone(); } else { break; }
                            }
                            path.reverse();

                            for (vertex_id, edge) in path {
                                if let Some((coll_name, key)) = vertex_id.split_once('/') {
                                    if let Ok(vertex_coll) = self.get_collection(coll_name) {
                                        if let Ok(vertex_doc) = vertex_coll.get(key) {
                                            let mut new_ctx = ctx.clone();
                                            new_ctx.insert(sp.vertex_var.clone(), vertex_doc.to_value());
                                            if let Some(ref edge_var) = sp.edge_var {
                                                new_ctx.insert(edge_var.clone(), edge.unwrap_or(Value::Null));
                                            }
                                            new_rows.push(new_ctx);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    rows = new_rows;
                }

                BodyClause::Collect(collect) => {
                    use std::collections::HashMap;
                    
                    // Group rows by the collect key(s)
                    let mut groups: HashMap<String, (Context, Vec<Context>, i64)> = HashMap::new();
                    
                    for ctx in rows {
                        // Evaluate group key expressions
                        let mut key_parts = Vec::new();
                        let mut group_ctx = Context::new();
                        
                        for (var_name, expr) in &collect.group_vars {
                            let val = self.evaluate_expr_with_context(expr, &ctx)?;
                            key_parts.push(serde_json::to_string(&val).unwrap_or_default());
                            group_ctx.insert(var_name.clone(), val);
                        }
                        
                        let group_key = key_parts.join("|");
                        
                        let entry = groups.entry(group_key).or_insert_with(|| {
                            (group_ctx.clone(), Vec::new(), 0)
                        });
                        
                        // Collect into groups
                        entry.1.push(ctx.clone());
                        entry.2 += 1;
                    }
                    
                    // Build result rows from groups
                    let mut new_rows = Vec::new();
                    
                    for (_key, (mut group_ctx, group_docs, count)) in groups {
                        // Add INTO variable if present
                        if let Some(ref into_var) = collect.into_var {
                            let group_array: Vec<Value> = group_docs.iter()
                                .map(|ctx| {
                                    // Create an object with all variables in the context
                                    let obj: serde_json::Map<String, Value> = ctx.iter()
                                        .map(|(k, v)| (k.clone(), v.clone()))
                                        .collect();
                                    Value::Object(obj)
                                })
                                .collect();
                            group_ctx.insert(into_var.clone(), Value::Array(group_array));
                        }
                        
                        // Add COUNT variable if present
                        if let Some(ref count_var) = collect.count_var {
                            group_ctx.insert(count_var.clone(), Value::Number(count.into()));
                        }
                        
                        // Compute aggregates
                        for agg in &collect.aggregates {
                            let agg_value = self.compute_aggregate(
                                &agg.function,
                                &agg.argument,
                                &group_docs,
                            )?;
                            group_ctx.insert(agg.variable.clone(), agg_value);
                        }
                        
                        new_rows.push(group_ctx);
                    }
                    
                    rows = new_rows;
                }
            }
            i += 1;
        }

        Ok((rows, stats))
    }

    /// Compute aggregate function over group of rows
    fn compute_aggregate(
        &self,
        function: &str,
        argument: &Option<Expression>,
        group_docs: &[Context],
    ) -> DbResult<Value> {
        match function {
            "COUNT" => {
                if argument.is_none() {
                    // COUNT() - count all rows
                    Ok(Value::Number((group_docs.len() as i64).into()))
                } else {
                    // COUNT(expr) - count non-null values
                    let mut count = 0i64;
                    for ctx in group_docs {
                        if let Some(expr) = argument {
                            let val = self.evaluate_expr_with_context(expr, ctx)?;
                            if !val.is_null() {
                                count += 1;
                            }
                        }
                    }
                    Ok(Value::Number(count.into()))
                }
            }
            "SUM" => {
                let mut sum = 0.0f64;
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        if let Some(n) = val.as_f64() {
                            sum += n;
                        } else if let Some(n) = val.as_i64() {
                            sum += n as f64;
                        }
                    }
                }
                Ok(Value::Number(serde_json::Number::from_f64(sum).unwrap_or_else(|| (sum as i64).into())))
            }
            "AVG" => {
                let mut sum = 0.0f64;
                let mut count = 0i64;
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        if let Some(n) = val.as_f64() {
                            sum += n;
                            count += 1;
                        } else if let Some(n) = val.as_i64() {
                            sum += n as f64;
                            count += 1;
                        }
                    }
                }
                if count == 0 {
                    Ok(Value::Null)
                } else {
                    let avg = sum / (count as f64);
                    Ok(Value::Number(serde_json::Number::from_f64(avg).unwrap_or_else(|| (avg as i64).into())))
                }
            }
            "MIN" => {
                let mut min: Option<Value> = None;
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        if val.is_null() { continue; }
                        
                        if min.is_none() {
                            min = Some(val);
                        } else if let (Some(cur), Some(new)) = (min.as_ref().and_then(|v| v.as_f64()), val.as_f64()) {
                            if new < cur {
                                min = Some(val);
                            }
                        } else if let (Some(cur_str), Some(new_str)) = (min.as_ref().and_then(|v| v.as_str()), val.as_str()) {
                            if new_str < cur_str {
                                min = Some(val);
                            }
                        }
                    }
                }
                Ok(min.unwrap_or(Value::Null))
            }
            "MAX" => {
                let mut max: Option<Value> = None;
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        if val.is_null() { continue; }
                        
                        if max.is_none() {
                            max = Some(val);
                        } else if let (Some(cur), Some(new)) = (max.as_ref().and_then(|v| v.as_f64()), val.as_f64()) {
                            if new > cur {
                                max = Some(val);
                            }
                        } else if let (Some(cur_str), Some(new_str)) = (max.as_ref().and_then(|v| v.as_str()), val.as_str()) {
                            if new_str > cur_str {
                                max = Some(val);
                            }
                        }
                    }
                }
                Ok(max.unwrap_or(Value::Null))
            }
            "LENGTH" | "COUNT_DISTINCT" => {
                use std::collections::HashSet;
                let mut seen: HashSet<String> = HashSet::new();
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        seen.insert(serde_json::to_string(&val).unwrap_or_default());
                    }
                }
                Ok(Value::Number((seen.len() as i64).into()))
            }
            "COLLECT_LIST" | "COLLECT" => {
                let mut list = Vec::new();
                if let Some(expr) = argument {
                    for ctx in group_docs {
                        let val = self.evaluate_expr_with_context(expr, ctx)?;
                        list.push(val);
                    }
                }
                Ok(Value::Array(list))
            }
            _ => Err(DbError::ExecutionError(format!(
                "Unknown aggregate function: {}", function
            ))),
        }
    }

    /// Get documents for a FOR clause source (collection or variable)
    fn get_for_source_docs(
        &self,
        for_clause: &ForClause,
        ctx: &Context,
        limit: Option<usize>,
    ) -> DbResult<Vec<Value>> {
        // Check if source is an expression (e.g., range 1..5)
        if let Some(expr) = &for_clause.source_expression {
            let value = self.evaluate_expr_with_context(expr, ctx)?;
            return match value {
                Value::Array(arr) => {
                    if let Some(n) = limit {
                        Ok(arr.into_iter().take(n).collect())
                    } else {
                        Ok(arr)
                    }
                }
                other => Ok(vec![other]),
            };
        }

        let source_name = for_clause
            .source_variable
            .as_ref()
            .unwrap_or(&for_clause.collection);

        // Check if source is a LET variable in current context
        if let Some(value) = ctx.get(source_name) {
            return match value {
                Value::Array(arr) => {
                    if let Some(n) = limit {
                        Ok(arr.iter().take(n).cloned().collect())
                    } else {
                        Ok(arr.clone())
                    }
                }
                other => Ok(vec![other.clone()]),
            };
        }

        // Otherwise it's a collection - use scan with limit for optimization
        let collection = self.get_collection(&for_clause.collection)?;

        // Use scatter-gather for sharded collections to get data from all nodes
        if let Some(shard_config) = collection.get_shard_config() {
            if shard_config.num_shards > 0 {
                if let Some(ref coordinator) = self.shard_coordinator {
                    tracing::debug!("[SDBQL] Using scatter-gather for sharded collection {} ({} shards)", 
                        for_clause.collection, shard_config.num_shards);
                    return self.scatter_gather_docs(
                        &for_clause.collection,
                        coordinator,
                        limit,
                    );
                }
            }
        }

        // Local scan - for non-sharded collections or when no coordinator
        Ok(collection
            .scan(limit)
            .into_iter()
            .map(|d| d.to_value())
            .collect())
    }

    /// Scatter-gather query: fetch documents from all cluster nodes for sharded collection
    /// Queries each shard's primary node for the physical shard collection
    fn scatter_gather_docs(
        &self,
        collection_name: &str,
        coordinator: &crate::sharding::ShardCoordinator,
        limit: Option<usize>,
    ) -> DbResult<Vec<Value>> {
        let db_name = self.database.as_ref()
            .ok_or_else(|| DbError::ExecutionError("No database context for scatter-gather".to_string()))?;

        // Get shard table to know which node owns each shard
        let Some(table) = coordinator.get_shard_table(db_name, collection_name) else {
            tracing::debug!("[SCATTER-GATHER] No shard table found for {}, falling back to local scan", collection_name);
            let collection = self.get_collection(collection_name)?;
            return Ok(collection.scan(limit).into_iter().map(|d| d.to_value()).collect());
        };
        
        let my_node_id = coordinator.my_node_id();
        let mut all_docs: Vec<Value> = Vec::new();
        let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
        
        // Build client for remote queries
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        let cluster_secret = std::env::var("SOLIDB_CLUSTER_SECRET").unwrap_or_default();
        
        // Query each shard's primary node
        for shard_id in 0..table.num_shards {
            let physical_coll = format!("{}_s{}", collection_name, shard_id);
            
            if let Some(assignment) = table.assignments.get(&shard_id) {
                // Check if we have this shard locally (either as primary or replica)
                let is_primary = assignment.primary_node == my_node_id || assignment.primary_node == "local";
                let is_replica = assignment.replica_nodes.contains(&my_node_id);
                
                if is_primary || is_replica {
                    // This shard is local - scan it directly
                    if let Ok(coll) = self.storage.get_database(db_name)
                        .and_then(|db| db.get_collection(&physical_coll)) 
                    {
                        for doc in coll.scan(limit) {
                            let value = doc.to_value();
                            if let Some(key) = value.get("_key").and_then(|k| k.as_str()) {
                                if seen_keys.insert(key.to_string()) {
                                    all_docs.push(value);
                                }
                            }
                        }
                    }
                } else {
                    // This shard is remote - try primary first, then replicas
                    let mut nodes_to_try = vec![assignment.primary_node.clone()];
                    nodes_to_try.extend(assignment.replica_nodes.clone());
                    
                    let mut found = false;
                    for node_id in &nodes_to_try {
                        if let Some(addr) = coordinator.get_node_api_address(node_id) {
                            // Query physical shard collection directly via SDBQL
                            let scheme = std::env::var("SOLIDB_CLUSTER_SCHEME").unwrap_or_else(|_| "http".to_string());
                            let url = format!("{}://{}/_api/database/{}/cursor", scheme, addr, db_name);
                            let query = if let Some(n) = limit {
                                format!("FOR doc IN `{}` LIMIT {} RETURN doc", physical_coll, n)
                            } else {
                                format!("FOR doc IN `{}` RETURN doc", physical_coll)
                            };
                            
                            let response = client
                                .post(&url)
                                .header("X-Scatter-Gather", "true")
                                .header("X-Cluster-Secret", &cluster_secret)
                                .json(&serde_json::json!({ "query": query }))
                                .send();
                                
                            match response {
                                Ok(resp) => {
                                    if let Ok(body) = resp.json::<serde_json::Value>() {
                                        if let Some(results) = body.get("result").and_then(|r| r.as_array()) {
                                            for doc in results {
                                                if let Some(key) = doc.get("_key").and_then(|k| k.as_str()) {
                                                    if seen_keys.insert(key.to_string()) {
                                                        all_docs.push(doc.clone());
                                                    }
                                                }
                                            }
                                            found = true;
                                            break; // Got data, no need to try other nodes
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("[SCATTER-GATHER] Failed to query shard {} from {}: {}, trying next", 
                                        shard_id, node_id, e);
                                }
                            }
                        }
                    }
                    
                    if !found {
                        tracing::error!("[SCATTER-GATHER] CRITICAL: Could not get data for shard {} from any node. Data may be missing!", shard_id);
                    }
                }
            }
        }
        
        // Apply final limit
        if let Some(n) = limit {
            if all_docs.len() > n {
                all_docs.truncate(n);
            }
        }

        tracing::info!(
            "[SCATTER-GATHER] Collection {}: gathered {} unique docs from {} shards",
            collection_name, all_docs.len(), table.num_shards
        );

        Ok(all_docs)
    }

    /// Explain and profile a query execution
    pub fn explain(&self, query: &Query) -> DbResult<QueryExplain> {
        let total_start = Instant::now();
        let mut warnings: Vec<String> = Vec::new();
        let mut collections_info: Vec<CollectionAccess> = Vec::new();
        let mut let_bindings_info: Vec<LetBinding> = Vec::new();
        let mut filters_info: Vec<FilterInfo> = Vec::new();

        // Timing accumulators
        // Timing accumulators

        // First, evaluate all LET clauses
        let let_start = Instant::now();
        let mut let_bindings: Context = HashMap::new();

        for (key, value) in &self.bind_vars {
            let_bindings.insert(format!("@{}", key), value.clone());
        }

        for let_clause in &query.let_clauses {
            let clause_start = Instant::now();
            let is_subquery = matches!(let_clause.expression, Expression::Subquery(_));
            let value = self.evaluate_expr_with_context(&let_clause.expression, &let_bindings)?;
            let_bindings.insert(let_clause.variable.clone(), value);
            let clause_time = clause_start.elapsed();

            let_bindings_info.push(LetBinding {
                variable: let_clause.variable.clone(),
                is_subquery,
                time_us: clause_time.as_micros() as u64,
            });
        }
        let let_clauses_time = let_start.elapsed();

        // Analyze FOR clauses and build row combinations

        let mut total_docs_scanned = 0usize;

        for for_clause in &query.for_clauses {
            let source_name = for_clause
                .source_variable
                .as_ref()
                .unwrap_or(&for_clause.collection);

            // Check if source is a LET variable or collection
            let (docs_count, access_type, index_used, index_type) =
                if let_bindings.contains_key(source_name) {
                    let arr_len = match let_bindings.get(source_name) {
                        Some(Value::Array(arr)) => arr.len(),
                        Some(_) => 1,
                        None => 0,
                    };
                    (arr_len, "variable_iteration".to_string(), None, None)
                } else {
                    // It's a collection - check for potential index usage
                    let collection = self.get_collection(&for_clause.collection)?;
                    let doc_count = collection.count();
                    total_docs_scanned += doc_count;

                    // Check if any filter can use an index
                    let mut found_index: Option<(String, String)> = None;
                    for filter in &query.filter_clauses {
                        if let Some(condition) = self
                            .extract_indexable_condition(&filter.expression, &for_clause.variable)
                        {
                            let indexes = collection.list_indexes();
                            for idx in &indexes {
                                if idx.field == condition.field {
                                    found_index =
                                        Some((idx.name.clone(), format!("{:?}", idx.index_type)));
                                    break;
                                }
                            }
                        }
                    }

                    if found_index.is_none() && doc_count > 100 {
                        warnings.push(format!(
                        "Full collection scan on '{}' ({} documents). Consider adding an index.",
                        for_clause.collection, doc_count
                    ));
                    }

                    let access = if found_index.is_some() {
                        "index_lookup"
                    } else {
                        "full_scan"
                    };
                    (
                        doc_count,
                        access.to_string(),
                        found_index.as_ref().map(|(n, _)| n.clone()),
                        found_index.map(|(_, t)| t),
                    )
                };

            collections_info.push(CollectionAccess {
                name: for_clause.collection.clone(),
                variable: for_clause.variable.clone(),
                access_type,
                index_used,
                index_type,
                documents_count: docs_count,
            });
        }

        // Execute query using optimized path (with index support)
        let scan_start = Instant::now();
        let mut rows = if !query.body_clauses.is_empty() {
            // Use optimized path with index support
            // Don't pass scan_limit to explain - we want to see full execution
            let (r, _) = self.execute_body_clauses(&query.body_clauses, &let_bindings, None)?;
            r
        } else {
            // Legacy path for old queries
            self.build_row_combinations_with_context(&query.for_clauses, &let_bindings)?
        };
        let collection_scan_time = scan_start.elapsed();
        let rows_after_scan = rows.len();

        // Note: Filters are already applied in execute_body_clauses, but we need to analyze them
        // So we'll extract filter info from body_clauses
        let filter_start = Instant::now();

        if !query.body_clauses.is_empty() {
            // Filters were already applied in execute_body_clauses
            // Extract filter info from body_clauses for reporting
            for clause in &query.body_clauses {
                if let BodyClause::Filter(filter) = clause {
                    // Try to find index candidate for this filter
                    let mut index_candidate = None;
                    let mut can_use_index = false;

                    if !query.for_clauses.is_empty() {
                        let var_name = &query.for_clauses[0].variable;
                        if let Some(condition) =
                            self.extract_indexable_condition(&filter.expression, var_name)
                        {
                            index_candidate = Some(condition.field.clone());
                            // Check if index exists
                            if let Ok(collection) =
                                self.get_collection(&query.for_clauses[0].collection)
                            {
                                for idx in collection.list_indexes() {
                                    if idx.field == condition.field {
                                        can_use_index = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }

                    filters_info.push(FilterInfo {
                        expression: format!("{:?}", filter.expression),
                        index_candidate,
                        can_use_index,
                        documents_before: total_docs_scanned,
                        documents_after: rows.len(),
                        time_us: 0, // Timing included in collection_scan_time
                    });
                }
            }
        } else {
            // Legacy path: Apply and analyze FILTER clauses
            for filter in &query.filter_clauses {
                let before_count = rows.len();
                let clause_start = Instant::now();

                rows.retain(|ctx| {
                    self.evaluate_filter_with_context(&filter.expression, ctx)
                        .unwrap_or(false)
                });

                let clause_time = clause_start.elapsed();
                let after_count = rows.len();

                // Try to find index candidate for this filter
                let mut index_candidate = None;
                let mut can_use_index = false;

                if !query.for_clauses.is_empty() {
                    let var_name = &query.for_clauses[0].variable;
                    if let Some(condition) =
                        self.extract_indexable_condition(&filter.expression, var_name)
                    {
                        index_candidate = Some(condition.field.clone());
                        // Check if index exists
                        if let Ok(collection) =
                            self.get_collection(&query.for_clauses[0].collection)
                        {
                            for idx in collection.list_indexes() {
                                if idx.field == condition.field {
                                    can_use_index = true;
                                    break;
                                }
                            }
                        }
                    }
                }

                filters_info.push(FilterInfo {
                    expression: format!("{:?}", filter.expression),
                    index_candidate,
                    can_use_index,
                    documents_before: before_count,
                    documents_after: after_count,
                    time_us: clause_time.as_micros() as u64,
                });
            }
        }
        let filter_time = filter_start.elapsed();

        // Apply SORT
        let sort_start = Instant::now();
        let sort_info = if let Some(sort) = &query.sort_clause {
            rows.sort_by(|a, b| {
                for (expr, ascending) in &sort.fields {
                    let a_val = self
                        .evaluate_expr_with_context(expr, a)
                        .unwrap_or(Value::Null);
                    let b_val = self
                        .evaluate_expr_with_context(expr, b)
                        .unwrap_or(Value::Null);

                    let cmp = compare_values(&a_val, &b_val);
                    if cmp != std::cmp::Ordering::Equal {
                        return if *ascending {
                            cmp
                        } else {
                            cmp.reverse()
                        };
                    }
                }
                std::cmp::Ordering::Equal
            });

            let field_desc = sort.fields.iter()
                 .map(|(e, asc)| format!("{} {}", format_expression(e), if *asc { "ASC" } else { "DESC" }))
                 .collect::<Vec<_>>()
                 .join(", ");

            Some(SortInfo {
                field: field_desc,
                direction: "".to_string(), // Direction included in field description

                time_us: 0, // Will be set below
            })
        } else {
            None
        };
        let sort_time = sort_start.elapsed();

        let sort_info = sort_info.map(|mut s| {
            s.time_us = sort_time.as_micros() as u64;
            s
        });

        // Apply LIMIT
        let limit_start = Instant::now();
        let limit_info = if let Some(limit) = &query.limit_clause {
            let offset = self.evaluate_expr_with_context(&limit.offset, &let_bindings)
                .ok().and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(0);
            let count = self.evaluate_expr_with_context(&limit.count, &let_bindings)
                .ok().and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(0);

            let start = offset.min(rows.len());
            let end = (start + count).min(rows.len());
            rows = rows[start..end].to_vec();

            Some(LimitInfo {
                offset,
                count,
            })
        } else {
            None
        };
        let limit_time = limit_start.elapsed();

        // Apply RETURN projection (if present)
        let return_start = Instant::now();
        let results = if let Some(ref return_clause) = query.return_clause {
            let results: DbResult<Vec<Value>> = rows
                .iter()
                .map(|ctx| self.evaluate_expr_with_context(&return_clause.expression, ctx))
                .collect();
            results?
        } else {
            vec![]
        };
        let return_time = return_start.elapsed();

        let total_time = total_start.elapsed();

        // Add warnings for slow operations
        if filter_time.as_millis() > 100 {
            warnings.push(format!(
                "Filter operations took {}ms. Consider adding indexes on filtered fields.",
                filter_time.as_millis()
            ));
        }

        if sort_time.as_millis() > 100 && rows_after_scan > 1000 {
            warnings.push(format!(
                "Sort operation on {} rows took {}ms. Consider adding a persistent index for sorting.",
                rows_after_scan, sort_time.as_millis()
            ));
        }

        Ok(QueryExplain {
            collections: collections_info,
            let_bindings: let_bindings_info,
            filters: filters_info,
            sort: sort_info,
            limit: limit_info,
            timing: ExecutionTiming {
                total_us: total_time.as_micros() as u64,
                let_clauses_us: let_clauses_time.as_micros() as u64,
                collection_scan_us: collection_scan_time.as_micros() as u64,
                filter_us: filter_time.as_micros() as u64,
                sort_us: sort_time.as_micros() as u64,
                limit_us: limit_time.as_micros() as u64,
                return_projection_us: return_time.as_micros() as u64,
            },
            documents_scanned: total_docs_scanned,
            documents_returned: results.len(),
            warnings,
        })
    }

    /// Build all row combinations from multiple FOR clauses
    /// This creates the Cartesian product for JOINs


    /// Build all row combinations from multiple FOR clauses with initial context (LET bindings)
    /// This creates the Cartesian product for JOINs
    fn build_row_combinations_with_context(
        &self,
        for_clauses: &[ForClause],
        let_bindings: &Context,
    ) -> DbResult<Vec<Context>> {
        if for_clauses.is_empty() {
            // If no FOR clauses but we have LET bindings, return single row with bindings
            if !let_bindings.is_empty() {
                return Ok(vec![let_bindings.clone()]);
            }
            return Ok(vec![HashMap::new()]);
        }

        // Start with LET bindings as initial context
        let mut result: Vec<Context> = vec![let_bindings.clone()];

        for for_clause in for_clauses {
            let source_name = for_clause
                .source_variable
                .as_ref()
                .unwrap_or(&for_clause.collection);

            // First check if source is a LET variable (array)
            let docs: Vec<Value> = if let Some(let_value) = let_bindings.get(source_name) {
                // Source is a LET variable - should be an array
                match let_value {
                    Value::Array(arr) => arr.clone(),
                    // If it's a single value, wrap it in an array
                    other => vec![other.clone()],
                }
            } else {
                // Source is a collection name
                let collection = self.get_collection(&for_clause.collection)?;
                collection.all().iter().map(|d| d.to_value()).collect()
            };

            let var_name = &for_clause.variable;

            // Cross product: for each existing row, create new rows with each doc
            let mut new_result = Vec::with_capacity(result.len() * docs.len());

            for existing_ctx in &result {
                for doc in &docs {
                    let mut new_ctx = existing_ctx.clone();
                    new_ctx.insert(var_name.clone(), doc.clone());
                    new_result.push(new_ctx);
                }
            }

            result = new_result;
        }

        Ok(result)
    }

    /// Evaluate a filter expression with full context
    pub fn evaluate_filter_with_context(&self, expr: &Expression, ctx: &Context) -> DbResult<bool> {
        match self.evaluate_expr_with_context(expr, ctx)? {
            Value::Bool(b) => Ok(b),
            _ => Ok(false),
        }
    }

    /// Evaluate an expression with a context containing multiple variables
    pub fn evaluate_expr_with_context(&self, expr: &Expression, ctx: &Context) -> DbResult<Value> {
        match expr {
            Expression::Variable(name) => ctx
                .get(name)
                .cloned()
                .ok_or_else(|| DbError::ExecutionError(format!("Variable '{}' not found", name))),

            Expression::BindVariable(name) => {
                // First check context (bind vars are stored with @ prefix)
                if let Some(value) = ctx.get(&format!("@{}", name)) {
                    return Ok(value.clone());
                }
                // Then check bind_vars directly
                self.bind_vars.get(name).cloned().ok_or_else(|| {
                    DbError::ExecutionError(format!(
                        "Bind variable '@{}' not found. Did you forget to pass it in bindVars?",
                        name
                    ))
                })
            }

            Expression::FieldAccess(base, field) => {
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                Ok(get_field_value(&base_value, field))
            }

            Expression::DynamicFieldAccess(base, field_expr) => {
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                let field_value = self.evaluate_expr_with_context(field_expr, ctx)?;

                // The field expression should evaluate to a string (field name)
                let field_name = match field_value {
                    Value::String(s) => s,
                    Value::Number(n) => n.to_string(),
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "Dynamic field access requires a string or number, got: {:?}",
                            field_value
                        )))
                    }
                };

                Ok(get_field_value(&base_value, &field_name))
            }

            Expression::ArrayAccess(base, index_expr) => {
                let base_value = self.evaluate_expr_with_context(base, ctx)?;
                let index_value = self.evaluate_expr_with_context(index_expr, ctx)?;

                // The index should be a number
                let index = match index_value {
                    Value::Number(n) => {
                        // Handle both integer and float numbers
                        if let Some(i) = n.as_u64() {
                            i as usize
                        } else if let Some(f) = n.as_f64() {
                            // Convert float to integer (truncate)
                            if f < 0.0 {
                                return Err(DbError::ExecutionError(format!(
                                    "Array index must be non-negative, got: {}",
                                    f
                                )));
                            }
                            f as usize
                        } else {
                            return Err(DbError::ExecutionError(format!(
                                "Invalid array index: {}",
                                n
                            )));
                        }
                    }
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "Array index must be a number, got: {:?}",
                            index_value
                        )))
                    }
                };

                // Access the array element
                match base_value {
                    Value::Array(ref arr) => Ok(arr.get(index).cloned().unwrap_or(Value::Null)),
                    _ => Ok(Value::Null), // Non-arrays return null
                }
            }

            Expression::Literal(value) => Ok(value.clone()),

            Expression::BinaryOp { left, op, right } => {
                match op {
                    BinaryOperator::And => {
                        let left_val = self.evaluate_expr_with_context(left, ctx)?;
                        if !to_bool(&left_val) {
                            return Ok(Value::Bool(false));
                        }
                        let right_val = self.evaluate_expr_with_context(right, ctx)?;
                        Ok(Value::Bool(to_bool(&right_val)))
                    }
                    BinaryOperator::Or => {
                        let left_val = self.evaluate_expr_with_context(left, ctx)?;
                        if to_bool(&left_val) {
                            return Ok(Value::Bool(true));
                        }
                        let right_val = self.evaluate_expr_with_context(right, ctx)?;
                        Ok(Value::Bool(to_bool(&right_val)))
                    }
                    _ => {
                        let left_val = self.evaluate_expr_with_context(left, ctx)?;
                        let right_val = self.evaluate_expr_with_context(right, ctx)?;
                        evaluate_binary_op(&left_val, op, &right_val)
                    }
                }
            }

            Expression::UnaryOp { op, operand } => {
                let val = self.evaluate_expr_with_context(operand, ctx)?;
                evaluate_unary_op(op, &val)
            }

            Expression::Object(fields) => {
                let mut obj = serde_json::Map::with_capacity(fields.len());
                for (key, value_expr) in fields {
                    let value = self.evaluate_expr_with_context(value_expr, ctx)?;
                    obj.insert(key.clone(), value);
                }
                Ok(Value::Object(obj))
            }

            Expression::Array(elements) => {
                let mut arr = Vec::with_capacity(elements.len());
                for elem in elements {
                    arr.push(self.evaluate_expr_with_context(elem, ctx)?);
                }
                Ok(Value::Array(arr))
            }

            Expression::Range(start_expr, end_expr) => {
                let start_val = self.evaluate_expr_with_context(start_expr, ctx)?;
                let end_val = self.evaluate_expr_with_context(end_expr, ctx)?;

                let start = match &start_val {
                    Value::Number(n) => {
                        // Try integer first, then fall back to truncating float
                        n.as_i64()
                            .or_else(|| n.as_f64().map(|f| f as i64))
                            .ok_or_else(|| {
                                DbError::ExecutionError("Range start must be a number".to_string())
                            })?
                    }
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "Range start must be a number, got: {:?}",
                            start_val
                        )))
                    }
                };

                let end = match &end_val {
                    Value::Number(n) => {
                        // Try integer first, then fall back to truncating float
                        n.as_i64()
                            .or_else(|| n.as_f64().map(|f| f as i64))
                            .ok_or_else(|| {
                                DbError::ExecutionError("Range end must be a number".to_string())
                            })?
                    }
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "Range end must be a number, got: {:?}",
                            end_val
                        )))
                    }
                };

                // Generate array from start to end (inclusive)
                let arr: Vec<Value> = (start..=end)
                    .map(|i| Value::Number(serde_json::Number::from(i)))
                    .collect();

                Ok(Value::Array(arr))
            }

            Expression::FunctionCall { name, args } => self.evaluate_function(name, args, ctx),

            Expression::Subquery(subquery) => {
                // Execute the subquery with parent context (enables correlated subqueries)
                let results = self.execute_with_parent_context(subquery, ctx)?;
                Ok(Value::Array(results))
            }

            Expression::Ternary {
                condition,
                true_expr,
                false_expr,
            } => {
                let cond_val = self.evaluate_expr_with_context(condition, ctx)?;
                if to_bool(&cond_val) {
                    self.evaluate_expr_with_context(true_expr, ctx)
                } else {
                    self.evaluate_expr_with_context(false_expr, ctx)
                }
            }
        }
    }

    /// Execute a subquery with access to parent context (for correlated subqueries)
    fn execute_with_parent_context(
        &self,
        query: &Query,
        parent_ctx: &Context,
    ) -> DbResult<Vec<Value>> {
        // Start with parent context (enables correlation with outer query)
        let mut initial_bindings = parent_ctx.clone();

        // Add bind variables
        for (key, value) in &self.bind_vars {
            initial_bindings.insert(format!("@{}", key), value.clone());
        }

        // Evaluate initial LET clauses (before FOR)
        for let_clause in &query.let_clauses {
            let value =
                self.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
            initial_bindings.insert(let_clause.variable.clone(), value);
        }

        // Process body_clauses in order
        let rows = if !query.body_clauses.is_empty() {
            let (r, _) = self.execute_body_clauses(&query.body_clauses, &initial_bindings, None)?;
            r
        } else {
            let mut rows =
                self.build_row_combinations_with_context(&query.for_clauses, &initial_bindings)?;
            for filter in &query.filter_clauses {
                rows.retain(|ctx| {
                    self.evaluate_filter_with_context(&filter.expression, ctx)
                        .unwrap_or(false)
                });
            }
            rows
        };

        let mut rows = rows;

        // Apply SORT
        if let Some(sort) = &query.sort_clause {
            rows.sort_by(|a, b| {
                for (expr, ascending) in &sort.fields {
                    let a_val = self
                        .evaluate_expr_with_context(expr, a)
                        .unwrap_or(Value::Null);
                    let b_val = self
                        .evaluate_expr_with_context(expr, b)
                        .unwrap_or(Value::Null);

                    let cmp = compare_values(&a_val, &b_val);
                    if cmp != std::cmp::Ordering::Equal {
                        return if *ascending {
                            cmp
                        } else {
                            cmp.reverse()
                        };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }

        // Apply LIMIT
        if let Some(limit) = &query.limit_clause {
            let offset = self.evaluate_expr_with_context(&limit.offset, &initial_bindings)
                .ok().and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(0);
            let count = self.evaluate_expr_with_context(&limit.count, &initial_bindings)
                .ok().and_then(|v| v.as_u64()).map(|n| n as usize).unwrap_or(0);

            let start = offset.min(rows.len());
            let end = (start + count).min(rows.len());
            rows = rows[start..end].to_vec();
        }

        // Apply RETURN projection (if present)
        if let Some(ref return_clause) = query.return_clause {
            let results: DbResult<Vec<Value>> = rows
                .iter()
                .map(|ctx| self.evaluate_expr_with_context(&return_clause.expression, ctx))
                .collect();
            results
        } else {
            Ok(vec![])
        }
    }

    /// Evaluate a function call
    fn evaluate_function(&self, name: &str, args: &[Expression], ctx: &Context) -> DbResult<Value> {
        // Evaluate all arguments
        let evaluated_args: Vec<Value> = args
            .iter()
            .map(|arg| self.evaluate_expr_with_context(arg, ctx))
            .collect::<DbResult<Vec<_>>>()?;

        match name.to_uppercase().as_str() {
            // IF(condition, true_val, false_val) - conditional evaluation
            "IF" | "IIF" => {
                if evaluated_args.len() != 3 {
                    return Err(DbError::ExecutionError(
                        "IF requires 3 arguments: condition, true_value, false_value".to_string(),
                    ));
                }
                if to_bool(&evaluated_args[0]) {
                    Ok(evaluated_args[1].clone())
                } else {
                    Ok(evaluated_args[2].clone())
                }
            }

            // Type checking functions
            "IS_ARRAY" | "IS_LIST" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_ARRAY requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::Array(_))))
            }

            "IS_BOOL" | "IS_BOOLEAN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_BOOLEAN requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::Bool(_))))
            }

            "IS_NUMBER" | "IS_NUMERIC" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_NUMBER requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::Number(_))))
            }

            "IS_INTEGER" | "IS_INT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_INTEGER requires 1 argument".to_string(),
                    ));
                }
                let is_int = match &evaluated_args[0] {
                    Value::Number(n) => {
                        // Check if it's an integer (no decimal part)
                        if n.as_i64().is_some() {
                            true
                        } else if let Some(f) = n.as_f64() {
                            f.fract() == 0.0 && f.is_finite()
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                Ok(Value::Bool(is_int))
            }

            "IS_STRING" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_STRING requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::String(_))))
            }

            "IS_OBJECT" | "IS_DOCUMENT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_OBJECT requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::Object(_))))
            }

            "IS_NULL" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_NULL requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::Null)))
            }

            "IS_DATETIME" | "IS_DATESTRING" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_DATETIME requires 1 argument".to_string(),
                    ));
                }
                let is_datetime = match &evaluated_args[0] {
                    Value::String(s) => {
                        // Try to parse as ISO 8601 datetime
                        chrono::DateTime::parse_from_rfc3339(s).is_ok()
                    }
                    Value::Number(n) => {
                        // Could be a Unix timestamp - check if it's a reasonable timestamp value
                        if let Some(ts) = n.as_i64() {
                            // Valid timestamp range: 1970-01-01 to 3000-01-01 approximately
                            ts >= 0 && ts < 32503680000000 // Year 3000 in milliseconds
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                Ok(Value::Bool(is_datetime))
            }

            "TYPENAME" | "TYPE_OF" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "TYPENAME requires 1 argument".to_string(),
                    ));
                }
                let type_name = match &evaluated_args[0] {
                    Value::Null => "null",
                    Value::Bool(_) => "bool",
                    Value::Number(n) => {
                        if n.is_i64() || n.is_u64() {
                            "int"
                        } else {
                            "number"
                        }
                    }
                    Value::String(_) => "string",
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                };
                Ok(Value::String(type_name.to_string()))
            }

            // TIME_BUCKET(timestamp, interval) - bucket timestamp into fixed intervals
            "TIME_BUCKET" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "TIME_BUCKET requires 2 arguments: timestamp, interval (e.g. '5m')".to_string(),
                    ));
                }

                // Parse interval
                let interval_str = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("TIME_BUCKET: interval must be a string".to_string())
                })?;

                let len = interval_str.len();
                if len < 2 {
                     return Err(DbError::ExecutionError("TIME_BUCKET: invalid interval format".to_string()));
                }

                let unit = &interval_str[len-1..];
                let val_str = &interval_str[..len-1];
                let val: u64 = val_str.parse().map_err(|_| {
                    DbError::ExecutionError("TIME_BUCKET: invalid interval number".to_string())
                })?;

                let interval_ms = match unit {
                    "s" => val * 1000,
                    "m" => val * 1000 * 60,
                    "h" => val * 1000 * 60 * 60,
                    "d" => val * 1000 * 60 * 60 * 24,
                    _ => return Err(DbError::ExecutionError("TIME_BUCKET: valid units are s, m, h, d".to_string())),
                };

                if interval_ms == 0 {
                    return Err(DbError::ExecutionError("TIME_BUCKET: interval cannot be 0".to_string()));
                }

                // Parse timestamp
                match &evaluated_args[0] {
                    Value::Number(n) => {
                        let ts = n.as_i64().ok_or_else(|| {
                            DbError::ExecutionError("TIME_BUCKET: timestamp must be a valid number".to_string())
                        })?;
                        // Bucket (use div_euclid to handle negative timestamps correctly)
                        let bucket = ts.div_euclid(interval_ms as i64) * (interval_ms as i64);
                        Ok(Value::Number(bucket.into()))
                    },
                    Value::String(s) => {
                         let dt = chrono::DateTime::parse_from_rfc3339(s).map_err(|_| {
                             DbError::ExecutionError("TIME_BUCKET: invalid timestamp string".to_string())
                         })?;
                         let ts = dt.timestamp_millis();
                         let bucket_ts = ts.div_euclid(interval_ms as i64) * (interval_ms as i64);
                         
                         // Convert back to string (UTC)
                         // We use basic arithmetic to get seconds/nanos for safe reconstruction
                         let seconds = bucket_ts.div_euclid(1000);
                         let nanos = (bucket_ts.rem_euclid(1000) * 1_000_000) as u32;
                         
                         // Try standard DateTime construction (compatible with most chrono versions)
                         // We rely on Utc being available
                         if let Some(dt) = chrono::DateTime::from_timestamp(seconds, nanos) {
                             Ok(Value::String(dt.to_rfc3339()))
                         } else {
                             // Fallback or error path
                             Err(DbError::ExecutionError("TIME_BUCKET: failed to construct date".to_string()))
                         }
                    },
                    _ => Err(DbError::ExecutionError("TIME_BUCKET: timestamp must be number or string".to_string()))
                }
            }

            // DISTANCE(lat1, lon1, lat2, lon2) - distance between two points in meters
            "DISTANCE" => {
                if evaluated_args.len() != 4 {
                    return Err(DbError::ExecutionError(
                        "DISTANCE requires 4 arguments: lat1, lon1, lat2, lon2".to_string(),
                    ));
                }
                let lat1 = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("DISTANCE: lat1 must be a number".to_string())
                })?;
                let lon1 = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("DISTANCE: lon1 must be a number".to_string())
                })?;
                let lat2 = evaluated_args[2].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("DISTANCE: lat2 must be a number".to_string())
                })?;
                let lon2 = evaluated_args[3].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("DISTANCE: lon2 must be a number".to_string())
                })?;

                let dist = distance_meters(lat1, lon1, lat2, lon2);
                Ok(Value::Number(number_from_f64(dist)))
            }

            // GEO_DISTANCE(geopoint1, geopoint2) - distance between two geo points
            "GEO_DISTANCE" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "GEO_DISTANCE requires 2 arguments: point1, point2".to_string(),
                    ));
                }
                let p1 = GeoPoint::from_value(&evaluated_args[0]).ok_or_else(|| {
                    DbError::ExecutionError(
                        "GEO_DISTANCE: first argument must be a geo point".to_string(),
                    )
                })?;
                let p2 = GeoPoint::from_value(&evaluated_args[1]).ok_or_else(|| {
                    DbError::ExecutionError(
                        "GEO_DISTANCE: second argument must be a geo point".to_string(),
                    )
                })?;

                let dist = distance_meters(p1.lat, p1.lon, p2.lat, p2.lon);
                Ok(Value::Number(number_from_f64(dist)))
            }

            // HAS(doc, attribute) - check if document has attribute
            "HAS" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "HAS requires 2 arguments: document, attribute".to_string(),
                    ));
                }

                let doc = evaluated_args[0].as_object().ok_or_else(|| {
                    DbError::ExecutionError(
                        "HAS: first argument must be a document/object".to_string(),
                    )
                })?;

                let attr_name = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("HAS: second argument must be a string".to_string())
                })?;

                Ok(Value::Bool(doc.contains_key(attr_name)))
            }

            // KEEP(doc, attr1, attr2, ...) OR KEEP(doc, [attr1, attr2, ...])
            "KEEP" => {
                if evaluated_args.len() < 2 {
                    return Err(DbError::ExecutionError(
                        "KEEP requires at least 2 arguments: document, attributes...".to_string(),
                    ));
                }

                let doc = evaluated_args[0].as_object().ok_or_else(|| {
                    DbError::ExecutionError(
                        "KEEP: first argument must be a document/object".to_string(),
                    )
                })?;

                let mut keys_to_keep = Vec::new();

                // Handle second argument as array or varargs
                if evaluated_args.len() == 2 && evaluated_args[1].is_array() {
                    let arr = evaluated_args[1].as_array().unwrap();
                    for val in arr {
                        if let Some(s) = val.as_str() {
                            keys_to_keep.push(s);
                        }
                    }
                } else {
                    for arg in &evaluated_args[1..] {
                        if let Some(s) = arg.as_str() {
                            keys_to_keep.push(s);
                        } else {
                            return Err(DbError::ExecutionError(
                                "KEEP: attribute names must be strings".to_string(),
                            ));
                        }
                    }
                }

                let mut new_doc = serde_json::Map::new();
                for key in keys_to_keep {
                    if let Some(val) = doc.get(key) {
                        new_doc.insert(key.to_string(), val.clone());
                    }
                }

                Ok(Value::Object(new_doc))
            }

            // LENGTH(array_or_string_or_collection) - get length of array/string or count of collection
            "LENGTH" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "LENGTH requires 1 argument".to_string(),
                    ));
                }
                let len = match &evaluated_args[0] {
                    Value::Array(arr) => arr.len(),
                    Value::String(s) => {
                        // First try to treat it as a collection name
                        match self.get_collection(s) {
                            Ok(collection) => collection.count(),
                            Err(_) => s.len(), // Fallback to string length if not a valid collection
                        }
                    }
                    Value::Object(obj) => obj.len(),
                    _ => {
                        return Err(DbError::ExecutionError(
                            "LENGTH: argument must be array, string, object, or collection name"
                                .to_string(),
                        ))
                    }
                };
                Ok(Value::Number(serde_json::Number::from(len)))
            }

            // SUM(array) - sum of numeric array elements
            "SUM" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "SUM requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("SUM: argument must be an array".to_string())
                })?;

                let sum: f64 = arr.iter().filter_map(|v| v.as_f64()).sum();

                Ok(Value::Number(
                    serde_json::Number::from_f64(sum).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // AVG(array) - average of numeric array elements
            "AVG" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "AVG requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("AVG: argument must be an array".to_string())
                })?;

                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                let avg = nums.iter().sum::<f64>() / nums.len() as f64;
                Ok(Value::Number(
                    serde_json::Number::from_f64(avg).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // MIN(array) - minimum value in array
            "MIN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "MIN requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("MIN: argument must be an array".to_string())
                })?;

                let min = arr
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                match min {
                    Some(n) => Ok(Value::Number(number_from_f64(n))),
                    None => Ok(Value::Null),
                }
            }

            // MAX(array) - maximum value in array
            "MAX" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "MAX requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("MAX: argument must be an array".to_string())
                })?;

                let max = arr
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                match max {
                    Some(n) => Ok(Value::Number(number_from_f64(n))),
                    None => Ok(Value::Null),
                }
            }

            // COUNT(array) - count elements in array
            "COUNT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "COUNT requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("COUNT: argument must be an array".to_string())
                })?;
                Ok(Value::Number(serde_json::Number::from(arr.len())))
            }

            // COUNT_DISTINCT(array) - count distinct values in array
            "COUNT_DISTINCT" | "COUNT_UNIQUE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "COUNT_DISTINCT requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("COUNT_DISTINCT: argument must be an array".to_string())
                })?;
                let unique: std::collections::HashSet<String> =
                    arr.iter().map(|v| v.to_string()).collect();
                Ok(Value::Number(serde_json::Number::from(unique.len())))
            }

            // VARIANCE_POPULATION(array) - population variance
            "VARIANCE_POPULATION" | "VARIANCE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "VARIANCE_POPULATION requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VARIANCE_POPULATION: argument must be an array".to_string(),
                    )
                })?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance =
                    nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                Ok(Value::Number(
                    serde_json::Number::from_f64(variance).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // VARIANCE_SAMPLE(array) - sample variance (n-1 denominator)
            "VARIANCE_SAMPLE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "VARIANCE_SAMPLE requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VARIANCE_SAMPLE: argument must be an array".to_string(),
                    )
                })?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.len() < 2 {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance =
                    nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64;
                Ok(Value::Number(
                    serde_json::Number::from_f64(variance).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // STDDEV_POPULATION(array) - population standard deviation
            "STDDEV_POPULATION" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "STDDEV_POPULATION requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "STDDEV_POPULATION: argument must be an array".to_string(),
                    )
                })?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance =
                    nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                let stddev = variance.sqrt();
                Ok(Value::Number(
                    serde_json::Number::from_f64(stddev).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // STDDEV_SAMPLE(array) / STDDEV(array) - sample standard deviation (n-1 denominator)
            "STDDEV_SAMPLE" | "STDDEV" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "STDDEV_SAMPLE requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("STDDEV_SAMPLE: argument must be an array".to_string())
                })?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.len() < 2 {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance =
                    nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64;
                let stddev = variance.sqrt();
                Ok(Value::Number(
                    serde_json::Number::from_f64(stddev).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // MEDIAN(array) - median value
            "MEDIAN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "MEDIAN requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("MEDIAN: argument must be an array".to_string())
                })?;
                let mut nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let len = nums.len();
                let median = if len % 2 == 0 {
                    (nums[len / 2 - 1] + nums[len / 2]) / 2.0
                } else {
                    nums[len / 2]
                };
                Ok(Value::Number(
                    serde_json::Number::from_f64(median).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // PERCENTILE(array, p) - percentile value (p between 0 and 100)
            "PERCENTILE" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "PERCENTILE requires 2 arguments: array, percentile (0-100)".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "PERCENTILE: first argument must be an array".to_string(),
                    )
                })?;
                let p = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError(
                        "PERCENTILE: second argument must be a number".to_string(),
                    )
                })?;
                if !(0.0..=100.0).contains(&p) {
                    return Err(DbError::ExecutionError(
                        "PERCENTILE: percentile must be between 0 and 100".to_string(),
                    ));
                }
                let mut nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let index = (p / 100.0) * (nums.len() - 1) as f64;
                let lower = index.floor() as usize;
                let upper = index.ceil() as usize;
                let result = if lower == upper {
                    nums[lower]
                } else {
                    let fraction = index - lower as f64;
                    nums[lower] * (1.0 - fraction) + nums[upper] * fraction
                };
                Ok(Value::Number(
                    serde_json::Number::from_f64(result).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // UNIQUE(array) - return unique values
            "UNIQUE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "UNIQUE requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("UNIQUE: argument must be an array".to_string())
                })?;
                let mut seen = std::collections::HashSet::new();
                let unique: Vec<Value> = arr
                    .iter()
                    .filter(|v| seen.insert(v.to_string()))
                    .cloned()
                    .collect();
                Ok(Value::Array(unique))
            }

            // SORTED(array) - sort array (ascending)
            "SORTED" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "SORTED requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("SORTED: argument must be an array".to_string())
                })?;
                let mut sorted = arr.clone();
                sorted.sort_by(|a, b| match (a, b) {
                    (Value::Number(n1), Value::Number(n2)) => n1
                        .as_f64()
                        .unwrap_or(0.0)
                        .partial_cmp(&n2.as_f64().unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal),
                    (Value::String(s1), Value::String(s2)) => s1.cmp(s2),
                    _ => a.to_string().cmp(&b.to_string()),
                });
                Ok(Value::Array(sorted))
            }

            // SORTED_UNIQUE(array) - sort and return unique values
            "SORTED_UNIQUE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "SORTED_UNIQUE requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("SORTED_UNIQUE: argument must be an array".to_string())
                })?;
                let mut seen = std::collections::HashSet::new();
                let mut unique: Vec<Value> = arr
                    .iter()
                    .filter(|v| seen.insert(v.to_string()))
                    .cloned()
                    .collect();
                unique.sort_by(|a, b| match (a, b) {
                    (Value::Number(n1), Value::Number(n2)) => n1
                        .as_f64()
                        .unwrap_or(0.0)
                        .partial_cmp(&n2.as_f64().unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal),
                    (Value::String(s1), Value::String(s2)) => s1.cmp(s2),
                    _ => a.to_string().cmp(&b.to_string()),
                });
                Ok(Value::Array(unique))
            }

            // REVERSE(array) - reverse array
            "REVERSE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "REVERSE requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("REVERSE: argument must be an array".to_string())
                })?;
                let mut reversed = arr.clone();
                reversed.reverse();
                Ok(Value::Array(reversed))
            }

            // FIRST(array) - first element
            "FIRST" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "FIRST requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("FIRST: argument must be an array".to_string())
                })?;
                Ok(arr.first().cloned().unwrap_or(Value::Null))
            }

            // LAST(array) - last element
            "LAST" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "LAST requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("LAST: argument must be an array".to_string())
                })?;
                Ok(arr.last().cloned().unwrap_or(Value::Null))
            }

            // NTH(array, index) - nth element (0-based)
            "NTH" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "NTH requires 2 arguments: array, index".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("NTH: first argument must be an array".to_string())
                })?;
                let index = if let Some(i) = evaluated_args[1].as_i64() {
                    i
                } else if let Some(f) = evaluated_args[1].as_f64() {
                    f as i64
                } else {
                    return Err(DbError::ExecutionError("NTH: second argument must be a number".to_string()));
                } as usize;
                Ok(arr.get(index).cloned().unwrap_or(Value::Null))
            }

            // SLICE(array, start, length?) - slice array
            "SLICE" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "SLICE requires 2-3 arguments: array, start, [length]".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("SLICE: first argument must be an array".to_string())
                })?;
                let start = evaluated_args[1].as_i64().ok_or_else(|| {
                    DbError::ExecutionError("SLICE: start must be an integer".to_string())
                })?;
                let start = if start < 0 {
                    (arr.len() as i64 + start).max(0) as usize
                } else {
                    start as usize
                };
                let length = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_u64().unwrap_or(arr.len() as u64) as usize
                } else {
                    arr.len().saturating_sub(start)
                };
                let end = (start + length).min(arr.len());
                Ok(Value::Array(arr[start..end].to_vec()))
            }

            // FLATTEN(array, depth?) - flatten nested arrays
            "FLATTEN" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "FLATTEN requires 1-2 arguments: array, [depth]".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("FLATTEN: first argument must be an array".to_string())
                })?;
                let depth = if evaluated_args.len() > 1 {
                    evaluated_args[1].as_u64().unwrap_or(1) as usize
                } else {
                    1
                };
                fn flatten_recursive(arr: &[Value], depth: usize) -> Vec<Value> {
                    let mut result = Vec::new();
                    for item in arr {
                        if let Value::Array(inner) = item {
                            if depth > 0 {
                                result.extend(flatten_recursive(inner, depth - 1));
                            } else {
                                result.push(item.clone());
                            }
                        } else {
                            result.push(item.clone());
                        }
                    }
                    result
                }
                Ok(Value::Array(flatten_recursive(arr, depth)))
            }

            // PUSH(array, element, unique?) - add element to array
            "PUSH" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "PUSH requires 2-3 arguments: array, element, [unique]".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("PUSH: first argument must be an array".to_string())
                })?;
                let element = &evaluated_args[1];
                let unique = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_bool().unwrap_or(false)
                } else {
                    false
                };
                let mut result = arr.clone();
                if unique {
                    if !result.iter().any(|v| v.to_string() == element.to_string()) {
                        result.push(element.clone());
                    }
                } else {
                    result.push(element.clone());
                }
                Ok(Value::Array(result))
            }

            // APPEND(array1, array2, unique?) - append arrays
            "APPEND" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "APPEND requires 2-3 arguments: array1, array2, [unique]".to_string(),
                    ));
                }
                let arr1 = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("APPEND: first argument must be an array".to_string())
                })?;
                let arr2 = evaluated_args[1].as_array().ok_or_else(|| {
                    DbError::ExecutionError("APPEND: second argument must be an array".to_string())
                })?;
                let unique = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_bool().unwrap_or(false)
                } else {
                    false
                };
                let mut result = arr1.clone();
                if unique {
                    let existing: std::collections::HashSet<String> =
                        result.iter().map(|v| v.to_string()).collect();
                    for item in arr2 {
                        if !existing.contains(&item.to_string()) {
                            result.push(item.clone());
                        }
                    }
                } else {
                    result.extend(arr2.iter().cloned());
                }
                Ok(Value::Array(result))
            }

            // ZIP(array1, array2) - zip two arrays into array of pairs
            "ZIP" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "ZIP requires 2 arguments: array1, array2".to_string(),
                    ));
                }
                let arr1 = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("ZIP: first argument must be an array".to_string())
                })?;
                let arr2 = evaluated_args[1].as_array().ok_or_else(|| {
                    DbError::ExecutionError("ZIP: second argument must be an array".to_string())
                })?;

                let len = std::cmp::min(arr1.len(), arr2.len());
                let mut result = Vec::with_capacity(len);

                for i in 0..len {
                    result.push(Value::Array(vec![arr1[i].clone(), arr2[i].clone()]));
                }
                Ok(Value::Array(result))
            }

            // REMOVE_VALUE(array, value, limit?) - remove value from array
            "REMOVE_VALUE" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "REMOVE_VALUE requires 2-3 arguments: array, value, [limit]".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "REMOVE_VALUE: first argument must be an array".to_string(),
                    )
                })?;
                let val_to_remove = &evaluated_args[1];

                // Optional limit: number of occurrences to remove (default: -1 = remove all)
                let limit = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_i64().unwrap_or(-1)
                } else {
                    -1
                };

                let mut result = Vec::new();
                let mut removed_count = 0;

                for item in arr {
                    if values_equal(item, val_to_remove) {
                        if limit != -1 && removed_count >= limit {
                            result.push(item.clone());
                        } else {
                            removed_count += 1;
                        }
                    } else {
                        result.push(item.clone());
                    }
                }
                Ok(Value::Array(result))
            }

            // ATTRIBUTES(doc, removeInternal?, sort?) - return top-level attribute keys
            "ATTRIBUTES" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "ATTRIBUTES requires at least 1 argument: document".to_string(),
                    ));
                }

                let doc = evaluated_args[0].as_object().ok_or_else(|| {
                    DbError::ExecutionError(
                        "ATTRIBUTES: first argument must be a document/object".to_string(),
                    )
                })?;

                let remove_internal = if evaluated_args.len() > 1 {
                    evaluated_args[1].as_bool().unwrap_or(false)
                } else {
                    false
                };

                let sort_keys = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_bool().unwrap_or(false)
                } else {
                    false
                };

                let mut keys: Vec<String> = doc
                    .keys()
                    .filter(|k| !remove_internal || !k.starts_with('_'))
                    .cloned()
                    .collect();

                if sort_keys {
                   keys.sort();
                }

                Ok(Value::Array(
                    keys.into_iter().map(Value::String).collect(),
                ))
            }

            // VALUES(doc, removeInternal?) - return top-level attribute values
            "VALUES" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "VALUES requires at least 1 argument: document".to_string(),
                    ));
                }

                let doc = evaluated_args[0].as_object().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VALUES: first argument must be a document/object".to_string(),
                    )
                })?;

                let remove_internal = if evaluated_args.len() > 1 {
                    evaluated_args[1].as_bool().unwrap_or(false)
                } else {
                    false
                };

                let values: Vec<Value> = doc
                    .iter()
                    .filter(|(k, _)| !remove_internal || !k.starts_with('_'))
                    .map(|(_, v)| v.clone())
                    .collect();

                Ok(Value::Array(values))
            }

            // UNSET(doc, attr1, attr2, ...) OR UNSET(doc, [attr1, attr2, ...])
            "UNSET" => {
                if evaluated_args.len() < 2 {
                    return Err(DbError::ExecutionError(
                        "UNSET requires at least 2 arguments: document, attributes...".to_string(),
                    ));
                }

                let doc = evaluated_args[0].as_object().ok_or_else(|| {
                    DbError::ExecutionError(
                        "UNSET: first argument must be a document/object".to_string(),
                    )
                })?;

                let mut keys_to_unset = std::collections::HashSet::new();

                // Handle second argument as array or varargs
                if evaluated_args.len() == 2 && evaluated_args[1].is_array() {
                    let arr = evaluated_args[1].as_array().unwrap();
                    for val in arr {
                        if let Some(s) = val.as_str() {
                            keys_to_unset.insert(s);
                        }
                    }
                } else {
                    for arg in &evaluated_args[1..] {
                        if let Some(s) = arg.as_str() {
                            keys_to_unset.insert(s);
                        } else {
                            // ArangoDB UNSET ignores non-string arguments for keys, so we just skip them
                            // but existing KEEP implementation errors. Let's error to be safe/consistent with KEEP for now or be lenient.
                            // Docs say: "All other arguments... are attribute names". If not string?
                            // Usually SDBQL functions are permissive. But KEEP errors.
                            // Let's mirror KEEP behavior but maybe loosen it if needed.
                            // However, strictly following KEEP pattern:
                             return Err(DbError::ExecutionError(
                                "UNSET: attribute names must be strings".to_string(),
                            ));
                        }
                    }
                }

                let mut new_doc = serde_json::Map::new();
                for (key, val) in doc {
                    if !keys_to_unset.contains(key.as_str()) {
                        new_doc.insert(key.clone(), val.clone());
                    }
                }

                Ok(Value::Object(new_doc))
            }

            // REGEX_REPLACE(text, search, replacement, caseInsensitive?)
            "REGEX_REPLACE" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "REGEX_REPLACE requires 3-4 arguments: text, search, replacement, [caseInsensitive]"
                            .to_string(),
                    ));
                }

                let text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "REGEX_REPLACE: first argument must be a string".to_string(),
                    )
                })?;

                let search_pattern = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "REGEX_REPLACE: second argument must be a string (regex)".to_string(),
                    )
                })?;

                let replacement = evaluated_args[2].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "REGEX_REPLACE: third argument must be a string".to_string(),
                    )
                })?;

                let case_insensitive = if evaluated_args.len() > 3 {
                    evaluated_args[3].as_bool().unwrap_or(false)
                } else {
                    false
                };

                let pattern = if case_insensitive {
                    format!("(?i){}", search_pattern)
                } else {
                    search_pattern.to_string()
                };

                let re = regex::Regex::new(&pattern).map_err(|e| {
                    DbError::ExecutionError(format!("REGEX_REPLACE: invalid regex: {}", e))
                })?;

                let result = re.replace_all(text, replacement).to_string();
                Ok(Value::String(result))
            }

            // CONTAINS(text, search, returnIndex?)
            "CONTAINS" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "CONTAINS requires 2-3 arguments: text, search, [returnIndex]".to_string(),
                    ));
                }

                let text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("CONTAINS: first argument must be a string".to_string())
                })?;

                let search = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("CONTAINS: second argument must be a string".to_string())
                })?;

                let return_index = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_bool().unwrap_or(false)
                } else {
                    false
                };

                if return_index {
                    match text.find(search) {
                        Some(index) => Ok(Value::Number(serde_json::Number::from(index))),
                        None => Ok(Value::Number(serde_json::Number::from(-1))),
                    }
                } else {
                    Ok(Value::Bool(text.contains(search)))
                }
            }

            // SUBSTITUTE(value, search, replace, limit?) OR SUBSTITUTE(value, mapping, limit?)
            "SUBSTITUTE" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "SUBSTITUTE requires 2-4 arguments".to_string(),
                    ));
                }

                let text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "SUBSTITUTE: first argument must be a string".to_string(),
                    )
                })?;

                let limit = if evaluated_args[1].is_object() {
                    // Mapping mode: SUBSTITUTE(value, mapping, limit?)
                    if evaluated_args.len() > 3 {
                        return Err(DbError::ExecutionError(
                            "SUBSTITUTE with mapping requires 2-3 arguments".to_string(),
                        ));
                    }
                    if evaluated_args.len() == 3 {
                         evaluated_args[2].as_i64().or_else(|| evaluated_args[2].as_f64().map(|f| f as i64))
                    } else {
                        None
                    }
                } else {
                     // Replace mode: SUBSTITUTE(value, search, replace, limit?)
                     if evaluated_args.len() < 3 {
                        return Err(DbError::ExecutionError(
                            "SUBSTITUTE requires search and replace strings".to_string(),
                        ));
                     }
                      if evaluated_args.len() == 4 {
                        evaluated_args[3].as_i64().or_else(|| evaluated_args[3].as_f64().map(|f| f as i64))
                     } else {
                        None
                     }
                };

                let count_limit = match limit {
                    Some(n) if n > 0 => Some(n as usize),
                    Some(_) => Some(0), // 0 or negative limit means 0 replacements? Actually ArangoDB might handle 0 as replace nothing? Or all? Docs say "optional limit to restrict the number of replacements". Usually 0 means 0.
                    None => None, // None means replace all
                };

                // Perform substitution
                if evaluated_args[1].is_object() {
                    let mapping = evaluated_args[1].as_object().unwrap();
                    // For mapping, we need to be careful about overlapping replacements.
                    // Simple approach: multiple passes? No, usually single pass.
                    // But standard approach for simple implementation: iterate over mapping keys.
                    // Note: order is not guaranteed in JSON object. ArangoDB docs say "If mapping is used, the order of the attributes is undefined."
                    // So iterative replacement is acceptable even if order varies.

                    let mut result = text.to_string();
                    let replacements_left = count_limit;

                     for (search, replace_val) in mapping {
                        let replace = replace_val.as_str().unwrap_or(""); // Treat non-string values as empty string or stringify? Docs say "mapping values are converted to strings".
                        let replace_str = if replace_val.is_string() {
                            replace.to_string()
                        } else {
                            replace_val.to_string()
                        };

                        if let Some(limit_val) = replacements_left {
                             if limit_val == 0 { break; }
                             // Rust's replacen doesn't return how many replaced.
                             // We might need to handle this manually if we want global limit across all keys.
                             // But wait, "limit" in mapping mode usually means "limit per search term" or "total replacements"?
                             // Arango docs: "limit argument can be used to restrict the number of replacements". It usually applies *per* operation or total?
                             // "length of the search and replace list must be equal".
                             // Let's assume global limit for now? Or per key?
                             // Actually, if using `replacen`, it's per key.
                             // Let's stick to simple iterative replacement.
                             result = result.replacen(search, &replace_str, limit_val);
                             // To correctly track total replacements we'd need a different approach.
                             // Given ArangoDB's undefined order for keys, maybe it doesn't matter much for complex cases.
                             // Let's assume the limit is applied per key for now as it's the simplest interpretation of iterative application.
                        } else {
                             result = result.replace(search, &replace_str);
                        }
                    }
                    Ok(Value::String(result))
                } else {
                    let search = evaluated_args[1].as_str().ok_or_else(|| {
                         DbError::ExecutionError("SUBSTITUTE: search argument must be a string".to_string())
                    })?;
                    let replace = evaluated_args[2].as_str().ok_or_else(|| {
                         DbError::ExecutionError("SUBSTITUTE: replace argument must be a string".to_string())
                    })?;

                    if let Some(n) = count_limit {
                        Ok(Value::String(text.replacen(search, replace, n)))
                    } else {
                        Ok(Value::String(text.replace(search, replace)))
                    }
                }
            }

            // SPLIT(value, separator, limit?)
            "SPLIT" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "SPLIT requires 2-3 arguments: value, separator, [limit]".to_string(),
                    ));
                }

                let value = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("SPLIT: first argument must be a string".to_string())
                })?;

                let separator = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("SPLIT: second argument must be a string".to_string())
                })?;

                let limit = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_i64().or_else(|| evaluated_args[2].as_f64().map(|f| f as i64))
                } else {
                    None
                };

                let parts: Vec<Value> = match limit {
                    Some(n) if n > 0 => {
                        // Split into at most n parts from left
                        value.splitn(n as usize, separator)
                             .map(|s| Value::String(s.to_string()))
                             .collect()
                    },
                    Some(n) if n < 0 => {
                         // Split into at most abs(n) parts from right
                         // rsplitn returns parts in reverse order, so we need to reverse them back
                         let mut p: Vec<Value> = value.rsplitn(n.abs() as usize, separator)
                             .map(|s| Value::String(s.to_string()))
                             .collect();
                         p.reverse();
                         p
                    },
                    _ => {
                        // Split all (limit 0 or None)
                        if separator.is_empty() {
                            value.chars().map(|c| Value::String(c.to_string())).collect()
                        } else {
                            value.split(separator)
                                 .map(|s| Value::String(s.to_string()))
                                 .collect()
                        }
                    }
                };

                Ok(Value::Array(parts))
            }

            // TRIM(value, type_or_chars?)
            "TRIM" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                     return Err(DbError::ExecutionError(
                        "TRIM requires 1-2 arguments: value, [type/chars]".to_string(),
                    ));
                }
                let value = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("TRIM: first argument must be a string".to_string())
                })?;

                let (trim_mode, chars) = if evaluated_args.len() == 2 {
                    if evaluated_args[1].is_number() {
                        // Type: 0=both, 1=left, 2=right
                        let t = evaluated_args[1].as_i64().unwrap_or(0);
                        (Some(t), None)
                    } else if evaluated_args[1].is_string() {
                         // Chars
                         (None, evaluated_args[1].as_str())
                    } else {
                        // Invalid type
                        (Some(0), None) // Fallback or strict error? Arango ignores invalid? Let's assume strict or default.
                    }
                } else {
                    (Some(0), None)
                };

                let result = match (trim_mode, chars) {
                    (Some(0), None) => value.trim(),
                    (Some(1), None) => value.trim_start(),
                    (Some(2), None) => value.trim_end(),
                    (None, Some(c)) => value.trim_matches(|ch| c.contains(ch)),
                    _ => value.trim(), // Default
                };
                Ok(Value::String(result.to_string()))
            }

            // LTRIM(value, chars?)
            "LTRIM" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                     return Err(DbError::ExecutionError(
                        "LTRIM requires 1-2 arguments: value, [chars]".to_string(),
                    ));
                }
                let value = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("LTRIM: first argument must be a string".to_string())
                })?;

                let result = if evaluated_args.len() == 2 {
                     let chars = evaluated_args[1].as_str().ok_or_else(|| {
                        DbError::ExecutionError("LTRIM: second argument must be a string".to_string())
                     })?;
                     value.trim_start_matches(|ch| chars.contains(ch))
                } else {
                    value.trim_start()
                };
                Ok(Value::String(result.to_string()))
            }

            // RTRIM(value, chars?)
            "RTRIM" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                     return Err(DbError::ExecutionError(
                        "RTRIM requires 1-2 arguments: value, [chars]".to_string(),
                    ));
                }
                let value = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("RTRIM: first argument must be a string".to_string())
                })?;

                let result = if evaluated_args.len() == 2 {
                     let chars = evaluated_args[1].as_str().ok_or_else(|| {
                        DbError::ExecutionError("RTRIM: second argument must be a string".to_string())
                     })?;
                     value.trim_end_matches(|ch| chars.contains(ch))
                } else {
                    value.trim_end()
                };
                Ok(Value::String(result.to_string()))
            }

            // JSON_PARSE(text)
            "JSON_PARSE" => {
                if evaluated_args.len() != 1 {
                     return Err(DbError::ExecutionError(
                        "JSON_PARSE requires 1 argument: text".to_string(),
                    ));
                }
                let text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("JSON_PARSE: argument must be a string".to_string())
                })?;

                match serde_json::from_str::<Value>(text) {
                    Ok(v) => Ok(v),
                    Err(_) => Ok(Value::Null), // ArangoDB spec: invalid JSON returns NULL
                }
            }

            // JSON_STRINGIFY(value)
            "JSON_STRINGIFY" => {
                if evaluated_args.len() != 1 {
                     return Err(DbError::ExecutionError(
                        "JSON_STRINGIFY requires 1 argument: value".to_string(),
                    ));
                }
                match serde_json::to_string(&evaluated_args[0]) {
                    Ok(s) => Ok(Value::String(s)),
                    Err(_) => Ok(Value::Null),
                }
            }

            // UUIDV4()
            "UUIDV4" => {
                 if !evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "UUIDV4 requires 0 arguments".to_string(),
                    ));
                }
                Ok(Value::String(Uuid::new_v4().to_string()))
            }

            // UUIDV7()
            "UUIDV7" => {
                 if !evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "UUIDV7 requires 0 arguments".to_string(),
                    ));
                }
                Ok(Value::String(Uuid::now_v7().to_string()))
            }

            // MD5(string)
            "MD5" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("MD5 requires 1 argument".to_string()));
                }
                let input = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("MD5: argument must be a string".to_string())
                })?;
                let digest = md5::compute(input.as_bytes());
                Ok(Value::String(hex::encode(*digest)))
            }

            // SHA256(string)
            "SHA256" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("SHA256 requires 1 argument".to_string()));
                }
                let input = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("SHA256: argument must be a string".to_string())
                })?;
                use sha2::{Sha256, Digest};
                let mut hasher = Sha256::new();
                hasher.update(input.as_bytes());
                Ok(Value::String(hex::encode(hasher.finalize())))
            }

            // BASE64_ENCODE(string)
            "BASE64_ENCODE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("BASE64_ENCODE requires 1 argument".to_string()));
                }
                let input = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("BASE64_ENCODE: argument must be a string".to_string())
                })?;
                use base64::{Engine as _, engine::general_purpose};
                Ok(Value::String(general_purpose::STANDARD.encode(input)))
            }

            // BASE64_DECODE(string)
            "BASE64_DECODE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("BASE64_DECODE requires 1 argument".to_string()));
                }
                let input = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("BASE64_DECODE: argument must be a string".to_string())
                })?;
                use base64::{Engine as _, engine::general_purpose};
                match general_purpose::STANDARD.decode(input) {
                    Ok(bytes) => {
                        let s = String::from_utf8(bytes).map_err(|_| DbError::ExecutionError("BASE64_DECODE: result is not valid utf8".to_string()))?;
                        Ok(Value::String(s))
                    },
                    Err(_) => Err(DbError::ExecutionError("BASE64_DECODE: invalid base64".to_string()))
                }
            }

            // SLEEP(ms)
            "SLEEP" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("SLEEP requires 1 argument".to_string()));
                }
                let ms = evaluated_args[0].as_u64().ok_or_else(|| {
                    DbError::ExecutionError("SLEEP: argument must be a positive number".to_string())
                })?;
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Ok(Value::Bool(true))
            }

            // ASSERT(condition, message)
            "ASSERT" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("ASSERT requires 2 arguments".to_string()));
                }
                let condition = to_bool(&evaluated_args[0]);
                if !condition {
                    let msg = evaluated_args[1].as_str().unwrap_or("Assertion failed");
                    return Err(DbError::ExecutionError(msg.to_string()));
                }
                Ok(Value::Bool(true))
            }

             // TO_BOOL(value)
            "TO_BOOL" => {
                if evaluated_args.len() != 1 {
                     return Err(DbError::ExecutionError(
                        "TO_BOOL requires 1 argument: value".to_string(),
                    ));
                }
                let val = &evaluated_args[0];
                let bool_val = match val {
                    Value::Null => false,
                    Value::Bool(b) => *b,
                    Value::Number(n) => {
                        // 0 is false, everything else is true
                        if let Some(i) = n.as_i64() {
                            i != 0
                        } else if let Some(f) = n.as_f64() {
                            f != 0.0
                        } else {
                            true // Should be covered
                        }
                    },
                    Value::String(s) => !s.is_empty(),
                    Value::Array(_) => true,
                    Value::Object(_) => true,
                };
                Ok(Value::Bool(bool_val))
            }

            // TO_NUMBER(value)
            "TO_NUMBER" => {
                if evaluated_args.len() != 1 {
                     return Err(DbError::ExecutionError(
                        "TO_NUMBER requires 1 argument: value".to_string(),
                    ));
                }
                
                let mut current = &evaluated_args[0];
                // Unwrap arrays with single element
                while let Value::Array(arr) = current {
                    if arr.len() == 1 {
                        current = &arr[0];
                    } else {
                         // Empty or >1 elements -> 0
                         return Ok(Value::Number(serde_json::Number::from(0)));
                    }
                }

                let num_val = match current {
                    Value::Null => 0.0,
                    Value::Bool(true) => 1.0,
                    Value::Bool(false) => 0.0,
                    Value::Number(n) => n.as_f64().unwrap_or(0.0), 
                    Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
                    Value::Array(_) => 0.0, 
                    Value::Object(_) => 0.0,
                };

                // Return as integer if it's a whole number
                if num_val.fract() == 0.0 {
                     // Check range? i64 range.
                     if num_val >= (i64::MIN as f64) && num_val <= (i64::MAX as f64) {
                         return Ok(Value::Number(serde_json::Number::from(num_val as i64)));
                     }
                }
                
               if let Some(n) = serde_json::Number::from_f64(num_val) {
                   Ok(Value::Number(n))
               } else {
                   Ok(Value::Number(serde_json::Number::from(0)))
               }
            }

            // TO_STRING(value)
            "TO_STRING" => {
                if evaluated_args.len() != 1 {
                     return Err(DbError::ExecutionError(
                        "TO_STRING requires 1 argument: value".to_string(),
                    ));
                }
                let val = &evaluated_args[0];
                match val {
                    Value::Null => Ok(Value::String("".to_string())),
                    Value::String(s) => Ok(Value::String(s.clone())),
                    _ => {
                        match serde_json::to_string(val) {
                            Ok(s) => Ok(Value::String(s)),
                            Err(_) => Ok(Value::String("".to_string())), // Should fail safe?
                        }
                    }
                }
            }

            // TO_ARRAY(value)
            "TO_ARRAY" => {
                if evaluated_args.len() != 1 {
                     return Err(DbError::ExecutionError(
                        "TO_ARRAY requires 1 argument: value".to_string(),
                    ));
                }
                let val = &evaluated_args[0];
                match val {
                    Value::Null => Ok(Value::Array(vec![])),
                    Value::Array(arr) => Ok(Value::Array(arr.clone())),
                    Value::Object(obj) => {
                        let values: Vec<Value> = obj.values().cloned().collect();
                        Ok(Value::Array(values))
                    },
                    _ => Ok(Value::Array(vec![val.clone()])),
                }
            }

            // UNION(array1, array2, ...) - union of arrays (unique values)
            "UNION" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "UNION requires at least 1 argument".to_string(),
                    ));
                }
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                for arg in &evaluated_args {
                    let arr = arg.as_array().ok_or_else(|| {
                        DbError::ExecutionError("UNION: all arguments must be arrays".to_string())
                    })?;
                    for item in arr {
                        if seen.insert(item.to_string()) {
                            result.push(item.clone());
                        }
                    }
                }
                Ok(Value::Array(result))
            }

            // UNION_DISTINCT(array1, array2, ...) - same as UNION (for compatibility)
            "UNION_DISTINCT" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "UNION_DISTINCT requires at least 1 argument".to_string(),
                    ));
                }
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                for arg in &evaluated_args {
                    let arr = arg.as_array().ok_or_else(|| {
                        DbError::ExecutionError(
                            "UNION_DISTINCT: all arguments must be arrays".to_string(),
                        )
                    })?;
                    for item in arr {
                        if seen.insert(item.to_string()) {
                            result.push(item.clone());
                        }
                    }
                }
                Ok(Value::Array(result))
            }

            // MINUS(array1, array2) - elements in array1 not in array2
            "MINUS" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "MINUS requires 2 arguments: array1, array2".to_string(),
                    ));
                }
                let arr1 = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("MINUS: first argument must be an array".to_string())
                })?;
                let arr2 = evaluated_args[1].as_array().ok_or_else(|| {
                    DbError::ExecutionError("MINUS: second argument must be an array".to_string())
                })?;
                let set2: std::collections::HashSet<String> =
                    arr2.iter().map(|v| v.to_string()).collect();
                let mut seen = std::collections::HashSet::new();
                let result: Vec<Value> = arr1
                    .iter()
                    .filter(|v| {
                        let key = v.to_string();
                        !set2.contains(&key) && seen.insert(key)
                    })
                    .cloned()
                    .collect();
                Ok(Value::Array(result))
            }

            // INTERSECTION(array1, array2, ...) - common elements in all arrays
            "INTERSECTION" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "INTERSECTION requires at least 1 argument".to_string(),
                    ));
                }
                let first = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "INTERSECTION: all arguments must be arrays".to_string(),
                    )
                })?;

                if evaluated_args.len() == 1 {
                    return Ok(Value::Array(first.clone()));
                }

                // Build sets for all other arrays
                let mut sets: Vec<std::collections::HashSet<String>> = Vec::new();
                for arg in &evaluated_args[1..] {
                    let arr = arg.as_array().ok_or_else(|| {
                        DbError::ExecutionError(
                            "INTERSECTION: all arguments must be arrays".to_string(),
                        )
                    })?;
                    sets.push(arr.iter().map(|v| v.to_string()).collect());
                }

                let mut seen = std::collections::HashSet::new();
                let result: Vec<Value> = first
                    .iter()
                    .filter(|v| {
                        let key = v.to_string();
                        sets.iter().all(|s| s.contains(&key)) && seen.insert(key)
                    })
                    .cloned()
                    .collect();
                Ok(Value::Array(result))
            }

            // POSITION(array, search, start?) - find position of element in array (0-based, -1 if not found)
            "POSITION" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "POSITION requires 2-3 arguments: array, search, [start]".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("POSITION: first argument must be an array".to_string())
                })?;
                let search = &evaluated_args[1];
                let start = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_i64().unwrap_or(0) as usize
                } else {
                    0
                };
                let position = arr
                    .iter()
                    .skip(start)
                    .position(|v| v.to_string() == search.to_string())
                    .map(|p| p + start);
                Ok(match position {
                    Some(p) => Value::Number(serde_json::Number::from(p)),
                    None => Value::Number(serde_json::Number::from(-1)),
                })
            }

            // CONTAINS_ARRAY(array, search) - check if array contains element
            "CONTAINS_ARRAY" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "CONTAINS_ARRAY requires 2 arguments: array, search".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "CONTAINS_ARRAY: first argument must be an array".to_string(),
                    )
                })?;
                let search = &evaluated_args[1];
                let contains = arr.iter().any(|v| v.to_string() == search.to_string());
                Ok(Value::Bool(contains))
            }

            // ROUND(number, precision?) - round a number
            "ROUND" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "ROUND requires 1-2 arguments".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ROUND: first argument must be a number".to_string())
                })?;
                let precision = if evaluated_args.len() > 1 {
                    evaluated_args[1].as_i64().unwrap_or(0) as i32
                } else {
                    0
                };
                let factor = 10_f64.powi(precision);
                let rounded = (num * factor).round() / factor;
                Ok(Value::Number(
                    number_from_f64(rounded),
                ))
            }

            // ABS(number) - absolute value
            "ABS" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "ABS requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ABS: argument must be a number".to_string())
                })?;
                Ok(Value::Number(
                    number_from_f64(num.abs()),
                ))
            }



            // SQRT(n) - square root
            "SQRT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("SQRT requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                     DbError::ExecutionError("SQRT: argument must be a number".to_string())
                })?;
                if num < 0.0 {
                    return Err(DbError::ExecutionError("SQRT: cannot take square root of negative number".to_string()));
                }
                Ok(Value::Number(number_from_f64(num.sqrt())))
            }

            // POW(base, exp) - power
            "POW" | "POWER" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("POW requires 2 arguments".to_string()));
                }
                let base = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("POW: base must be a number".to_string())
                })?;
                let exp = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("POW: exponent must be a number".to_string())
                })?;
                
                Ok(Value::Number(number_from_f64(base.powf(exp))))
            }

            // FLOOR(number) - floor
            "FLOOR" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "FLOOR requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("FLOOR: argument must be a number".to_string())
                })?;
                Ok(Value::Number(
                    number_from_f64(num.floor()),
                ))
            }

            // CEIL(number) - ceiling
            "CEIL" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "CEIL requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("CEIL: argument must be a number".to_string())
                })?;
                Ok(Value::Number(
                    number_from_f64(num.ceil()),
                ))
            }

            // RANDOM() - random float between 0 and 1
            "RANDOM" => {
                if !evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "RANDOM takes no arguments".to_string(),
                    ));
                }
                use rand::Rng;
                let random_val: f64 = rand::thread_rng().gen();
                Ok(Value::Number(
                    number_from_f64(random_val),
                ))
            }

            // LOG(x) - natural logarithm (ln)
            "LOG" | "LN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("LOG requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("LOG: argument must be a number".to_string())
                })?;
                if num <= 0.0 {
                    return Err(DbError::ExecutionError("LOG: argument must be positive".to_string()));
                }
                Ok(Value::Number(number_from_f64(num.ln())))
            }

            // LOG10(x) - base-10 logarithm
            "LOG10" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("LOG10 requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("LOG10: argument must be a number".to_string())
                })?;
                if num <= 0.0 {
                    return Err(DbError::ExecutionError("LOG10: argument must be positive".to_string()));
                }
                Ok(Value::Number(number_from_f64(num.log10())))
            }

            // LOG2(x) - base-2 logarithm
            "LOG2" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("LOG2 requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("LOG2: argument must be a number".to_string())
                })?;
                if num <= 0.0 {
                    return Err(DbError::ExecutionError("LOG2: argument must be positive".to_string()));
                }
                Ok(Value::Number(number_from_f64(num.log2())))
            }

            // EXP(x) - e^x
            "EXP" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("EXP requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("EXP: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(num.exp())))
            }

            // SIN(x) - sine (x in radians)
            "SIN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("SIN requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("SIN: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(num.sin())))
            }

            // COS(x) - cosine (x in radians)
            "COS" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("COS requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("COS: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(num.cos())))
            }

            // TAN(x) - tangent (x in radians)
            "TAN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("TAN requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("TAN: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(num.tan())))
            }

            // ASIN(x) - arc sine
            "ASIN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("ASIN requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ASIN: argument must be a number".to_string())
                })?;
                if num < -1.0 || num > 1.0 {
                    return Err(DbError::ExecutionError("ASIN: argument must be between -1 and 1".to_string()));
                }
                Ok(Value::Number(number_from_f64(num.asin())))
            }

            // ACOS(x) - arc cosine
            "ACOS" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("ACOS requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ACOS: argument must be a number".to_string())
                })?;
                if num < -1.0 || num > 1.0 {
                    return Err(DbError::ExecutionError("ACOS: argument must be between -1 and 1".to_string()));
                }
                Ok(Value::Number(number_from_f64(num.acos())))
            }

            // ATAN(x) - arc tangent
            "ATAN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("ATAN requires 1 argument".to_string()));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ATAN: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(num.atan())))
            }

            // ATAN2(y, x) - arc tangent of y/x
            "ATAN2" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("ATAN2 requires 2 arguments".to_string()));
                }
                let y = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ATAN2: y must be a number".to_string())
                })?;
                let x = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ATAN2: x must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(y.atan2(x))))
            }

            // PI() - returns pi constant
            "PI" => {
                if !evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError("PI takes no arguments".to_string()));
                }
                Ok(Value::Number(number_from_f64(std::f64::consts::PI)))
            }

            // COALESCE(a, b, ...) - return first non-null value
            "COALESCE" | "NOT_NULL" | "FIRST_NOT_NULL" => {
                for arg in &evaluated_args {
                    if !arg.is_null() {
                        return Ok(arg.clone());
                    }
                }
                Ok(Value::Null)
            }

            // LEFT(str, n) - get first n characters
            "LEFT" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("LEFT requires 2 arguments".to_string()));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("LEFT: first argument must be a string".to_string())
                })?;
                let n = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("LEFT: second argument must be a number".to_string())
                })? as usize;
                let result: String = s.chars().take(n).collect();
                Ok(Value::String(result))
            }

            // RIGHT(str, n) - get last n characters
            "RIGHT" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("RIGHT requires 2 arguments".to_string()));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("RIGHT: first argument must be a string".to_string())
                })?;
                let n = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("RIGHT: second argument must be a number".to_string())
                })? as usize;
                let chars: Vec<char> = s.chars().collect();
                let start = chars.len().saturating_sub(n);
                let result: String = chars[start..].iter().collect();
                Ok(Value::String(result))
            }

            // CHAR_LENGTH(str) - character count (unicode-aware)
            "CHAR_LENGTH" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("CHAR_LENGTH requires 1 argument".to_string()));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("CHAR_LENGTH: argument must be a string".to_string())
                })?;
                Ok(Value::Number(serde_json::Number::from(s.chars().count())))
            }

            // FIND_FIRST(str, search, start?) - find first occurrence, return index or -1
            "FIND_FIRST" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError("FIND_FIRST requires 2-3 arguments".to_string()));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("FIND_FIRST: first argument must be a string".to_string())
                })?;
                let search = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("FIND_FIRST: second argument must be a string".to_string())
                })?;
                let start = if evaluated_args.len() == 3 {
                    evaluated_args[2].as_f64().unwrap_or(0.0) as usize
                } else {
                    0
                };
                
                if start >= s.len() {
                    return Ok(Value::Number(serde_json::Number::from(-1)));
                }
                
                match s[start..].find(search) {
                    Some(idx) => Ok(Value::Number(serde_json::Number::from(start + idx))),
                    None => Ok(Value::Number(serde_json::Number::from(-1))),
                }
            }

            // FIND_LAST(str, search, end?) - find last occurrence, return index or -1
            "FIND_LAST" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError("FIND_LAST requires 2-3 arguments".to_string()));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("FIND_LAST: first argument must be a string".to_string())
                })?;
                let search = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("FIND_LAST: second argument must be a string".to_string())
                })?;
                let end = if evaluated_args.len() == 3 {
                    evaluated_args[2].as_f64().unwrap_or(s.len() as f64) as usize
                } else {
                    s.len()
                };
                
                let search_str = &s[..end.min(s.len())];
                match search_str.rfind(search) {
                    Some(idx) => Ok(Value::Number(serde_json::Number::from(idx))),
                    None => Ok(Value::Number(serde_json::Number::from(-1))),
                }
            }

            // REGEX_TEST(str, pattern) - test if string matches regex pattern
            "REGEX_TEST" | "REGEX_MATCH" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError("REGEX_TEST requires 2 arguments".to_string()));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("REGEX_TEST: first argument must be a string".to_string())
                })?;
                let pattern = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("REGEX_TEST: second argument must be a string (pattern)".to_string())
                })?;
                
                use regex::Regex;
                let re = Regex::new(pattern).map_err(|e| {
                    DbError::ExecutionError(format!("REGEX_TEST: invalid regex '{}': {}", pattern, e))
                })?;
                Ok(Value::Bool(re.is_match(s)))
            }

            // DATE_YEAR(date) - extract year from date
            "DATE_YEAR" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("DATE_YEAR requires 1 argument".to_string()));
                }
                let dt = parse_datetime(&evaluated_args[0])?;
                use chrono::Datelike;
                Ok(Value::Number(serde_json::Number::from(dt.year())))
            }

            // DATE_MONTH(date) - extract month from date (1-12)
            "DATE_MONTH" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("DATE_MONTH requires 1 argument".to_string()));
                }
                let dt = parse_datetime(&evaluated_args[0])?;
                use chrono::Datelike;
                Ok(Value::Number(serde_json::Number::from(dt.month())))
            }

            // DATE_DAY(date) - extract day of month from date (1-31)
            "DATE_DAY" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("DATE_DAY requires 1 argument".to_string()));
                }
                let dt = parse_datetime(&evaluated_args[0])?;
                use chrono::Datelike;
                Ok(Value::Number(serde_json::Number::from(dt.day())))
            }

            // DATE_HOUR(date) - extract hour from date (0-23)
            "DATE_HOUR" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("DATE_HOUR requires 1 argument".to_string()));
                }
                let dt = parse_datetime(&evaluated_args[0])?;
                use chrono::Timelike;
                Ok(Value::Number(serde_json::Number::from(dt.hour())))
            }

            // DATE_MINUTE(date) - extract minute from date (0-59)
            "DATE_MINUTE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("DATE_MINUTE requires 1 argument".to_string()));
                }
                let dt = parse_datetime(&evaluated_args[0])?;
                use chrono::Timelike;
                Ok(Value::Number(serde_json::Number::from(dt.minute())))
            }

            // DATE_SECOND(date) - extract second from date (0-59)
            "DATE_SECOND" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("DATE_SECOND requires 1 argument".to_string()));
                }
                let dt = parse_datetime(&evaluated_args[0])?;
                use chrono::Timelike;
                Ok(Value::Number(serde_json::Number::from(dt.second())))
            }

            // DATE_DAYOFWEEK(date) - extract day of week (0=Sunday, 1=Monday, ..., 6=Saturday)
            "DATE_DAYOFWEEK" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("DATE_DAYOFWEEK requires 1 argument".to_string()));
                }
                let dt = parse_datetime(&evaluated_args[0])?;
                use chrono::Datelike;
                // chrono returns Monday=0, we want Sunday=0
                let weekday = dt.weekday().num_days_from_sunday();
                Ok(Value::Number(serde_json::Number::from(weekday)))
            }

            // DATE_QUARTER(date) - extract quarter (1-4)
            "DATE_QUARTER" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError("DATE_QUARTER requires 1 argument".to_string()));
                }
                let dt = parse_datetime(&evaluated_args[0])?;
                use chrono::Datelike;
                let quarter = (dt.month() - 1) / 3 + 1;
                Ok(Value::Number(serde_json::Number::from(quarter)))
            }

            // RANGE(start, end, step?) - generate array of numbers
            "RANGE" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError("RANGE requires 2-3 arguments".to_string()));
                }
                let start = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("RANGE: start must be a number".to_string())
                })? as i64;
                let end = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("RANGE: end must be a number".to_string())
                })? as i64;
                let step = if evaluated_args.len() == 3 {
                    evaluated_args[2].as_f64().ok_or_else(|| {
                        DbError::ExecutionError("RANGE: step must be a number".to_string())
                    })? as i64
                } else {
                    1
                };
                
                if step == 0 {
                    return Err(DbError::ExecutionError("RANGE: step cannot be 0".to_string()));
                }
                
                let mut result = Vec::new();
                if step > 0 {
                    let mut i = start;
                    while i <= end {
                        result.push(Value::Number(serde_json::Number::from(i)));
                        i += step;
                    }
                } else {
                    let mut i = start;
                    while i >= end {
                        result.push(Value::Number(serde_json::Number::from(i)));
                        i += step;
                    }
                }
                Ok(Value::Array(result))
            }


            // UPPER(string) - uppercase
            "UPPER" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "UPPER requires 1 argument".to_string(),
                    ));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("UPPER: argument must be a string".to_string())
                })?;
                Ok(Value::String(s.to_uppercase()))
            }

            // LOWER(string) - lowercase
            "LOWER" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "LOWER requires 1 argument".to_string(),
                    ));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("LOWER: argument must be a string".to_string())
                })?;
                Ok(Value::String(s.to_lowercase()))
            }

            // CONCAT(str1, str2, ...) - concatenate strings
            "CONCAT" => {
                let mut result = String::new();
                for arg in &evaluated_args {
                    match arg {
                        Value::String(s) => result.push_str(s),
                        Value::Number(n) => result.push_str(&n.to_string()),
                        Value::Bool(b) => result.push_str(&b.to_string()),
                        Value::Null => result.push_str("null"),
                        _ => {
                            return Err(DbError::ExecutionError(
                                "CONCAT: arguments must be strings or primitives".to_string(),
                            ))
                        }
                    }
                }
                Ok(Value::String(result))
            }

            // CONCAT_SEPARATOR(separator, array) - join array elements with separator
            "CONCAT_SEPARATOR" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "CONCAT_SEPARATOR requires 2 arguments: separator and array".to_string(),
                    ));
                }
                let separator = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "CONCAT_SEPARATOR: first argument (separator) must be a string".to_string(),
                    )
                })?;

                let array = match &evaluated_args[1] {
                    Value::Array(arr) => arr,
                    _ => {
                        return Err(DbError::ExecutionError(
                            "CONCAT_SEPARATOR: second argument must be an array".to_string(),
                        ))
                    }
                };

                let strings: Vec<String> = array
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => "null".to_string(),
                        _ => format!("{}", v),
                    })
                    .collect();

                Ok(Value::String(strings.join(separator)))
            }

            // SUBSTRING(string, start, length?) - substring
            "SUBSTRING" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "SUBSTRING requires 2-3 arguments".to_string(),
                    ));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "SUBSTRING: first argument must be a string".to_string(),
                    )
                })?;
                let start = evaluated_args[1].as_i64().ok_or_else(|| {
                    DbError::ExecutionError("SUBSTRING: start must be a number".to_string())
                })? as usize;
                let len = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_i64().unwrap_or(s.len() as i64) as usize
                } else {
                    s.len() - start
                };

                let result: String = s.chars().skip(start).take(len).collect();
                Ok(Value::String(result))
            }

            // FULLTEXT(collection, field, query, maxDistance?) - fulltext search with fuzzy matching
            "FULLTEXT" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "FULLTEXT requires 3-4 arguments: collection, field, query, [maxDistance]"
                            .to_string(),
                    ));
                }
                let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("FULLTEXT: collection must be a string".to_string())
                })?;
                let field = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("FULLTEXT: field must be a string".to_string())
                })?;
                let query = evaluated_args[2].as_str().ok_or_else(|| {
                    DbError::ExecutionError("FULLTEXT: query must be a string".to_string())
                })?;
                let max_distance = if evaluated_args.len() == 4 {
                    evaluated_args[3].as_u64().unwrap_or(2) as usize
                } else {
                    2 // Default Levenshtein distance
                };

                let collection = self.get_collection(collection_name)?;

                match collection.fulltext_search(field, query, max_distance) {
                    Some(matches) => {
                        let results: Vec<Value> = matches
                            .iter()
                            .filter_map(|m| {
                                collection.get(&m.doc_key).ok().map(|doc| {
                                    let mut obj = serde_json::Map::new();
                                    obj.insert("doc".to_string(), doc.to_value());
                                    obj.insert("score".to_string(), json!(m.score));
                                    obj.insert("matched".to_string(), json!(m.matched_terms));
                                    Value::Object(obj)
                                })
                            })
                            .collect();
                        Ok(Value::Array(results))
                    }
                    None => Err(DbError::ExecutionError(format!(
                        "No fulltext index found on field '{}' in collection '{}'",
                        field, collection_name
                    ))),
                }
            }

            // LEVENSHTEIN(string1, string2) - Levenshtein distance between two strings
            "LEVENSHTEIN" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "LEVENSHTEIN requires 2 arguments: string1, string2".to_string(),
                    ));
                }
                let s1 = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "LEVENSHTEIN: first argument must be a string".to_string(),
                    )
                })?;
                let s2 = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "LEVENSHTEIN: second argument must be a string".to_string(),
                    )
                })?;

                let distance = crate::storage::levenshtein_distance(s1, s2);
                Ok(Value::Number(serde_json::Number::from(distance)))
            }

            // BM25(field, query) - BM25 relevance scoring for a document field
            // Returns a numeric score that can be used in SORT clauses
            // Usage: SORT BM25(doc.content, "search query") DESC
            "BM25" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "BM25 requires 2 arguments: field, query".to_string(),
                    ));
                }

                // Get the field value (should be a string from the document)
                let field_text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("BM25: field must be a string".to_string())
                })?;

                let query = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("BM25: query must be a string".to_string())
                })?;

                // Tokenize query and document
                use crate::storage::{bm25_score, tokenize};
                let query_terms = tokenize(query);
                let doc_terms = tokenize(field_text);
                let doc_length = doc_terms.len();

                // For BM25, we need collection statistics
                // Since we don't have access to the collection here, we'll use simplified scoring
                // In a real implementation, we'd need to pass collection context
                // For now, use a simplified version with estimated parameters
                let avg_doc_length = 100.0; // Estimated average
                let total_docs = 1000; // Estimated total

                // Create a simple term document frequency map
                // In a real implementation, this would come from the collection's fulltext index
                let mut term_doc_freq = std::collections::HashMap::new();
                for term in &query_terms {
                    // Estimate: assume each term appears in ~10% of documents
                    term_doc_freq.insert(term.clone(), total_docs / 10);
                }

                let score = bm25_score(
                    &query_terms,
                    &doc_terms,
                    doc_length,
                    avg_doc_length,
                    total_docs,
                    &term_doc_freq,
                );

                Ok(Value::Number(
                    serde_json::Number::from_f64(score).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // MERGE(obj1, obj2, ...) - merge multiple objects (later objects override earlier ones)
            "MERGE" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "MERGE requires at least 1 argument".to_string(),
                    ));
                }

                let mut result = serde_json::Map::new();

                for arg in &evaluated_args {
                    match arg {
                        Value::Object(obj) => {
                            // Merge this object into the result
                            for (key, value) in obj {
                                result.insert(key.clone(), value.clone());
                            }
                        }
                        Value::Null => {
                            // Skip null values
                            continue;
                        }
                        _ => {
                            return Err(DbError::ExecutionError(format!(
                                "MERGE: all arguments must be objects, got: {:?}",
                                arg
                            )));
                        }
                    }
                }

                Ok(Value::Object(result))
            }

            // DATE_NOW() - current timestamp in milliseconds since Unix epoch
            "DATE_NOW" => {
                if !evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "DATE_NOW requires 0 arguments".to_string(),
                    ));
                }
                let timestamp = Utc::now().timestamp_millis();
                Ok(Value::Number(serde_json::Number::from(timestamp)))
            }

            // COLLECTION_COUNT(collection) - get the count of documents in a collection
            "COLLECTION_COUNT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "COLLECTION_COUNT requires 1 argument: collection name".to_string(),
                    ));
                }
                let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "COLLECTION_COUNT: argument must be a string (collection name)".to_string(),
                    )
                })?;

                let collection = self.get_collection(collection_name)?;
                let count = collection.count();
                Ok(Value::Number(serde_json::Number::from(count)))
            }

            // DATE_ISO8601(date) - convert timestamp to ISO 8601 string
            "DATE_ISO8601" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "DATE_ISO8601 requires 1 argument: timestamp in milliseconds".to_string(),
                    ));
                }

                // Handle both integer and float timestamps
                let timestamp_ms = match &evaluated_args[0] {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError("DATE_ISO8601: argument must be a number (timestamp in milliseconds)".to_string()));
                        }
                    }
                    _ => {
                        return Err(DbError::ExecutionError(
                            "DATE_ISO8601: argument must be a number (timestamp in milliseconds)"
                                .to_string(),
                        ))
                    }
                };

                // Convert milliseconds to seconds for chrono
                let timestamp_secs = timestamp_ms / 1000;
                let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;

                // Create DateTime from timestamp
                use chrono::TimeZone;
                let datetime = match Utc.timestamp_opt(timestamp_secs, nanos) {
                    chrono::LocalResult::Single(dt) => dt,
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "DATE_ISO8601: invalid timestamp: {}",
                            timestamp_ms
                        )))
                    }
                };

                // Format as ISO 8601 string (e.g., "2023-12-03T13:44:00.000Z")
                let iso_string = datetime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                Ok(Value::String(iso_string))
            }

            // DATE_TIMESTAMP(date) - convert ISO 8601 string to timestamp in milliseconds
            "DATE_TIMESTAMP" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "DATE_TIMESTAMP requires 1 argument: ISO 8601 date string".to_string(),
                    ));
                }

                let date_str = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "DATE_TIMESTAMP: argument must be a string (ISO 8601 date)".to_string(),
                    )
                })?;

                // Parse ISO 8601 string to DateTime
                use chrono::DateTime;
                let datetime = DateTime::parse_from_rfc3339(date_str).map_err(|e| {
                    DbError::ExecutionError(format!(
                        "DATE_TIMESTAMP: invalid ISO 8601 date '{}': {}",
                        date_str, e
                    ))
                })?;

                // Convert to milliseconds since Unix epoch
                let timestamp_ms = datetime.timestamp_millis();
                Ok(Value::Number(serde_json::Number::from(timestamp_ms)))
            }

            // DATE_TRUNC(date, unit, timezone?) - truncate date to specified unit
            "DATE_TRUNC" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "DATE_TRUNC requires 2-3 arguments: date, unit, [timezone]".to_string(),
                    ));
                }

                use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Timelike};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_TRUNC: invalid timestamp".to_string(),
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => {
                                return Err(DbError::ExecutionError(format!(
                                    "DATE_TRUNC: invalid timestamp: {}",
                                    timestamp_ms
                                )))
                            }
                        }
                    }
                    Value::String(s) => DateTime::parse_from_rfc3339(s)
                        .map_err(|e| {
                            DbError::ExecutionError(format!(
                                "DATE_TRUNC: invalid ISO 8601 date '{}': {}",
                                s, e
                            ))
                        })?
                        .with_timezone(&Utc),
                    _ => {
                        return Err(DbError::ExecutionError(
                            "DATE_TRUNC: first argument must be a timestamp or ISO 8601 string"
                                .to_string(),
                        ))
                    }
                };

                // Parse the unit
                let unit = evaluated_args[1]
                    .as_str()
                    .ok_or_else(|| {
                        DbError::ExecutionError("DATE_TRUNC: unit must be a string".to_string())
                    })?
                    .to_lowercase();

                // Parse optional timezone
                let tz: Tz = if evaluated_args.len() == 3 {
                    let tz_str = evaluated_args[2].as_str().ok_or_else(|| {
                        DbError::ExecutionError("DATE_TRUNC: timezone must be a string".to_string())
                    })?;
                    tz_str.parse::<Tz>().map_err(|_| {
                        DbError::ExecutionError(format!(
                            "DATE_TRUNC: unknown timezone '{}'",
                            tz_str
                        ))
                    })?
                } else {
                    chrono_tz::UTC
                };

                // Convert to the target timezone for truncation
                let datetime_tz = datetime_utc.with_timezone(&tz);

                // Truncate based on unit
                let truncated: DateTime<Tz> = match unit.as_str() {
                    "y" | "year" | "years" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), 1, 1).unwrap(),
                            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "m" | "month" | "months" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), 1).unwrap(),
                            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "d" | "day" | "days" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                            chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "h" | "hour" | "hours" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                            chrono::NaiveTime::from_hms_opt(datetime_tz.hour(), 0, 0).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "i" | "minute" | "minutes" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                            chrono::NaiveTime::from_hms_opt(datetime_tz.hour(), datetime_tz.minute(), 0).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "s" | "second" | "seconds" => {
                        let naive = NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(datetime_tz.year(), datetime_tz.month(), datetime_tz.day()).unwrap(),
                            chrono::NaiveTime::from_hms_opt(datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second()).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_TRUNC: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "f" | "millisecond" | "milliseconds" => {
                        // Keep the original datetime (milliseconds are the finest granularity we support)
                        datetime_tz
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("DATE_TRUNC: unknown unit '{}'. Valid units: y/year/years, m/month/months, d/day/days, h/hour/hours, i/minute/minutes, s/second/seconds, f/millisecond/milliseconds", unit)
                    )),
                };

                // Convert back to UTC and format as ISO 8601
                let truncated_utc = truncated.with_timezone(&Utc);
                let iso_string = truncated_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                Ok(Value::String(iso_string))
            }

            // DATE_DAYS_IN_MONTH(date, timezone?) - return number of days in the month
            "DATE_DAYS_IN_MONTH" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "DATE_DAYS_IN_MONTH requires 1-2 arguments: date, [timezone]".to_string(),
                    ));
                }

                use chrono::{DateTime, Datelike, NaiveDate, TimeZone};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_DAYS_IN_MONTH: invalid timestamp".to_string(),
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => {
                                return Err(DbError::ExecutionError(format!(
                                    "DATE_DAYS_IN_MONTH: invalid timestamp: {}",
                                    timestamp_ms
                                )))
                            }
                        }
                    }
                    Value::String(s) => DateTime::parse_from_rfc3339(s)
                        .map_err(|e| {
                            DbError::ExecutionError(format!(
                                "DATE_DAYS_IN_MONTH: invalid ISO 8601 date '{}': {}",
                                s, e
                            ))
                        })?
                        .with_timezone(&Utc),
                    _ => return Err(DbError::ExecutionError(
                        "DATE_DAYS_IN_MONTH: first argument must be a timestamp or ISO 8601 string"
                            .to_string(),
                    )),
                };

                // Get year and month, optionally in a specific timezone
                let (year, month) = if evaluated_args.len() == 2 {
                    let tz_str = evaluated_args[1].as_str().ok_or_else(|| {
                        DbError::ExecutionError(
                            "DATE_DAYS_IN_MONTH: timezone must be a string".to_string(),
                        )
                    })?;
                    let tz: Tz = tz_str.parse().map_err(|_| {
                        DbError::ExecutionError(format!(
                            "DATE_DAYS_IN_MONTH: unknown timezone '{}'",
                            tz_str
                        ))
                    })?;
                    let dt_tz = datetime_utc.with_timezone(&tz);
                    (dt_tz.year(), dt_tz.month())
                } else {
                    (datetime_utc.year(), datetime_utc.month())
                };

                // Calculate days in month by finding the first day of next month
                // and subtracting from it
                let days_in_month = if month == 12 {
                    NaiveDate::from_ymd_opt(year + 1, 1, 1)
                } else {
                    NaiveDate::from_ymd_opt(year, month + 1, 1)
                }
                .and_then(|next_month| {
                    NaiveDate::from_ymd_opt(year, month, 1)
                        .map(|this_month| (next_month - this_month).num_days())
                })
                .unwrap_or(30) as u32; // Fallback, though this shouldn't happen

                Ok(Value::Number(serde_json::Number::from(days_in_month)))
            }

            // DATE_DAYOFYEAR(date, timezone?) - return day of year (1-366)
            "DATE_DAYOFYEAR" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "DATE_DAYOFYEAR requires 1-2 arguments: date, [timezone]".to_string(),
                    ));
                }

                use chrono::{DateTime, Datelike, TimeZone};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> =
                    match &evaluated_args[0] {
                        Value::Number(n) => {
                            let timestamp_ms = if let Some(i) = n.as_i64() {
                                i
                            } else if let Some(f) = n.as_f64() {
                                f as i64
                            } else {
                                return Err(DbError::ExecutionError(
                                    "DATE_DAYOFYEAR: invalid timestamp".to_string(),
                                ));
                            };
                            let secs = timestamp_ms / 1000;
                            let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                            match Utc.timestamp_opt(secs, nanos) {
                                chrono::LocalResult::Single(dt) => dt,
                                _ => {
                                    return Err(DbError::ExecutionError(format!(
                                        "DATE_DAYOFYEAR: invalid timestamp: {}",
                                        timestamp_ms
                                    )))
                                }
                            }
                        }
                        Value::String(s) => DateTime::parse_from_rfc3339(s)
                            .map_err(|e| {
                                DbError::ExecutionError(format!(
                                    "DATE_DAYOFYEAR: invalid ISO 8601 date '{}': {}",
                                    s, e
                                ))
                            })?
                            .with_timezone(&Utc),
                        _ => return Err(DbError::ExecutionError(
                            "DATE_DAYOFYEAR: first argument must be a timestamp or ISO 8601 string"
                                .to_string(),
                        )),
                    };

                // Parse optional timezone
                let day_of_year = if evaluated_args.len() == 2 {
                    let tz_str = evaluated_args[1].as_str().ok_or_else(|| {
                        DbError::ExecutionError(
                            "DATE_DAYOFYEAR: timezone must be a string".to_string(),
                        )
                    })?;
                    let tz: Tz = tz_str.parse().map_err(|_| {
                        DbError::ExecutionError(format!(
                            "DATE_DAYOFYEAR: unknown timezone '{}'",
                            tz_str
                        ))
                    })?;
                    datetime_utc.with_timezone(&tz).ordinal()
                } else {
                    datetime_utc.ordinal()
                };

                Ok(Value::Number(serde_json::Number::from(day_of_year)))
            }

            // DATE_ISOWEEK(date) - return ISO 8601 week number
            "DATE_ISOWEEK" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "DATE_ISOWEEK requires 1 argument: date".to_string(),
                    ));
                }

                use chrono::{DateTime, Datelike, TimeZone};

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_ISOWEEK: invalid timestamp".to_string(),
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => {
                                return Err(DbError::ExecutionError(format!(
                                    "DATE_ISOWEEK: invalid timestamp: {}",
                                    timestamp_ms
                                )))
                            }
                        }
                    }
                    Value::String(s) => DateTime::parse_from_rfc3339(s)
                        .map_err(|e| {
                            DbError::ExecutionError(format!(
                                "DATE_ISOWEEK: invalid ISO 8601 date '{}': {}",
                                s, e
                            ))
                        })?
                        .with_timezone(&Utc),
                    _ => {
                        return Err(DbError::ExecutionError(
                            "DATE_ISOWEEK: argument must be a timestamp or ISO 8601 string"
                                .to_string(),
                        ))
                    }
                };

                // Get ISO week number
                let iso_week = datetime_utc.iso_week().week();
                Ok(Value::Number(serde_json::Number::from(iso_week)))
            }

            // DATE_FORMAT(date, format, timezone?) - format date according to format string
            "DATE_FORMAT" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "DATE_FORMAT requires 2-3 arguments: date, format, [timezone]".to_string(),
                    ));
                }

                use chrono::{DateTime, TimeZone};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> =
                    match &evaluated_args[0] {
                        Value::Number(n) => {
                            let timestamp_ms = if let Some(i) = n.as_i64() {
                                i
                            } else if let Some(f) = n.as_f64() {
                                f as i64
                            } else {
                                return Err(DbError::ExecutionError(
                                    "DATE_FORMAT: invalid timestamp".to_string(),
                                ));
                            };
                            let secs = timestamp_ms / 1000;
                            let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                            match Utc.timestamp_opt(secs, nanos) {
                                chrono::LocalResult::Single(dt) => dt,
                                _ => {
                                    return Err(DbError::ExecutionError(format!(
                                        "DATE_FORMAT: invalid timestamp: {}",
                                        timestamp_ms
                                    )))
                                }
                            }
                        }
                        Value::String(s) => DateTime::parse_from_rfc3339(s)
                            .map_err(|e| {
                                DbError::ExecutionError(format!(
                                    "DATE_FORMAT: invalid ISO 8601 date '{}': {}",
                                    s, e
                                ))
                            })?
                            .with_timezone(&Utc),
                        _ => return Err(DbError::ExecutionError(
                            "DATE_FORMAT: first argument must be a timestamp or ISO 8601 string"
                                .to_string(),
                        )),
                    };

                // Parse the format string
                let format_str = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("DATE_FORMAT: format must be a string".to_string())
                })?;

                // Parse optional timezone
                let tz: Tz = if evaluated_args.len() == 3 {
                    let tz_str = evaluated_args[2].as_str().ok_or_else(|| {
                        DbError::ExecutionError(
                            "DATE_FORMAT: timezone must be a string".to_string(),
                        )
                    })?;
                    tz_str.parse::<Tz>().map_err(|_| {
                        DbError::ExecutionError(format!(
                            "DATE_FORMAT: unknown timezone '{}'",
                            tz_str
                        ))
                    })?
                } else {
                    chrono_tz::UTC
                };

                // Convert to the target timezone
                let datetime_tz = datetime_utc.with_timezone(&tz);

                // Format using strftime-style format string
                // Chrono supports: %Y, %m, %d, %H, %M, %S, %f, %a, %A, %b, %B, %j, %U, %W, %w, %Z, etc.
                let formatted = datetime_tz.format(format_str).to_string();
                Ok(Value::String(formatted))
            }

            // DATE_ADD(date, amount, unit, timezone?) - add amount of time to date
            "DATE_ADD" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "DATE_ADD requires 3-4 arguments: date, amount, unit, [timezone]"
                            .to_string(),
                    ));
                }

                use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_ADD: invalid timestamp".to_string(),
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => {
                                return Err(DbError::ExecutionError(format!(
                                    "DATE_ADD: invalid timestamp: {}",
                                    timestamp_ms
                                )))
                            }
                        }
                    }
                    Value::String(s) => DateTime::parse_from_rfc3339(s)
                        .map_err(|e| {
                            DbError::ExecutionError(format!(
                                "DATE_ADD: invalid ISO 8601 date '{}': {}",
                                s, e
                            ))
                        })?
                        .with_timezone(&Utc),
                    _ => {
                        return Err(DbError::ExecutionError(
                            "DATE_ADD: first argument must be a timestamp or ISO 8601 string"
                                .to_string(),
                        ))
                    }
                };

                // Parse the amount
                let amount = match &evaluated_args[1] {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_ADD: amount must be a number".to_string(),
                            ));
                        }
                    }
                    _ => {
                        return Err(DbError::ExecutionError(
                            "DATE_ADD: amount must be a number".to_string(),
                        ))
                    }
                };

                // Parse the unit
                let unit = evaluated_args[2]
                    .as_str()
                    .ok_or_else(|| {
                        DbError::ExecutionError("DATE_ADD: unit must be a string".to_string())
                    })?
                    .to_lowercase();

                // Parse optional timezone
                let tz: Tz = if evaluated_args.len() == 4 {
                    let tz_str = evaluated_args[3].as_str().ok_or_else(|| {
                        DbError::ExecutionError("DATE_ADD: timezone must be a string".to_string())
                    })?;
                    tz_str.parse::<Tz>().map_err(|_| {
                        DbError::ExecutionError(format!("DATE_ADD: unknown timezone '{}'", tz_str))
                    })?
                } else {
                    chrono_tz::UTC
                };

                // Convert to the target timezone for calculation
                let datetime_tz = datetime_utc.with_timezone(&tz);

                // Perform the addition based on unit
                let result_tz: DateTime<Tz> = match unit.as_str() {
                    "y" | "year" | "years" => {
                        // Add years
                        let new_year = datetime_tz.year() + amount as i32;
                        let naive = chrono::NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(new_year, datetime_tz.month(), datetime_tz.day())
                                .unwrap_or_else(|| {
                                    // Handle invalid dates (e.g., Feb 29 in non-leap year)
                                    chrono::NaiveDate::from_ymd_opt(new_year, datetime_tz.month(), 28).unwrap()
                                }),
                            chrono::NaiveTime::from_hms_milli_opt(
                                datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second(),
                                datetime_tz.timestamp_subsec_millis()
                            ).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "m" | "month" | "months" => {
                        // Add months
                        let total_months = datetime_tz.year() * 12 + datetime_tz.month() as i32 - 1 + amount as i32;
                        let new_year = total_months / 12;
                        let new_month = (total_months % 12 + 1) as u32;

                        // Handle day overflow (e.g., Jan 31 + 1 month = Feb 28/29)
                        let max_day = chrono::NaiveDate::from_ymd_opt(new_year, new_month + 1, 1)
                            .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(new_year + 1, 1, 1).unwrap())
                            .pred_opt()
                            .unwrap()
                            .day();
                        let new_day = datetime_tz.day().min(max_day);

                        let naive = chrono::NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(new_year, new_month, new_day).unwrap(),
                            chrono::NaiveTime::from_hms_milli_opt(
                                datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second(),
                                datetime_tz.timestamp_subsec_millis()
                            ).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "w" | "week" | "weeks" => {
                        // Add weeks (7 days)
                        datetime_tz.checked_add_signed(Duration::weeks(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    "d" | "day" | "days" => {
                        // Add days
                        datetime_tz.checked_add_signed(Duration::days(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    "h" | "hour" | "hours" => {
                        // Add hours
                        datetime_tz.checked_add_signed(Duration::hours(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    "i" | "minute" | "minutes" => {
                        // Add minutes
                        datetime_tz.checked_add_signed(Duration::minutes(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    "s" | "second" | "seconds" => {
                        // Add seconds
                        datetime_tz.checked_add_signed(Duration::seconds(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    "f" | "millisecond" | "milliseconds" => {
                        // Add milliseconds
                        datetime_tz.checked_add_signed(Duration::milliseconds(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_ADD: date arithmetic overflow".to_string()
                            ))?
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("DATE_ADD: unknown unit '{}'. Valid units: y/year/years, m/month/months, w/week/weeks, d/day/days, h/hour/hours, i/minute/minutes, s/second/seconds, f/millisecond/milliseconds", unit)
                    )),
                };

                // Convert back to UTC and format as ISO 8601
                let result_utc = result_tz.with_timezone(&Utc);
                let iso_string = result_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                Ok(Value::String(iso_string))
            }

            // DATE_SUBTRACT(date, amount, unit, timezone?) - subtract amount of time from date
            // This is a convenience wrapper around DATE_ADD with negated amount
            "DATE_SUBTRACT" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "DATE_SUBTRACT requires 3-4 arguments: date, amount, unit, [timezone]"
                            .to_string(),
                    ));
                }

                // Negate the amount and reuse DATE_ADD logic
                let negated_amount = match &evaluated_args[1] {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            Value::Number(serde_json::Number::from(-i))
                        } else if let Some(f) = n.as_f64() {
                            Value::Number(number_from_f64(-f))
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_SUBTRACT: amount must be a number".to_string(),
                            ));
                        }
                    }
                    _ => {
                        return Err(DbError::ExecutionError(
                            "DATE_SUBTRACT: amount must be a number".to_string(),
                        ))
                    }
                };

                // Build new evaluated_args with negated amount
                let mut new_evaluated_args = evaluated_args.clone();
                new_evaluated_args[1] = negated_amount;

                // Now execute the DATE_ADD logic inline with the negated amount
                // (We can't easily call evaluate_function recursively with modified args,
                // so we just negate and fall through to DATE_ADD logic by swapping the name)

                // Actually, let's just duplicate the key parts of DATE_ADD logic here
                // but with the negated amount
                use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike};
                use chrono_tz::Tz;

                // Parse the date (can be timestamp or ISO string)
                let datetime_utc: DateTime<Utc> =
                    match &new_evaluated_args[0] {
                        Value::Number(n) => {
                            let timestamp_ms = if let Some(i) = n.as_i64() {
                                i
                            } else if let Some(f) = n.as_f64() {
                                f as i64
                            } else {
                                return Err(DbError::ExecutionError(
                                    "DATE_SUBTRACT: invalid timestamp".to_string(),
                                ));
                            };
                            let secs = timestamp_ms / 1000;
                            let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                            match Utc.timestamp_opt(secs, nanos) {
                                chrono::LocalResult::Single(dt) => dt,
                                _ => {
                                    return Err(DbError::ExecutionError(format!(
                                        "DATE_SUBTRACT: invalid timestamp: {}",
                                        timestamp_ms
                                    )))
                                }
                            }
                        }
                        Value::String(s) => DateTime::parse_from_rfc3339(s)
                            .map_err(|e| {
                                DbError::ExecutionError(format!(
                                    "DATE_SUBTRACT: invalid ISO 8601 date '{}': {}",
                                    s, e
                                ))
                            })?
                            .with_timezone(&Utc),
                        _ => return Err(DbError::ExecutionError(
                            "DATE_SUBTRACT: first argument must be a timestamp or ISO 8601 string"
                                .to_string(),
                        )),
                    };

                // Parse the negated amount
                let amount = match &new_evaluated_args[1] {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_SUBTRACT: amount must be a number".to_string(),
                            ));
                        }
                    }
                    _ => {
                        return Err(DbError::ExecutionError(
                            "DATE_SUBTRACT: amount must be a number".to_string(),
                        ))
                    }
                };

                // Parse the unit
                let unit = new_evaluated_args[2]
                    .as_str()
                    .ok_or_else(|| {
                        DbError::ExecutionError("DATE_SUBTRACT: unit must be a string".to_string())
                    })?
                    .to_lowercase();

                // Parse optional timezone
                let tz: Tz = if new_evaluated_args.len() == 4 {
                    let tz_str = new_evaluated_args[3].as_str().ok_or_else(|| {
                        DbError::ExecutionError(
                            "DATE_SUBTRACT: timezone must be a string".to_string(),
                        )
                    })?;
                    tz_str.parse::<Tz>().map_err(|_| {
                        DbError::ExecutionError(format!(
                            "DATE_SUBTRACT: unknown timezone '{}'",
                            tz_str
                        ))
                    })?
                } else {
                    chrono_tz::UTC
                };

                // Convert to the target timezone for calculation
                let datetime_tz = datetime_utc.with_timezone(&tz);

                // Perform the addition based on unit (amount is already negated)
                let result_tz: DateTime<Tz> = match unit.as_str() {
                    "y" | "year" | "years" => {
                        let new_year = datetime_tz.year() + amount as i32;
                        let naive = chrono::NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(new_year, datetime_tz.month(), datetime_tz.day())
                                .unwrap_or_else(|| {
                                    chrono::NaiveDate::from_ymd_opt(new_year, datetime_tz.month(), 28).unwrap()
                                }),
                            chrono::NaiveTime::from_hms_milli_opt(
                                datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second(),
                                datetime_tz.timestamp_subsec_millis()
                            ).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "m" | "month" | "months" => {
                        let total_months = datetime_tz.year() * 12 + datetime_tz.month() as i32 - 1 + amount as i32;
                        let new_year = total_months / 12;
                        let new_month = (total_months % 12 + 1) as u32;

                        let max_day = chrono::NaiveDate::from_ymd_opt(new_year, new_month + 1, 1)
                            .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(new_year + 1, 1, 1).unwrap())
                            .pred_opt()
                            .unwrap()
                            .day();
                        let new_day = datetime_tz.day().min(max_day);

                        let naive = chrono::NaiveDateTime::new(
                            chrono::NaiveDate::from_ymd_opt(new_year, new_month, new_day).unwrap(),
                            chrono::NaiveTime::from_hms_milli_opt(
                                datetime_tz.hour(), datetime_tz.minute(), datetime_tz.second(),
                                datetime_tz.timestamp_subsec_millis()
                            ).unwrap()
                        );
                        tz.from_local_datetime(&naive)
                            .single()
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: ambiguous or invalid datetime".to_string()
                            ))?
                    }
                    "w" | "week" | "weeks" => {
                        datetime_tz.checked_add_signed(Duration::weeks(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    "d" | "day" | "days" => {
                        datetime_tz.checked_add_signed(Duration::days(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    "h" | "hour" | "hours" => {
                        datetime_tz.checked_add_signed(Duration::hours(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    "i" | "minute" | "minutes" => {
                        datetime_tz.checked_add_signed(Duration::minutes(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    "s" | "second" | "seconds" => {
                        datetime_tz.checked_add_signed(Duration::seconds(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    "f" | "millisecond" | "milliseconds" => {
                        datetime_tz.checked_add_signed(Duration::milliseconds(amount))
                            .ok_or_else(|| DbError::ExecutionError(
                                "DATE_SUBTRACT: date arithmetic overflow".to_string()
                            ))?
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("DATE_SUBTRACT: unknown unit '{}'. Valid units: y/year/years, m/month/months, w/week/weeks, d/day/days, h/hour/hours, i/minute/minutes, s/second/seconds, f/millisecond/milliseconds", unit)
                    )),
                };

                // Convert back to UTC and format as ISO 8601
                let result_utc = result_tz.with_timezone(&Utc);
                let iso_string = result_utc.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                Ok(Value::String(iso_string))
            }

            // DATE_DIFF(date1, date2, unit, asFloat?, timezone1?, timezone2?) - calculate difference between dates
            "DATE_DIFF" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 6 {
                    return Err(DbError::ExecutionError(
                        "DATE_DIFF requires 3-6 arguments: date1, date2, unit, [asFloat], [timezone1], [timezone2]".to_string()
                    ));
                }

                use chrono::{DateTime, Datelike, TimeZone};
                use chrono_tz::Tz;

                // Parse date1 (can be timestamp or ISO string)
                let datetime1_utc: DateTime<Utc> = match &evaluated_args[0] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_DIFF: invalid timestamp for date1".to_string(),
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => {
                                return Err(DbError::ExecutionError(format!(
                                    "DATE_DIFF: invalid timestamp for date1: {}",
                                    timestamp_ms
                                )))
                            }
                        }
                    }
                    Value::String(s) => DateTime::parse_from_rfc3339(s)
                        .map_err(|e| {
                            DbError::ExecutionError(format!(
                                "DATE_DIFF: invalid ISO 8601 date for date1 '{}': {}",
                                s, e
                            ))
                        })?
                        .with_timezone(&Utc),
                    _ => {
                        return Err(DbError::ExecutionError(
                            "DATE_DIFF: date1 must be a timestamp or ISO 8601 string".to_string(),
                        ))
                    }
                };

                // Parse date2 (can be timestamp or ISO string)
                let datetime2_utc: DateTime<Utc> = match &evaluated_args[1] {
                    Value::Number(n) => {
                        let timestamp_ms = if let Some(i) = n.as_i64() {
                            i
                        } else if let Some(f) = n.as_f64() {
                            f as i64
                        } else {
                            return Err(DbError::ExecutionError(
                                "DATE_DIFF: invalid timestamp for date2".to_string(),
                            ));
                        };
                        let secs = timestamp_ms / 1000;
                        let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
                        match Utc.timestamp_opt(secs, nanos) {
                            chrono::LocalResult::Single(dt) => dt,
                            _ => {
                                return Err(DbError::ExecutionError(format!(
                                    "DATE_DIFF: invalid timestamp for date2: {}",
                                    timestamp_ms
                                )))
                            }
                        }
                    }
                    Value::String(s) => DateTime::parse_from_rfc3339(s)
                        .map_err(|e| {
                            DbError::ExecutionError(format!(
                                "DATE_DIFF: invalid ISO 8601 date for date2 '{}': {}",
                                s, e
                            ))
                        })?
                        .with_timezone(&Utc),
                    _ => {
                        return Err(DbError::ExecutionError(
                            "DATE_DIFF: date2 must be a timestamp or ISO 8601 string".to_string(),
                        ))
                    }
                };

                // Parse the unit
                let unit = evaluated_args[2]
                    .as_str()
                    .ok_or_else(|| {
                        DbError::ExecutionError("DATE_DIFF: unit must be a string".to_string())
                    })?
                    .to_lowercase();

                // Parse optional asFloat (default: false)
                let as_float = if evaluated_args.len() >= 4 {
                    evaluated_args[3].as_bool().unwrap_or(false)
                } else {
                    false
                };

                // Parse optional timezones
                let (tz1, tz2) = if evaluated_args.len() >= 5 {
                    let tz1_str = evaluated_args[4].as_str().ok_or_else(|| {
                        DbError::ExecutionError("DATE_DIFF: timezone1 must be a string".to_string())
                    })?;
                    let tz1: Tz = tz1_str.parse().map_err(|_| {
                        DbError::ExecutionError(format!(
                            "DATE_DIFF: unknown timezone1 '{}'",
                            tz1_str
                        ))
                    })?;

                    let tz2 = if evaluated_args.len() >= 6 {
                        let tz2_str = evaluated_args[5].as_str().ok_or_else(|| {
                            DbError::ExecutionError(
                                "DATE_DIFF: timezone2 must be a string".to_string(),
                            )
                        })?;
                        tz2_str.parse::<Tz>().map_err(|_| {
                            DbError::ExecutionError(format!(
                                "DATE_DIFF: unknown timezone2 '{}'",
                                tz2_str
                            ))
                        })?
                    } else {
                        tz1 // If timezone2 not specified, use timezone1 for both
                    };

                    (tz1, tz2)
                } else {
                    (chrono_tz::UTC, chrono_tz::UTC)
                };

                // Convert dates to their respective timezones
                let datetime1_tz = datetime1_utc.with_timezone(&tz1);
                let datetime2_tz = datetime2_utc.with_timezone(&tz2);

                // Calculate the difference based on unit
                let diff: f64 = match unit.as_str() {
                    "y" | "year" | "years" => {
                        // Calculate year difference
                        let year_diff = datetime2_tz.year() - datetime1_tz.year();
                        if as_float {
                            // More precise calculation considering months and days
                            let month_diff = datetime2_tz.month() as i32 - datetime1_tz.month() as i32;
                            let day_diff = datetime2_tz.day() as i32 - datetime1_tz.day() as i32;
                            year_diff as f64 + (month_diff as f64 / 12.0) + (day_diff as f64 / 365.25)
                        } else {
                            year_diff as f64
                        }
                    }
                    "m" | "month" | "months" => {
                        // Calculate month difference
                        let total_months1 = datetime1_tz.year() * 12 + datetime1_tz.month() as i32;
                        let total_months2 = datetime2_tz.year() * 12 + datetime2_tz.month() as i32;
                        let month_diff = total_months2 - total_months1;
                        if as_float {
                            // Add fractional part based on days
                            let day_diff = datetime2_tz.day() as i32 - datetime1_tz.day() as i32;
                            month_diff as f64 + (day_diff as f64 / 30.0)
                        } else {
                            month_diff as f64
                        }
                    }
                    "w" | "week" | "weeks" => {
                        // Calculate week difference using milliseconds
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        let weeks = diff_ms as f64 / (7.0 * 24.0 * 60.0 * 60.0 * 1000.0);
                        if as_float {
                            weeks
                        } else {
                            weeks.trunc()
                        }
                    }
                    "d" | "day" | "days" => {
                        // Calculate day difference using milliseconds
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        let days = diff_ms as f64 / (24.0 * 60.0 * 60.0 * 1000.0);
                        if as_float {
                            days
                        } else {
                            days.trunc()
                        }
                    }
                    "h" | "hour" | "hours" => {
                        // Calculate hour difference using milliseconds
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        let hours = diff_ms as f64 / (60.0 * 60.0 * 1000.0);
                        if as_float {
                            hours
                        } else {
                            hours.trunc()
                        }
                    }
                    "i" | "minute" | "minutes" => {
                        // Calculate minute difference using milliseconds
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        let minutes = diff_ms as f64 / (60.0 * 1000.0);
                        if as_float {
                            minutes
                        } else {
                            minutes.trunc()
                        }
                    }
                    "s" | "second" | "seconds" => {
                        // Calculate second difference using milliseconds
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        let seconds = diff_ms as f64 / 1000.0;
                        if as_float {
                            seconds
                        } else {
                            seconds.trunc()
                        }
                    }
                    "f" | "millisecond" | "milliseconds" => {
                        // Calculate millisecond difference
                        let diff_ms = datetime2_utc.timestamp_millis() - datetime1_utc.timestamp_millis();
                        diff_ms as f64
                    }
                    _ => return Err(DbError::ExecutionError(
                        format!("DATE_DIFF: unknown unit '{}'. Valid units: y/year/years, m/month/months, w/week/weeks, d/day/days, h/hour/hours, i/minute/minutes, s/second/seconds, f/millisecond/milliseconds", unit)
                    )),
                };

                Ok(Value::Number(number_from_f64(diff)))
            }

            _ => Err(DbError::ExecutionError(format!(
                "Unknown function: {}",
                name
            ))),
        }
    }

    // ==================== Index Optimization (for single FOR queries) ====================

    /// Try to use index for single-FOR queries
    #[allow(dead_code)]
    fn get_indexed_documents(
        &self,
        collection: &Collection,
        filter_clauses: &[FilterClause],
        var_name: &str,
    ) -> Option<Vec<Value>> {
        for filter in filter_clauses {
            if let Some(condition) = self.extract_indexable_condition(&filter.expression, var_name)
            {
                if let Some(docs) = self.use_index_for_condition(collection, &condition) {
                    return Some(docs.iter().map(|d| d.to_value()).collect());
                }
            }
        }
        None
    }

    /// Extract a simple indexable condition from a filter expression
    fn extract_indexable_condition(
        &self,
        expr: &Expression,
        var_name: &str,
    ) -> Option<IndexableCondition> {
        match expr {
            Expression::BinaryOp { left, op, right } => {
                match op {
                    BinaryOperator::Equal
                    | BinaryOperator::LessThan
                    | BinaryOperator::LessThanOrEqual
                    | BinaryOperator::GreaterThan
                    | BinaryOperator::GreaterThanOrEqual => {
                        // Try left = field access, right = literal
                        if let Some(field) = self.extract_field_path(left, var_name) {
                            if let Expression::Literal(value) = right.as_ref() {
                                return Some(IndexableCondition {
                                    field,
                                    op: op.clone(),
                                    value: value.clone(),
                                });
                            }
                        }
                        // Try right = field access, left = literal
                        if let Some(field) = self.extract_field_path(right, var_name) {
                            if let Expression::Literal(value) = left.as_ref() {
                                let reversed_op = match op {
                                    BinaryOperator::LessThan => BinaryOperator::GreaterThan,
                                    BinaryOperator::LessThanOrEqual => {
                                        BinaryOperator::GreaterThanOrEqual
                                    }
                                    BinaryOperator::GreaterThan => BinaryOperator::LessThan,
                                    BinaryOperator::GreaterThanOrEqual => {
                                        BinaryOperator::LessThanOrEqual
                                    }
                                    other => other.clone(),
                                };
                                return Some(IndexableCondition {
                                    field,
                                    op: reversed_op,
                                    value: value.clone(),
                                });
                            }
                        }
                    }
                    BinaryOperator::And => {
                        if let Some(cond) = self.extract_indexable_condition(left, var_name) {
                            return Some(cond);
                        }
                        return self.extract_indexable_condition(right, var_name);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        None
    }

    /// Extract field path from an expression
    fn extract_field_path(&self, expr: &Expression, var_name: &str) -> Option<String> {
        match expr {
            Expression::FieldAccess(base, field) => {
                if let Expression::Variable(name) = base.as_ref() {
                    if name == var_name {
                        return Some(field.clone());
                    }
                }
                if let Some(base_path) = self.extract_field_path(base, var_name) {
                    return Some(format!("{}.{}", base_path, field));
                }
                None
            }
            _ => None,
        }
    }

    /// Use index for a condition lookup
    fn use_index_for_condition(
        &self,
        collection: &Collection,
        condition: &IndexableCondition,
    ) -> Option<Vec<crate::storage::Document>> {
        // Normalize the value for index lookup
        // If it's a float that's actually an integer (e.g., 30.0), convert to integer
        // This handles the case where SDBQL parses "30" as 30.0 but data has integer 30
        let normalized_value = if let Value::Number(n) = &condition.value {
            if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.is_finite() {
                    // It's a whole number, try as integer first
                    Value::Number(serde_json::Number::from(f as i64))
                } else {
                    condition.value.clone()
                }
            } else {
                condition.value.clone()
            }
        } else {
            condition.value.clone()
        };

        match condition.op {
            BinaryOperator::Equal => {
                // Try with normalized value first
                if let Some(docs) = collection.index_lookup_eq(&condition.field, &normalized_value)
                {
                    if !docs.is_empty() {
                        return Some(docs);
                    }
                }
                // Fall back to original value
                collection.index_lookup_eq(&condition.field, &condition.value)
            }
            BinaryOperator::GreaterThan => {
                collection.index_lookup_gt(&condition.field, &normalized_value)
            }
            BinaryOperator::GreaterThanOrEqual => {
                collection.index_lookup_gte(&condition.field, &normalized_value)
            }
            BinaryOperator::LessThan => {
                collection.index_lookup_lt(&condition.field, &normalized_value)
            }
            BinaryOperator::LessThanOrEqual => {
                collection.index_lookup_lte(&condition.field, &normalized_value)
            }
            _ => None,
        }
    }
}





#[inline]
fn get_field_value(value: &Value, field_path: &str) -> Value {
    let mut current = value;

    for part in field_path.split('.') {
        match current.get(part) {
            Some(val) => current = val,
            None => return Value::Null,
        }
    }

    current.clone()
}

#[inline]
fn values_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        _ => left == right,
    }
}

#[inline]
fn evaluate_binary_op(left: &Value, op: &BinaryOperator, right: &Value) -> DbResult<Value> {
    match op {
        BinaryOperator::Equal => Ok(Value::Bool(values_equal(left, right))),
        BinaryOperator::NotEqual => Ok(Value::Bool(!values_equal(left, right))),

        BinaryOperator::LessThan => Ok(Value::Bool(
            compare_values(left, right) == std::cmp::Ordering::Less,
        )),
        BinaryOperator::LessThanOrEqual => Ok(Value::Bool(
            compare_values(left, right) != std::cmp::Ordering::Greater,
        )),
        BinaryOperator::GreaterThan => Ok(Value::Bool(
            compare_values(left, right) == std::cmp::Ordering::Greater,
        )),
        BinaryOperator::GreaterThanOrEqual => Ok(Value::Bool(
            compare_values(left, right) != std::cmp::Ordering::Less,
        )),
        BinaryOperator::In => {
            match right {
                Value::Array(arr) => {
                    let mut found = false;
                    for val in arr {
                        if values_equal(left, val) {
                            found = true;
                            break;
                        }
                    }
                    Ok(Value::Bool(found))
                }
                Value::Object(obj) => {
                    if let Some(s) = left.as_str() {
                        Ok(Value::Bool(obj.contains_key(s)))
                    } else {
                        Ok(Value::Bool(false))
                    }
                }
                _ => Ok(Value::Bool(false)),
            }
        }

        BinaryOperator::Like | BinaryOperator::NotLike => {
            let s = left.as_str().unwrap_or("");
            let pattern = right.as_str().unwrap_or("");

            // Convert SQL LIKE pattern to Regex
            // Escape regex characters
            let mut regex_pattern = String::new();
            regex_pattern.push('^');
            for c in pattern.chars() {
                match c {
                    '%' => regex_pattern.push_str(".*"),
                    '_' => regex_pattern.push('.'),
                    '^' | '$' | '.' | '*' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' => {
                        regex_pattern.push('\\');
                        regex_pattern.push(c);
                    }
                    _ => regex_pattern.push(c),
                }
            }
            regex_pattern.push('$');

            match regex::Regex::new(&regex_pattern) {
                Ok(re) => {
                    let is_match = re.is_match(s);
                    if matches!(op, BinaryOperator::NotLike) {
                        Ok(Value::Bool(!is_match))
                    } else {
                        Ok(Value::Bool(is_match))
                    }
                }
                Err(_) => Ok(Value::Bool(false)), // Invalid regex (shouldn't happen with escaped pattern)
            }
        }

        BinaryOperator::RegEx | BinaryOperator::NotRegEx => {
            let s = left.as_str().unwrap_or("");
            let pattern = right.as_str().unwrap_or("");

            match regex::Regex::new(pattern) {
                Ok(re) => {
                    let is_match = re.is_match(s);
                    if matches!(op, BinaryOperator::NotRegEx) {
                        Ok(Value::Bool(!is_match))
                    } else {
                        Ok(Value::Bool(is_match))
                    }
                }
                Err(_) => Ok(Value::Bool(false)), // Invalid regex results in false
            }
        }

        BinaryOperator::And => Ok(Value::Bool(to_bool(left) && to_bool(right))),
        BinaryOperator::Or => Ok(Value::Bool(to_bool(left) || to_bool(right))),

        BinaryOperator::Add => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(a + b)))
            } else if let (Some(a), Some(b)) = (left.as_str(), right.as_str()) {
                Ok(Value::String(format!("{}{}", a, b)))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot add these types".to_string(),
                ))
            }
        }

        BinaryOperator::Subtract => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(a - b)))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot subtract non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Multiply => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(a * b)))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot multiply non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Divide => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                if b == 0.0 {
                    Err(DbError::ExecutionError("Division by zero".to_string()))
                } else {
                    Ok(Value::Number(number_from_f64(a / b)))
                }
            } else {
                Err(DbError::ExecutionError(
                    "Cannot divide non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Modulus => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                if b == 0.0 {
                    Err(DbError::ExecutionError("Division by zero".to_string()))
                } else {
                    Ok(Value::Number(number_from_f64(a % b)))
                }
            } else {
                Err(DbError::ExecutionError(
                    "Cannot modulus non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::BitwiseAnd => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from(
                    (a as i64) & (b as i64)
                )))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot bitwise AND non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::BitwiseOr => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from(
                    (a as i64) | (b as i64)
                )))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot bitwise OR non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::BitwiseXor => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from(
                    (a as i64) ^ (b as i64)
                )))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot bitwise XOR non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::LeftShift => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from(
                    (a as i64) << (b as i64)
                )))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot left shift non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::RightShift => {
            if let (Some(a), Some(b)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(serde_json::Number::from(
                    (a as i64) >> (b as i64)
                )))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot right shift non-numbers".to_string(),
                ))
            }
        }

        BinaryOperator::Exponent => {
            if let (Some(base), Some(exp)) = (left.as_f64(), right.as_f64()) {
                Ok(Value::Number(number_from_f64(base.powf(exp))))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot exponentiate non-numbers".to_string(),
                ))
            }
        }
    }
}

#[inline]
fn evaluate_unary_op(op: &UnaryOperator, operand: &Value) -> DbResult<Value> {
    match op {
        UnaryOperator::Not => Ok(Value::Bool(!to_bool(operand))),
        UnaryOperator::Negate => {
            if let Some(n) = operand.as_f64() {
                Ok(Value::Number(number_from_f64(-n)))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot negate non-number".to_string(),
                ))
            }
        }
        UnaryOperator::BitwiseNot => {
            if let Some(n) = operand.as_f64() {
                Ok(Value::Number(serde_json::Number::from(!(n as i64))))
            } else {
                Err(DbError::ExecutionError(
                    "Cannot bitwise NOT non-number".to_string(),
                ))
            }
        }
    }
}

#[inline]
fn to_bool(value: &Value) -> bool {
    match value {
        Value::Bool(b) => *b,
        Value::Null => false,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

#[inline]
fn compare_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;

    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(a), Value::Number(b)) => {
            let a_f64 = a.as_f64().unwrap_or(0.0);
            let b_f64 = b.as_f64().unwrap_or(0.0);
            a_f64.partial_cmp(&b_f64).unwrap_or(Ordering::Equal)
        }
        (Value::String(a), Value::String(b)) => a.cmp(b),
        _ => Ordering::Equal,
    }
}
