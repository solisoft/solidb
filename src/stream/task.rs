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
    // Buffer of (timestamp, document) for proper sliding window support
    buffer: Vec<(DateTime<Utc>, Value)>,
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
        let now = Utc::now();

        match event.type_ {
            ChangeType::Insert | ChangeType::Update => {
                if let Some(data) = event.data {
                    self.buffer.push((now, data));
                }
            }
            ChangeType::Delete => {
                // For now we ignore deletes in window buffers (common for many streaming use cases).
                // Advanced: could support "subtract" for some aggs.
            }
            ChangeType::Truncate => {
                if !self.buffer.is_empty() {
                    tracing::warn!(
                        "Stream {}: source collection truncated, discarding {} buffered events",
                        self.name,
                        self.buffer.len()
                    );
                    self.buffer.clear();
                }
            }
        }

        // Opportunistic prune for sliding windows
        if matches!(self.window_type, WindowType::Sliding) {
            let cutoff = now - self.window_duration;
            self.buffer.retain(|(ts, _)| *ts > cutoff);
        }
    }

    async fn process_window(&mut self) -> DbResult<()> {
        let event_count = self.buffer.len();
        tracing::info!(
            "Stream {}: Processing window with {} events",
            self.name,
            event_count
        );

        if event_count == 0 {
            // Advance window anyway
            if matches!(self.window_type, WindowType::Tumbling) {
                self.buffer.clear();
            }
            return Ok(());
        }

        let for_clause = &self.query.for_clauses[0];
        let var_name = &for_clause.variable;

        let executor = QueryExecutor::with_database(&self.storage, self.db_name.clone());

        // Build contexts from (timestamped) buffer. For sliding we already pruned opportunistically.
        let mut contexts: Vec<std::collections::HashMap<String, Value>> = Vec::new();

        for (_ts, doc) in &self.buffer {
            let mut keep = true;
            let mut ctx = std::collections::HashMap::new();
            ctx.insert(var_name.clone(), doc.clone());

            // Apply FILTERs. Treat errors as non-matching (exclude) to avoid polluting
            // aggregates with bad data (e.g. missing fields).
            for filter_clause in &self.query.filter_clauses {
                match executor.evaluate_filter_with_context(&filter_clause.expression, &ctx) {
                    Ok(true) => {}
                    _ => {
                        keep = false;
                        break;
                    }
                }
            }

            if keep {
                contexts.push(ctx);
            }
        }

        // Very basic COLLECT support (WITH COUNT, simple groups)
        let mut results = contexts;

        for clause in &self.query.body_clauses {
            if let BodyClause::Collect(collect) = clause {
                if let Some(count_var) = &collect.count_var {
                    if collect.group_vars.is_empty() {
                        // Single aggregate row
                        let count = results.len();
                        let mut agg = std::collections::HashMap::new();
                        agg.insert(
                            count_var.clone(),
                            Value::Number(serde_json::Number::from(count)),
                        );
                        // Also carry a sample window size
                        agg.insert(
                            "window_size".to_string(),
                            Value::Number(serde_json::Number::from(count)),
                        );
                        results = vec![agg];
                    }
                }
            }
        }

        // Evaluate RETURN and persist useful results into a stream-backed collection
        // Target collection: "<db>:_streams:<stream_name>" (auto created)
        let stream_output_name = format!("_streams:{}", self.name);

        // Ensure the output collection exists (lightweight)
        let db = match self.storage.get_database(&self.db_name) {
            Ok(d) => d,
            Err(_) => return Ok(()),
        };

        let output_coll = if db.get_collection(&stream_output_name).is_err() {
            let _ = db.create_collection(stream_output_name.clone(), None);
            db.get_collection(&stream_output_name).ok()
        } else {
            db.get_collection(&stream_output_name).ok()
        };

        // Create TTL index for automatic retention (7 days) to avoid O(N) manual scans every window
        const STREAM_RETENTION_SECS: u64 = 7 * 24 * 3600;
        if let Some(coll) = &output_coll {
            if coll.get_ttl_index("retention").is_none() {
                let _ = coll.create_ttl_index(
                    "retention".to_string(),
                    "emitted_at".to_string(),
                    STREAM_RETENTION_SECS,
                );
            }
        }

        if let Some(return_clause) = &self.query.return_clause {
            for ctx in results {
                if let Ok(result_val) =
                    executor.evaluate_expr_with_context(&return_clause.expression, &ctx)
                {
                    tracing::info!("Stream {}: Emit result -> {:?}", self.name, result_val);

                    // Persist to the stream output collection if object-like
                    if let Some(coll) = &output_coll {
                        // Best effort insert of the emitted value (wrap non-objects)
                        let mut to_store = if result_val.is_object() {
                            result_val.clone()
                        } else {
                            serde_json::json!({ "value": result_val })
                        };
                        if let Some(obj) = to_store.as_object_mut() {
                            obj.entry("emitted_at".to_string())
                                .or_insert(serde_json::json!(Utc::now().to_rfc3339()));
                        }
                        let _ = coll.insert(to_store);
                    }
                }
            }
        } else {
            // No explicit RETURN: store a summary document for the window
            if let Some(coll) = &output_coll {
                let now = Utc::now().to_rfc3339();
                let summary = serde_json::json!({
                    "stream": self.name,
                    "window_end": now,
                    "emitted_at": now,
                    "event_count": event_count,
                    "source": self.collection
                });
                let _ = coll.insert(summary);
            }
        }

        // Tumbling: clear everything. Sliding: already prunes on ingest + here keep recent.
        // Old results are automatically expired via TTL index on "emitted_at" (created on first use).
        if matches!(self.window_type, WindowType::Tumbling) {
            self.buffer.clear();
        } else {
            // Final prune for safety on sliding
            let cutoff = Utc::now() - self.window_duration;
            self.buffer.retain(|(ts, _)| *ts > cutoff);
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
