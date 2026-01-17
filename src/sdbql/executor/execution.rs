//! Query execution implementation for SDBQL executor.
//!
//! This module contains the main execution logic:
//! - execute: Main execution entry point
//! - execute_with_stats: Execution with mutation statistics
//! - execute_body_clauses: Process body clauses (FOR, LET, FILTER, etc.)

use std::collections::HashMap;
use std::time::Instant;

use serde_json::Value;

use super::types::{Context, MutationStats, QueryExecutionResult};
use super::window::contains_window_functions;
use super::{compare_values, get_field_value, to_bool, QueryExecutor};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::*;
use crate::sync::log::LogEntry;
use crate::sync::protocol::Operation;

impl<'a> QueryExecutor<'a> {
    pub(super) fn try_streaming_bulk_insert(
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
                tracing::debug!(
                    "Streaming insert disabled for sharded collection: {}",
                    insert_clause.collection
                );
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
                ctx.insert(var_name.clone(), Value::Number(serde_json::Number::from(i)));
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
    pub(super) fn log_mutation(
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
    pub(super) fn log_mutations_async(
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
    pub fn execute(&self, query: &Query) -> DbResult<Vec<Value>> {
        let result = self.execute_with_stats(query)?;
        Ok(result.results)
    }

    /// Execute query and return full results with mutation statistics
    pub fn execute_with_stats(&self, query: &Query) -> DbResult<QueryExecutionResult> {
        // Handle CREATE MATERIALIZED VIEW
        if let Some(ref clause) = query.create_materialized_view_clause {
            return self.execute_create_materialized_view(clause);
        }

        // Handle REFRESH MATERIALIZED VIEW
        if let Some(ref clause) = query.refresh_materialized_view_clause {
            return self.execute_refresh_materialized_view(clause);
        }

        // First, evaluate initial LET clauses (before any FOR) to create initial binding
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
        if let Some((results, insert_count)) =
            self.try_streaming_bulk_insert(query, &initial_bindings)?
        {
            return Ok(QueryExecutionResult {
                results,
                mutations: MutationStats {
                    documents_inserted: insert_count,
                    documents_updated: 0,
                    documents_removed: 0,
                },
            });
        }

        // Optimization: Columnar aggregation queries
        // Pattern: FOR x IN columnar_collection COLLECT AGGREGATE ... RETURN ...
        if let Some(results) = self.try_columnar_aggregation(query, &initial_bindings)? {
            return Ok(QueryExecutionResult {
                results,
                mutations: MutationStats::new(),
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
                    let limit_offset = self
                        .evaluate_expr_with_context(&limit.offset, &initial_bindings)
                        .ok()
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(0);
                    let limit_count = self
                        .evaluate_expr_with_context(&limit.count, &initial_bindings)
                        .ok()
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(0);

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

                                        let results =
                                            if let Some(ref return_clause) = query.return_clause {
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
                    let offset = self
                        .evaluate_expr_with_context(&l.offset, &initial_bindings)
                        .ok()
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(0);
                    let count = self
                        .evaluate_expr_with_context(&l.count, &initial_bindings)
                        .ok()
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(0);
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
                        return if *ascending { cmp } else { cmp.reverse() };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }

        // Apply window functions if RETURN clause contains any
        if let Some(ref return_clause) = query.return_clause {
            if contains_window_functions(&return_clause.expression) {
                rows = self.apply_window_functions(rows, &return_clause.expression)?;
            }
        }

        // Apply LIMIT
        if let Some(limit) = &query.limit_clause {
            let offset = self
                .evaluate_expr_with_context(&limit.offset, &initial_bindings)
                .ok()
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);
            let count = self
                .evaluate_expr_with_context(&limit.count, &initial_bindings)
                .ok()
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);

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
    pub(super) fn execute_body_clauses(
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
                    if let (Some(config), Some(coordinator)) =
                        (collection.get_shard_config(), &self.shard_coordinator)
                    {
                        if config.num_shards > 0 {
                            tracing::info!(
                                "INSERT: Using ShardCoordinator BATCH for {} documents into {}",
                                rows.len(),
                                insert_clause.collection
                            );

                            // Evaluate all documents first
                            let mut documents = Vec::with_capacity(rows.len());
                            for ctx in &rows {
                                let doc_value =
                                    self.evaluate_expr_with_context(&insert_clause.document, ctx)?;
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
                                let res = coord
                                    .insert_batch(&db_name, &coll_name, &config, documents)
                                    .await;
                                let _ = tx.send(res);
                            });

                            // Wait for batch result
                            let result = rx.recv().map_err(|_| {
                                DbError::InternalError("Sharded batch insert failed".to_string())
                            })??;
                            tracing::debug!(
                                "INSERT: Sharded batch completed - {} success, {} failed",
                                result.0,
                                result.1
                            );
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
                    if let (Some(config), Some(coordinator)) =
                        (collection.get_shard_config(), &self.shard_coordinator)
                    {
                        if config.num_shards > 0 {
                            tracing::debug!(
                                "UPDATE: Delegating to ShardCoordinator for {}",
                                update_clause.collection
                            );
                            let handle = tokio::runtime::Handle::current();
                            let db_name = self.database.as_deref().unwrap_or("_system").to_string();
                            let coll_name = update_clause.collection.clone();
                            let config = config.clone();

                            for ctx in &mut rows {
                                // Evaluate selector (Duplicated logic)
                                let selector_value =
                                    self.evaluate_expr_with_context(&update_clause.selector, ctx)?;
                                let key = match &selector_value {
                                    Value::String(s) => s.clone(),
                                    Value::Object(obj) => obj
                                        .get("_key")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string())
                                        .ok_or_else(|| {
                                            DbError::ExecutionError(
                                                "UPDATE: missing _key".to_string(),
                                            )
                                        })?,
                                    _ => {
                                        return Err(DbError::ExecutionError(
                                            "UPDATE: invalid selector".to_string(),
                                        ))
                                    }
                                };
                                let changes =
                                    self.evaluate_expr_with_context(&update_clause.changes, ctx)?;
                                if !changes.is_object() {
                                    return Err(DbError::ExecutionError(
                                        "UPDATE: changes must be object".to_string(),
                                    ));
                                }

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
                                let updated_doc = rx.recv().map_err(|_| {
                                    DbError::InternalError("Sharded update task failed".to_string())
                                })??;
                                stats.documents_updated += 1;

                                // Inject NEW variable
                                ctx.insert("NEW".to_string(), updated_doc.clone());
                            }
                            i += 1; // CRITICAL: Advance to next clause
                            continue;
                        }
                    }

                    // Non-sharded UPDATE: Use automatic batching for large updates (>100 rows)
                    let bulk_mode = rows.len() > 100;

                    if bulk_mode {
                        // AUTOMATIC BATCH MODE - use update_batch() like INSERT uses insert_batch()
                        tracing::debug!(
                            "UPDATE: Bulk mode for {} rows (threshold: 100)",
                            rows.len()
                        );

                        // Evaluate all updates first
                        let eval_start = std::time::Instant::now();
                        let mut updates: Vec<(String, Value)> = Vec::with_capacity(rows.len());

                        for ctx in &rows {
                            // Evaluate selector expression to get the document key
                            let selector_value =
                                self.evaluate_expr_with_context(&update_clause.selector, ctx)?;

                            // Extract _key from selector
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

                            updates.push((key, changes_value));
                        }
                        let eval_time = eval_start.elapsed();
                        tracing::debug!("UPDATE: Evaluation took {:?}", eval_time);

                        // Batch update all documents at once (uses RocksDB WriteBatch)
                        let update_start = std::time::Instant::now();
                        let updated_docs = collection.update_batch(&updates)?;
                        let update_time = update_start.elapsed();
                        stats.documents_updated += updated_docs.len();
                        tracing::debug!(
                            "UPDATE: Batch update of {} docs took {:?}",
                            updated_docs.len(),
                            update_time
                        );

                        // Log to replication asynchronously for bulk updates
                        self.log_mutations_async(
                            &update_clause.collection,
                            Operation::Update,
                            &updated_docs,
                        );
                    } else {
                        // STANDARD MODE (<=100 rows) - update individually
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
                }
                BodyClause::Remove(remove_clause) => {
                    // Get collection once, outside the loop
                    let collection = self.get_collection(&remove_clause.collection)?;

                    // SHARDING SUPPORT
                    if let (Some(config), Some(coordinator)) =
                        (collection.get_shard_config(), &self.shard_coordinator)
                    {
                        if config.num_shards > 0 {
                            tracing::debug!(
                                "REMOVE: Delegating to ShardCoordinator for {}",
                                remove_clause.collection
                            );
                            let handle = tokio::runtime::Handle::current();
                            let db_name = self.database.as_deref().unwrap_or("_system").to_string();
                            let coll_name = remove_clause.collection.clone();
                            let config = config.clone();

                            for ctx in &rows {
                                // Evaluate selector (Duplicated logic)
                                let selector_value =
                                    self.evaluate_expr_with_context(&remove_clause.selector, ctx)?;
                                let key = match &selector_value {
                                    Value::String(s) => s.clone(),
                                    Value::Object(obj) => obj
                                        .get("_key")
                                        .and_then(|v| v.as_str())
                                        .map(|s| s.to_string())
                                        .ok_or_else(|| {
                                            DbError::ExecutionError(
                                                "REMOVE: missing _key".to_string(),
                                            )
                                        })?,
                                    _ => {
                                        return Err(DbError::ExecutionError(
                                            "REMOVE: invalid selector".to_string(),
                                        ))
                                    }
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
                                let _ = rx.recv().map_err(|_| {
                                    DbError::InternalError("Sharded remove task failed".to_string())
                                })??;
                                stats.documents_removed += 1;
                            }
                            i += 1; // CRITICAL: Advance to next clause
                            continue;
                        }
                    }

                    // Non-sharded REMOVE: Use automatic batching for large removes (>100 rows)
                    let bulk_mode = rows.len() > 100;

                    if bulk_mode {
                        // AUTOMATIC BATCH MODE - use delete_batch() like INSERT uses insert_batch()
                        tracing::debug!(
                            "REMOVE: Bulk mode for {} rows (threshold: 100)",
                            rows.len()
                        );

                        // Evaluate all keys first
                        let eval_start = std::time::Instant::now();
                        let mut keys: Vec<String> = Vec::with_capacity(rows.len());

                        for ctx in &rows {
                            // Evaluate selector expression to get the document key
                            let selector_value =
                                self.evaluate_expr_with_context(&remove_clause.selector, ctx)?;

                            // Extract _key from selector
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

                            keys.push(key);
                        }
                        let eval_time = eval_start.elapsed();
                        tracing::debug!("REMOVE: Evaluation took {:?}", eval_time);

                        // Batch delete all documents at once (uses RocksDB WriteBatch)
                        let delete_start = std::time::Instant::now();
                        let deleted_count = collection.delete_batch(&keys)?;
                        let delete_time = delete_start.elapsed();
                        stats.documents_removed += deleted_count;
                        tracing::debug!(
                            "REMOVE: Batch delete of {} docs took {:?}",
                            deleted_count,
                            delete_time
                        );

                        // Log to replication (keys only for deletes)
                        for key in &keys {
                            self.log_mutation(
                                &remove_clause.collection,
                                Operation::Delete,
                                key,
                                None,
                            );
                        }
                    } else {
                        // STANDARD MODE (<=100 rows) - delete individually
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
                            self.log_mutation(
                                &remove_clause.collection,
                                Operation::Delete,
                                &key,
                                None,
                            );
                        }
                    }
                }
                BodyClause::Upsert(upsert_clause) => {
                    let collection = self.get_collection(&upsert_clause.collection)?;

                    for ctx in &mut rows {
                        let search_value =
                            self.evaluate_expr_with_context(&upsert_clause.search, ctx)?;

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
                            let update_value =
                                self.evaluate_expr_with_context(&upsert_clause.update, ctx)?;
                            if !update_value.is_object() {
                                return Err(DbError::ExecutionError(
                                    "UPSERT: update expression must be an object".to_string(),
                                ));
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
                            let insert_value =
                                self.evaluate_expr_with_context(&upsert_clause.insert, ctx)?;
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
                            _ => {
                                return Err(DbError::ExecutionError(
                                    "Start vertex must be a string (e.g., 'users/alice')"
                                        .to_string(),
                                ))
                            }
                        };

                        // Get edge collection
                        let edge_collection = self.get_collection(&gt.edge_collection)?;

                        // BFS traversal
                        let mut visited: std::collections::HashSet<String> =
                            std::collections::HashSet::new();
                        let mut queue: std::collections::VecDeque<(String, usize, Option<Value>)> =
                            std::collections::VecDeque::new();
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
                                            new_ctx.insert(
                                                gt.vertex_var.clone(),
                                                vertex_doc.to_value(),
                                            );
                                            if let Some(ref edge_var) = gt.edge_var {
                                                new_ctx.insert(
                                                    edge_var.clone(),
                                                    edge.clone().unwrap_or(Value::Null),
                                                );
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
                                        } else {
                                            None
                                        }
                                    }
                                    EdgeDirection::Inbound => {
                                        if to == Some(current_id.as_str()) {
                                            from.map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                    EdgeDirection::Any => {
                                        if from == Some(current_id.as_str()) {
                                            to.map(|s| s.to_string())
                                        } else if to == Some(current_id.as_str()) {
                                            from.map(|s| s.to_string())
                                        } else {
                                            None
                                        }
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
                            _ => {
                                return Err(DbError::ExecutionError(
                                    "Start vertex must be a string".to_string(),
                                ))
                            }
                        };

                        let end_value = self.evaluate_expr_with_context(&sp.end_vertex, ctx)?;
                        let end_id = match &end_value {
                            Value::String(s) => s.clone(),
                            _ => {
                                return Err(DbError::ExecutionError(
                                    "End vertex must be a string".to_string(),
                                ))
                            }
                        };

                        let edge_collection = self.get_collection(&sp.edge_collection)?;

                        // BFS with parent tracking
                        let mut visited: std::collections::HashMap<
                            String,
                            (Option<String>, Option<Value>),
                        > = std::collections::HashMap::new();
                        let mut queue: std::collections::VecDeque<String> =
                            std::collections::VecDeque::new();

                        visited.insert(start_id.clone(), (None, None));
                        queue.push_back(start_id.clone());
                        let mut found = false;

                        while let Some(current_id) = queue.pop_front() {
                            if current_id == end_id {
                                found = true;
                                break;
                            }

                            let edges = edge_collection.scan(None);
                            for edge_doc in edges {
                                let edge_val = edge_doc.to_value();
                                let from = edge_val.get("_from").and_then(|v| v.as_str());
                                let to = edge_val.get("_to").and_then(|v| v.as_str());

                                let next_id = match sp.direction {
                                    EdgeDirection::Outbound => {
                                        if from == Some(current_id.as_str()) {
                                            to.map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                    EdgeDirection::Inbound => {
                                        if to == Some(current_id.as_str()) {
                                            from.map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                    EdgeDirection::Any => {
                                        if from == Some(current_id.as_str()) {
                                            to.map(|s| s.to_string())
                                        } else if to == Some(current_id.as_str()) {
                                            from.map(|s| s.to_string())
                                        } else {
                                            None
                                        }
                                    }
                                };

                                if let Some(next) = next_id {
                                    if !visited.contains_key(&next) {
                                        visited.insert(
                                            next.clone(),
                                            (Some(current_id.clone()), Some(edge_val.clone())),
                                        );
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
                                if let Some(p) = parent {
                                    current = p.clone();
                                } else {
                                    break;
                                }
                            }
                            path.reverse();

                            for (vertex_id, edge) in path {
                                if let Some((coll_name, key)) = vertex_id.split_once('/') {
                                    if let Ok(vertex_coll) = self.get_collection(coll_name) {
                                        if let Ok(vertex_doc) = vertex_coll.get(key) {
                                            let mut new_ctx = ctx.clone();
                                            new_ctx.insert(
                                                sp.vertex_var.clone(),
                                                vertex_doc.to_value(),
                                            );
                                            if let Some(ref edge_var) = sp.edge_var {
                                                new_ctx.insert(
                                                    edge_var.clone(),
                                                    edge.unwrap_or(Value::Null),
                                                );
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

                BodyClause::Window(_) => {
                    return Err(DbError::ExecutionError(
                        "Window operations are only supported in STREAM definitions".to_string(),
                    ));
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

                        let entry = groups
                            .entry(group_key)
                            .or_insert_with(|| (group_ctx.clone(), Vec::new(), 0));

                        // Collect into groups
                        entry.1.push(ctx.clone());
                        entry.2 += 1;
                    }

                    // Build result rows from groups
                    let mut new_rows = Vec::new();

                    for (_key, (mut group_ctx, group_docs, count)) in groups {
                        // Add INTO variable if present
                        if let Some(ref into_var) = collect.into_var {
                            let group_array: Vec<Value> = group_docs
                                .iter()
                                .map(|ctx| {
                                    // Create an object with all variables in the context
                                    let obj: serde_json::Map<String, Value> =
                                        ctx.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
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
                            let agg_value =
                                self.compute_aggregate(&agg.function, &agg.argument, &group_docs)?;
                            group_ctx.insert(agg.variable.clone(), agg_value);
                        }

                        new_rows.push(group_ctx);
                    }

                    rows = new_rows;
                }

                BodyClause::Join(join_clause) => {
                    // Execute JOIN using appropriate strategy based on join type
                    let collection = self.get_collection(&join_clause.collection)?;

                    match join_clause.join_type {
                        JoinType::Inner | JoinType::Left => {
                            // Standard LEFT/INNER JOIN: iterate left side, find matches on right
                            let mut new_rows = Vec::new();

                            for ctx in &rows {
                                // Get all documents from joined collection
                                let all_docs: Vec<Value> = collection
                                    .scan(None)
                                    .into_iter()
                                    .map(|doc| doc.to_value())
                                    .collect();

                                // Find matching documents by evaluating join condition
                                let mut matches = Vec::new();
                                for doc in all_docs {
                                    let mut temp_ctx = ctx.clone();
                                    temp_ctx.insert(join_clause.variable.clone(), doc.clone());

                                    if let Ok(result) = self.evaluate_expr_with_context(
                                        &join_clause.condition,
                                        &temp_ctx,
                                    ) {
                                        if result.as_bool().unwrap_or(false) {
                                            matches.push(doc);
                                        }
                                    }
                                }

                                // Handle INNER vs LEFT
                                match join_clause.join_type {
                                    JoinType::Inner => {
                                        if !matches.is_empty() {
                                            let mut new_ctx = ctx.clone();
                                            new_ctx.insert(
                                                join_clause.variable.clone(),
                                                Value::Array(matches),
                                            );
                                            new_rows.push(new_ctx);
                                        }
                                    }
                                    JoinType::Left => {
                                        let mut new_ctx = ctx.clone();
                                        new_ctx.insert(
                                            join_clause.variable.clone(),
                                            Value::Array(matches),
                                        );
                                        new_rows.push(new_ctx);
                                    }
                                    _ => unreachable!(),
                                }
                            }
                            rows = new_rows;
                        }

                        JoinType::Right => {
                            // RIGHT JOIN: iterate right side, find matching left rows
                            // Keep all right rows, group left matches into array
                            let mut new_rows = Vec::new();
                            let all_right_docs: Vec<Value> = collection
                                .scan(None)
                                .into_iter()
                                .map(|doc| doc.to_value())
                                .collect();

                            for right_doc in all_right_docs {
                                // Find matching left rows for this right doc
                                let mut left_matches = Vec::new();
                                for left_ctx in &rows {
                                    // Check if left row matches this right doc
                                    let mut temp_ctx = left_ctx.clone();
                                    temp_ctx
                                        .insert(join_clause.variable.clone(), right_doc.clone());

                                    if let Ok(result) = self.evaluate_expr_with_context(
                                        &join_clause.condition,
                                        &temp_ctx,
                                    ) {
                                        if result.as_bool().unwrap_or(false) {
                                            // Convert left context to Value for grouping
                                            left_matches.push(
                                                serde_json::to_value(left_ctx).unwrap_or(
                                                    Value::Object(serde_json::Map::new()),
                                                ),
                                            );
                                        }
                                    }
                                }

                                // Create result: right doc + array of matching left rows
                                //  This mirrors LEFT JOIN behavior but from right perspective
                                let mut new_ctx = std::collections::HashMap::new();
                                new_ctx.insert(join_clause.variable.clone(), right_doc);

                                // For RIGHT JOIN, we need a way to access left-side data
                                // Since we don't have a specific variable for it, we'll flatten the first match
                                // and put the rest in an array if there are multiple matches
                                if !left_matches.is_empty() {
                                    // Merge fields from first left match
                                    if let Value::Object(map) = &left_matches[0] {
                                        for (key, value) in map.iter() {
                                            new_ctx.insert(key.clone(), value.clone());
                                        }
                                    }
                                }
                                new_rows.push(new_ctx);
                            }
                            rows = new_rows;
                        }

                        JoinType::FullOuter => {
                            // FULL OUTER JOIN: combination of LEFT and RIGHT
                            let mut new_rows = Vec::new();
                            let mut matched_right_indices = std::collections::HashSet::new();

                            let all_right_docs: Vec<Value> = collection
                                .scan(None)
                                .into_iter()
                                .map(|doc| doc.to_value())
                                .collect();

                            // Phase 1: LEFT JOIN part - iterate left, find right matches
                            for ctx in &rows {
                                let mut matches = Vec::new();
                                for (idx, doc) in all_right_docs.iter().enumerate() {
                                    let mut temp_ctx = ctx.clone();
                                    temp_ctx.insert(join_clause.variable.clone(), doc.clone());

                                    if let Ok(result) = self.evaluate_expr_with_context(
                                        &join_clause.condition,
                                        &temp_ctx,
                                    ) {
                                        if result.as_bool().unwrap_or(false) {
                                            matches.push(doc.clone());
                                            matched_right_indices.insert(idx);
                                        }
                                    }
                                }

                                // Always include left row (LEFT JOIN semantics)
                                let mut new_ctx = ctx.clone();
                                new_ctx.insert(join_clause.variable.clone(), Value::Array(matches));
                                new_rows.push(new_ctx);
                            }

                            // Phase 2: Add unmatched right rows (RIGHT JOIN part)
                            for (idx, right_doc) in all_right_docs.iter().enumerate() {
                                if !matched_right_indices.contains(&idx) {
                                    let mut new_ctx = std::collections::HashMap::new();
                                    new_ctx.insert(join_clause.variable.clone(), right_doc.clone());
                                    new_rows.push(new_ctx);
                                }
                            }

                            rows = new_rows;
                        }
                    }
                }
            }
            i += 1;
        }

        Ok((rows, stats))
    }
    pub(super) fn execute_with_parent_context(
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
                        return if *ascending { cmp } else { cmp.reverse() };
                    }
                }
                std::cmp::Ordering::Equal
            });
        }

        // Apply LIMIT
        if let Some(limit) = &query.limit_clause {
            let offset = self
                .evaluate_expr_with_context(&limit.offset, &initial_bindings)
                .ok()
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);
            let count = self
                .evaluate_expr_with_context(&limit.count, &initial_bindings)
                .ok()
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(0);

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
}
