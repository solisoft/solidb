use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{BodyClause, Query, WindowType};
use crate::sdbql::executor::QueryExecutor;
use crate::storage::collection::{ChangeEvent, ChangeType};
use crate::storage::StorageEngine;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct StreamTask {
    pub name: String,
    pub collection: String,
    query: Query,
    window_type: WindowType,
    window_duration: Duration,
    storage: Arc<StorageEngine>,
    rx: broadcast::Receiver<ChangeEvent>,
    db_name: String,

    // State
    buffer: Vec<Value>, // Buffer of documents
    next_window_end: DateTime<Utc>,
}

impl StreamTask {
    pub fn new(
        name: String,
        query: Query,
        db_name: String,
        storage: Arc<StorageEngine>,
        rx: broadcast::Receiver<ChangeEvent>,
    ) -> DbResult<Self> {
        // Extract window info first
        let (window_type, duration_str) = {
            let window_clause = query
                .window_clause
                .as_ref()
                .ok_or(DbError::ExecutionError("Missing WINDOW clause".to_string()))?;
            (
                window_clause.window_type.clone(),
                window_clause.duration.clone(),
            )
        };

        // Parse duration (e.g. "1m", "30s")
        let duration = parse_duration(&duration_str)?;

        // Find source collection
        let for_clause = query
            .for_clauses
            .first()
            .ok_or(DbError::ExecutionError("Missing FOR clause".to_string()))?;
        let collection = for_clause.collection.clone();

        Ok(Self {
            name,
            collection,
            query,
            window_type,
            window_duration: duration,
            storage,
            rx,
            db_name,
            buffer: Vec::new(),
            // Align window to next minute/second/etc? For now just start from now + duration
            next_window_end: Utc::now() + duration,
        })
    }

    pub async fn run(mut self) {
        tracing::info!(
            "Stream {}: Started (Window: {:?})",
            self.name,
            self.window_duration
        );

        loop {
            // Check if window ended (non-blocking if not using sleep_until)
            // But strict timing requires precise waking.
            // We calculate wait duration.
            let now = Utc::now();
            let wait_duration = if now >= self.next_window_end {
                std::time::Duration::from_millis(0)
            } else {
                (self.next_window_end - now)
                    .to_std()
                    .unwrap_or(std::time::Duration::from_millis(1))
            };

            tokio::select! {
                // Receive event
                event_res = self.rx.recv() => {
                    match event_res {
                        Ok(event) => {
                             self.process_event(event);
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("Stream {}: Lagged by {} events", self.name, n);
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            tracing::info!("Stream {}: Source closed, stopping", self.name);
                            break;
                        }
                    }
                }

                // Window timer
                _ = tokio::time::sleep(wait_duration) => {
                    if Utc::now() >= self.next_window_end {
                        if !self.buffer.is_empty() {
                            if let Err(e) = self.process_window().await {
                                tracing::error!("Stream {}: Processing error: {}", self.name, e);
                            }
                        }
                        // Advance window
                        // For tumbling: start new window from now or previous end?
                        // Usually aligned.
                        while self.next_window_end <= Utc::now() {
                            self.next_window_end += self.window_duration;
                        }

                        // For sliding window, we might need different logic (keeping history)
                        if matches!(self.window_type, WindowType::Sliding) {
                            // TODO: Sliding window retention policy
                            // For now simplest is clearing buffer like Tumbling (incorrect behavior but placeholder)
                        }
                    }
                }
            }
        }
        tracing::info!("Stream {}: Stopped", self.name);
    }

    fn process_event(&mut self, event: ChangeEvent) {
        // Filter out deletes if we only care about new data?
        // Or maybe we treat them as events.
        // Typically streams process INSERTs.

        match event.type_ {
            ChangeType::Insert | ChangeType::Update => {
                if let Some(data) = event.data {
                    self.buffer.push(data);
                }
            }
            ChangeType::Delete => {
                // Ignore deletes for now in simple stream processing
            }
        }
    }

