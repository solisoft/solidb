//! Body clause execution for SDBQL executor.
//!
//! This module contains execute_body_clauses which handles:
//! - FOR clauses with index optimization
//! - LET clauses
//! - FILTER clauses
//! - JOIN clauses (INNER, LEFT, RIGHT, FULL OUTER)
//! - Mutation clauses (INSERT, UPDATE, REMOVE, UPSERT)
//! - COLLECT (GROUP BY) clauses
//! - Graph traversal and shortest path
//! - Stream clauses

use serde_json::Value;

use super::super::types::{Context, MutationStats};
use super::super::QueryExecutor;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::*;
use crate::sync::protocol::Operation;

/// Block on the result of a sharded mutation that was spawned onto the tokio
/// runtime, bounding the wait. The executor thread is synchronous and uses a
/// `sync_channel` to hand the async coordinator call its result; an unbounded
/// `recv()` there parks this thread forever if the spawned future stalls
/// (unreachable shard, lock, fsync) — surfacing to callers as an idle, never
/// answered request. `recv_timeout` turns that into a 504 instead.
fn recv_sharded<T>(rx: std::sync::mpsc::Receiver<DbResult<T>>, op: &str) -> DbResult<T> {
    recv_sharded_with(rx, op, std::time::Duration::from_secs(10))
}

fn recv_sharded_with<T>(
    rx: std::sync::mpsc::Receiver<DbResult<T>>,
    op: &str,
    wait: std::time::Duration,
) -> DbResult<T> {
    use std::sync::mpsc::RecvTimeoutError;
    match rx.recv_timeout(wait) {
        Ok(inner) => inner,
        Err(RecvTimeoutError::Timeout) => {
            tracing::warn!(
                layer = "solidb",
                op = op,
                timeout_secs = wait.as_secs(),
                "sharded operation timed out"
            );
            Err(DbError::Timeout(format!(
                "sharded {op} exceeded {}s",
                wait.as_secs()
            )))
        }
        Err(RecvTimeoutError::Disconnected) => {
            Err(DbError::InternalError(format!("sharded {op} task failed")))
        }
    }
}

impl<'a> QueryExecutor<'a> {
    /// For each row, the indices (in scan order) of `docs` matching the join
    /// condition with `var_name` bound to the doc.
    ///
    /// When the condition contains an equi-join term (`var.field == expr`),
    /// this builds a hash table over the docs' field values and probes it
    /// once per row — O(L+R) instead of the O(L×R) nested loop. Matching
    /// semantics are unchanged: `codec::encode_key` equality coincides with
    /// `values_equal` (numbers compare as f64, everything else structurally,
    /// serde_json maps serialize in canonical key order), and when the equi
    /// term was pulled out of an AND the full condition is re-checked per
    /// candidate. Non-equi conditions fall back to the nested loop.
    fn join_match_indices(
        &self,
        rows: &[Context],
        docs: &[Value],
        condition: &Expression,
        var_name: &str,
    ) -> Vec<Vec<usize>> {
        use super::super::get_field_value;
        use crate::storage::codec::encode_key;

        if let Some((field_path, key_expr, is_whole_condition)) =
            self.extract_equi_join_term(condition, var_name)
        {
            let mut table: std::collections::HashMap<Vec<u8>, Vec<usize>> =
                std::collections::HashMap::new();
            for (idx, doc) in docs.iter().enumerate() {
                let key = encode_key(&get_field_value(doc, &field_path));
                table.entry(key).or_default().push(idx);
            }

            return rows
                .iter()
                .map(|ctx| {
                    // An evaluation error reads as "no match", exactly like
                    // the nested loop's `if let Ok(..)`.
                    let Ok(key_val) = self.evaluate_expr_with_context(key_expr, ctx) else {
                        return Vec::new();
                    };
                    let Some(candidates) = table.get(&encode_key(&key_val)) else {
                        return Vec::new();
                    };
                    if is_whole_condition {
                        return candidates.clone();
                    }
                    candidates
                        .iter()
                        .copied()
                        .filter(|&idx| {
                            self.join_condition_matches(ctx, &docs[idx], condition, var_name)
                        })
                        .collect()
                })
                .collect();
        }

        // Fallback: nested loop for non-equi conditions
        rows.iter()
            .map(|ctx| {
                (0..docs.len())
                    .filter(|&idx| {
                        self.join_condition_matches(ctx, &docs[idx], condition, var_name)
                    })
                    .collect()
            })
            .collect()
    }

    fn join_condition_matches(
        &self,
        ctx: &Context,
        doc: &Value,
        condition: &Expression,
        var_name: &str,
    ) -> bool {
        let mut temp_ctx = ctx.clone();
        temp_ctx.insert(var_name.to_string(), doc.clone());
        self.evaluate_expr_with_context(condition, &temp_ctx)
            .map(|v| v.as_bool().unwrap_or(false))
            .unwrap_or(false)
    }

