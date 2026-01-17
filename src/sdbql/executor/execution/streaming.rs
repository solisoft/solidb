//! Streaming and bulk insert operations for SDBQL executor.
//!
//! This module contains:
//! - try_streaming_bulk_insert: Optimized bulk insert for large ranges
//! - log_mutation: Log single mutation for replication
//! - log_mutations_async: Async batch mutation logging

use serde_json::Value;

use super::super::types::Context;
use super::super::QueryExecutor;
use crate::error::DbResult;
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
}