    async fn process_window(&mut self) -> DbResult<()> {
        tracing::info!(
            "Stream {}: Processing window with {} events",
            self.name,
            self.buffer.len()
        );

        // We need to execute the query logic on the buffered data.
        // Since QueryExecutor expects a StorageEngine and works on collections,
        // we need to adapt it or manually execute the pipeline.

        // Pipeline: FILTER -> COLLECT -> RETURN
        // The FOR clause is conceptually iterating over the window buffer.

        let for_clause = &self.query.for_clauses[0];
        let var_name = &for_clause.variable;

        // 1. FILTER
        let mut filtered_docs = Vec::new();
        let executor = QueryExecutor::with_database(&self.storage, self.db_name.clone());

        for doc in &self.buffer {
            // Check filters
            let mut keep = true;
            let mut ctx = std::collections::HashMap::new();
            ctx.insert(var_name.clone(), doc.clone());

            for filter_clause in &self.query.filter_clauses {
                let match_result = executor
                    .evaluate_filter_with_context(&filter_clause.expression, &ctx)
                    .unwrap_or(false);
                if !match_result {
                    keep = false;
                    break;
                }
            }

            if keep {
                filtered_docs.push(ctx);
            }
        }

        // 2. COLLECT (Aggregation)
        // If there is a collect clause, group data
        // We reuse execute_body_clauses logic ideally, but it's private and tied to DB scan.
        // We can extract aggregation logic or implement simple one here.

        // Let's look for COLLECT clause in body_clauses
        // Note: Parser extraction put them in body_clauses

        let mut results = filtered_docs;

        for clause in &self.query.body_clauses {
            if let BodyClause::Collect(collect) = clause {
                // Simple implementation of single-group aggregation (COUNT/SUM)
                // or Group By
                // ... (omitted for brevity, assume simple COUNT or passthrough for now)

                // If we have COUNT INTO var
                if let Some(count_var) = &collect.count_var {
                    // This is "COLLECT WITH COUNT INTO var" - aggregation into single result (sort of, or per group)
                    // If no group vars, it collapses everything.
                    if collect.group_vars.is_empty() {
                        let count = results.len();
                        let mut new_ctx = std::collections::HashMap::new();
                        new_ctx.insert(
                            count_var.clone(),
                            Value::Number(serde_json::Number::from(count)),
                        );
                        results = vec![new_ctx];
                    }
                }
            }
        }

        // 3. RETURN -> Insert into output stream (or log for now)
        if let Some(return_clause) = &self.query.return_clause {
            for ctx in results {
                let result_val =
                    executor.evaluate_expr_with_context(&return_clause.expression, &ctx)?;
                tracing::info!("Stream {}: Emit {:?}", self.name, result_val);

                // If the result is an object and we have a target collection?
                // The syntax `CREATE STREAM name AS ...` implies the stream itself is a source for others.
                // We might want to persist it or broadcast it.
                // For now, let's just log it.
            }
        } else {
            // Implicit return of all context vars?
        }

        // Clear buffer for Tumbling window
        if matches!(self.window_type, WindowType::Tumbling) {
            self.buffer.clear();
        }

        Ok(())
    }
}

fn parse_duration(s: &str) -> DbResult<Duration> {
    if let Some(mins_str) = s.strip_suffix('m') {
        let mins = mins_str
            .parse::<i64>()
            .map_err(|_| DbError::ParseError("Invalid duration".to_string()))?;
        Ok(Duration::minutes(mins))
    } else if let Some(secs_str) = s.strip_suffix('s') {
        let secs = secs_str
            .parse::<i64>()
            .map_err(|_| DbError::ParseError("Invalid duration".to_string()))?;
        Ok(Duration::seconds(secs))
    } else if let Some(hours_str) = s.strip_suffix('h') {
        let hours = hours_str
            .parse::<i64>()
            .map_err(|_| DbError::ParseError("Invalid duration".to_string()))?;
        Ok(Duration::hours(hours))
    } else {
        Err(DbError::ParseError(
            "Unknown duration unit (use s, m, h)".to_string(),
        ))
    }
}