    /// Execute body clauses and return row contexts with mutation stats
    pub(super) fn execute_body_clauses(
        &self,
        clauses: &[BodyClause],
        initial_ctx: &Context,
        scan_limit: Option<usize>,
        indexed_filter_limit: Option<usize>,
    ) -> DbResult<(Vec<Context>, MutationStats)> {
        let mut rows: Vec<Context> = vec![initial_ctx.clone()];
        let mut stats = MutationStats::new();

        // Optimization: Check if we can use index for FOR + FILTER pattern
        // Pattern: FOR var IN collection, followed by FILTER on var.field == value
        let mut i = 0;
        while i < clauses.len() {
            match &clauses[i] {
                BodyClause::For(for_clause) => {
                    // Check if next clause is a FILTER that can use an index.
                    // Note: the indexable check is performed per-row inside the
                    // loop below — `extract_indexable_condition` now evaluates the
                    // non-field side against the row context, so a correlated
                    // FILTER (e.g. `rel._key == doc.organisation_id`) can still
                    // hit the index even though it isn't a literal.
                    let next_is_filter =
                        i + 1 < clauses.len() && matches!(&clauses[i + 1], BodyClause::Filter(_));
                    let is_collection = if let Some(src) = &for_clause.source_variable {
                        src == &for_clause.collection
                    } else {
                        true
                    };

                    if next_is_filter && is_collection {
                        if let BodyClause::Filter(filter_clause) = &clauses[i + 1] {
                            // Index path only if *every* row's filter is
                            // indexable — mixing index and scan per-row would
                            // silently drop rows whose condition couldn't be
                            // extracted. If any row fails, abandon and fall
                            // through to the normal FOR + FILTER scan path.
                            let mut new_rows = Vec::new();
                            let mut all_rows_indexable = true;

                            if let Ok(collection) = self.get_collection(&for_clause.collection) {
                                for ctx in &rows {
                                    // Cap the lookup at LIMIT only when the
                                    // whole FILTER is the index condition — a
                                    // residual conjunct could reject fetched
                                    // rows and under-fill the LIMIT.
                                    let lookup_limit = indexed_filter_limit.filter(|_| {
                                        self.filter_fully_covered_by_index(
                                            &filter_clause.expression,
                                            &for_clause.variable,
                                            ctx,
                                        )
                                    });
                                    let Some((docs, _name, _ty)) = self
                                        .lookup_index_for_filter_limited(
                                            &collection,
                                            &filter_clause.expression,
                                            &for_clause.variable,
                                            ctx,
                                            lookup_limit,
                                        )
                                    else {
                                        all_rows_indexable = false;
                                        break;
                                    };
                                    let docs: Vec<_> = if let Some(n) = scan_limit {
                                        docs.into_iter().take(n).collect()
                                    } else {
                                        docs
                                    };
                                    for doc in docs {
                                        let mut new_ctx = ctx.clone();
                                        new_ctx
                                            .insert(for_clause.variable.clone(), doc.into_value());
                                        new_rows.push(new_ctx);
                                    }
                                }
                            } else {
                                all_rows_indexable = false;
                            }

                            if all_rows_indexable {
                                // `extract_indexable_condition` only pulls ONE conjunct
                                // out of `FILTER a AND b ...` for the index lookup; the
                                // remaining conjuncts (e.g. `doc._key != @key`) are not
                                // applied by the index path. Re-evaluate the full FILTER
                                // expression against the index-loaded rows so multi-term
                                // filters return correct results.
                                new_rows.retain(|ctx| {
                                    self.evaluate_filter_with_context(
                                        &filter_clause.expression,
                                        ctx,
                                    )
                                    .unwrap_or(false)
                                });
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
                            let result = recv_sharded(rx, "insert")?;
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
                                let updated_doc = recv_sharded(rx, "update")?;
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
                                recv_sharded(rx, "remove")?;
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
                        let deleted_count = collection.delete_batch(keys.clone())?;
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

                    // Get edge collection (shared by every start vertex)
                    let edge_collection = self.get_collection(&gt.edge_collection)?;

                    // Without an index on _from/_to, the per-vertex fallback
                    // used to rescan the whole edge collection for EVERY
                    // visited vertex (B^depth full scans). Probe for the
                    // indexes once and, when missing, build an in-memory
                    // adjacency map with a single scan instead.
                    let probe = Value::String(String::new());
                    let has_from_index = edge_collection.index_lookup_eq("_from", &probe).is_some();
                    let has_to_index = edge_collection.index_lookup_eq("_to", &probe).is_some();
                    let needs_adjacency = match gt.direction {
                        EdgeDirection::Outbound => !has_from_index,
                        EdgeDirection::Inbound => !has_to_index,
                        EdgeDirection::Any => !(has_from_index && has_to_index),
                    };
                    let adjacency: Option<
                        std::collections::HashMap<String, Vec<crate::storage::Document>>,
                    > = if needs_adjacency {
                        let mut map: std::collections::HashMap<
                            String,
                            Vec<crate::storage::Document>,
                        > = std::collections::HashMap::new();
                        let want_from =
                            matches!(gt.direction, EdgeDirection::Outbound | EdgeDirection::Any);
                        let want_to =
                            matches!(gt.direction, EdgeDirection::Inbound | EdgeDirection::Any);
                        for doc in edge_collection.scan(None) {
                            let from = match doc.get("_from") {
                                Some(Value::String(s)) => Some(s.clone()),
                                _ => None,
                            };
                            let to = match doc.get("_to") {
                                Some(Value::String(s)) => Some(s.clone()),
                                _ => None,
                            };
                            if want_from {
                                if let Some(ref f) = from {
                                    map.entry(f.clone()).or_default().push(doc.clone());
                                }
                            }
                            if want_to {
                                if let Some(ref t) = to {
                                    // Self-loop already inserted under _from
                                    if !(want_from && from.as_deref() == Some(t.as_str())) {
                                        map.entry(t.clone()).or_default().push(doc.clone());
                                    }
                                }
                            }
                        }
                        Some(map)
                    } else {
                        None
                    };

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

                            // Find connected vertices using indexed lookup (O(1) per edge) instead of scan (O(E))
                            let current_id_str = current_id.clone();
                            let current_value = Value::String(current_id_str.clone());

                            // Use index lookup on _from/_to if available; otherwise the
                            // prebuilt adjacency map (single scan, done above).
                            let adjacency_edges = |key: &str| -> Vec<crate::storage::Document> {
                                adjacency
                                    .as_ref()
                                    .and_then(|m| m.get(key).cloned())
                                    .unwrap_or_default()
                            };
                            let edges: Vec<_> = match gt.direction {
                                EdgeDirection::Outbound => {
                                    // Look up edges where _from = current_id
                                    edge_collection
                                        .index_lookup_eq("_from", &current_value)
                                        .unwrap_or_else(|| adjacency_edges(&current_id_str))
                                }
                                EdgeDirection::Inbound => {
                                    // Look up edges where _to = current_id
                                    edge_collection
                                        .index_lookup_eq("_to", &current_value)
                                        .unwrap_or_else(|| adjacency_edges(&current_id_str))
                                }
                                EdgeDirection::Any => {
                                    // Union of both directions; with both indexes present use
                                    // them, otherwise the adjacency map (keyed on both ends).
                                    match (
                                        edge_collection.index_lookup_eq("_from", &current_value),
                                        edge_collection.index_lookup_eq("_to", &current_value),
                                    ) {
                                        (Some(from_edges), Some(to_edges)) => {
                                            let mut seen: std::collections::HashSet<String> =
                                                std::collections::HashSet::new();
                                            from_edges
                                                .into_iter()
                                                .chain(to_edges)
                                                .filter(|e| seen.insert(e.key.clone()))
                                                .collect()
                                        }
                                        _ => adjacency_edges(&current_id_str),
                                    }
                                }
                            };

                            for edge_doc in edges {
                                let edge_val = edge_doc.to_value();
                                let from = edge_val.get("_from").and_then(|v| v.as_str());
                                let to = edge_val.get("_to").and_then(|v| v.as_str());

                                let next_id = match gt.direction {
                                    EdgeDirection::Outbound => {
                                        // Already filtered by _from, so just get _to
                                        to.map(|s| s.to_string())
                                    }
                                    EdgeDirection::Inbound => {
                                        // Already filtered by _to, so just get _from
                                        from.map(|s| s.to_string())
                                    }
                                    EdgeDirection::Any => {
                                        // Need to determine direction
                                        if from == Some(&current_id_str) {
                                            to.map(|s| s.to_string())
                                        } else if to == Some(&current_id_str) {
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

                        // Scan edges ONCE per path search: the edge set doesn't
                        // change mid-BFS, and rescanning it for every dequeued
                        // vertex made shortest-path O(V × E) disk reads.
                        let all_edges = edge_collection.scan(None);

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

                            for edge_doc in &all_edges {
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
                            // Standard LEFT/INNER JOIN: iterate left side, find matches on right.
                            // Scan the joined collection ONCE: it doesn't depend on the
                            // left row, and rescanning it per row turned every join into
                            // O(left × right) disk reads. Matching goes through
                            // join_match_indices (hash join for equi-conditions).
                            let all_docs: Vec<Value> = collection
                                .scan(None)
                                .into_iter()
                                .map(|doc| doc.to_value())
                                .collect();

                            let match_indices = self.join_match_indices(
                                &rows,
                                &all_docs,
                                &join_clause.condition,
                                &join_clause.variable,
                            );

                            let mut new_rows = Vec::new();

                            for (ctx, indices) in rows.iter().zip(match_indices) {
                                let matches: Vec<Value> =
                                    indices.iter().map(|&i| all_docs[i].clone()).collect();

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

                            // Transpose row→docs matches into doc→rows so the
                            // hash-join path serves RIGHT JOIN too (row order
                            // within each doc is preserved by ascending
                            // iteration).
                            let match_indices = self.join_match_indices(
                                &rows,
                                &all_right_docs,
                                &join_clause.condition,
                                &join_clause.variable,
                            );
                            let mut rows_per_doc: Vec<Vec<usize>> =
                                vec![Vec::new(); all_right_docs.len()];
                            for (row_idx, indices) in match_indices.iter().enumerate() {
                                for &doc_idx in indices {
                                    rows_per_doc[doc_idx].push(row_idx);
                                }
                            }

                            for (doc_idx, right_doc) in all_right_docs.into_iter().enumerate() {
                                // Convert matching left contexts to Values for grouping
                                let left_matches: Vec<Value> = rows_per_doc[doc_idx]
                                    .iter()
                                    .map(|&row_idx| {
                                        serde_json::to_value(&rows[row_idx])
                                            .unwrap_or(Value::Object(serde_json::Map::new()))
                                    })
                                    .collect();

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

                            // Find the left variable name from the first FOR clause
                            // This is needed for orphan right rows to include the left variable as null
                            let left_variable_name = clauses
                                .iter()
                                .find_map(|c| {
                                    if let BodyClause::For(for_clause) = c {
                                        Some(for_clause.variable.clone())
                                    } else {
                                        None
                                    }
                                })
                                .unwrap_or_else(|| "left".to_string());

                            let all_right_docs: Vec<Value> = collection
                                .scan(None)
                                .into_iter()
                                .map(|doc| doc.to_value())
                                .collect();

                            // Phase 1: LEFT JOIN part - iterate left, find right matches
                            let match_indices = self.join_match_indices(
                                &rows,
                                &all_right_docs,
                                &join_clause.condition,
                                &join_clause.variable,
                            );
                            for (ctx, indices) in rows.iter().zip(match_indices) {
                                let matches: Vec<Value> = indices
                                    .iter()
                                    .map(|&idx| {
                                        matched_right_indices.insert(idx);
                                        all_right_docs[idx].clone()
                                    })
                                    .collect();

                                // Always include left row (LEFT JOIN semantics)
                                let mut new_ctx = ctx.clone();
                                new_ctx.insert(join_clause.variable.clone(), Value::Array(matches));
                                new_rows.push(new_ctx);
                            }

                            // Phase 2: Add unmatched right rows (RIGHT JOIN part)
                            for (idx, right_doc) in all_right_docs.iter().enumerate() {
                                if !matched_right_indices.contains(&idx) {
                                    let mut new_ctx = std::collections::HashMap::new();
                                    // Include left-side variable with null (no match)
                                    new_ctx.insert(left_variable_name.clone(), Value::Null);
                                    // Wrap right doc in array for consistency with Phase 1
                                    new_ctx.insert(
                                        join_clause.variable.clone(),
                                        Value::Array(vec![right_doc.clone()]),
                                    );
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
}

#[cfg(test)]
mod recv_sharded_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn timeout_when_sender_never_sends() {
        // Hold the sender open but never send → recv_timeout elapses →
        // a bounded 504-mapped Timeout, not an indefinite park.
        let (_tx, rx) = std::sync::mpsc::sync_channel::<DbResult<u32>>(1);
        let err = recv_sharded_with(rx, "insert", Duration::from_millis(50)).unwrap_err();
        assert!(matches!(err, DbError::Timeout(_)), "got {err:?}");
    }

    #[test]
    fn disconnected_when_sender_dropped() {
        // Sender dropped without sending (e.g. spawned task panicked) →
        // InternalError, distinct from the timeout case.
        let (tx, rx) = std::sync::mpsc::sync_channel::<DbResult<u32>>(1);
        drop(tx);
        let err = recv_sharded_with(rx, "update", Duration::from_secs(10)).unwrap_err();
        assert!(matches!(err, DbError::InternalError(_)), "got {err:?}");
    }

    #[test]
    fn passes_through_inner_result() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<DbResult<u32>>(1);
        tx.send(Ok(7)).unwrap();
        assert_eq!(
            recv_sharded_with(rx, "remove", Duration::from_secs(10)).unwrap(),
            7
        );
    }
}
