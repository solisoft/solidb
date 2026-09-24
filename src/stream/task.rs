use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{BodyClause, Query, WindowType};
use crate::sdbql::executor::{QueryExecutor, QueryPrincipal};
use crate::storage::collection::{ChangeEvent, ChangeType};
use crate::storage::StorageEngine;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::broadcast;

/// Longest accepted window. Audit A1: an unbounded size either panicked in
/// chrono (`"9999999999h"`) or, with M4, kept a month of writes in RAM.
pub const MAX_WINDOW: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

const DEFAULT_MAX_BUFFER_EVENTS: usize = 100_000;
const DEFAULT_MAX_BUFFER_BYTES: usize = 64 * 1024 * 1024;

/// Events dropped from stream window buffers because a cap was reached.
static STREAM_EVENTS_DROPPED: AtomicU64 = AtomicU64::new(0);

/// Total events dropped from stream window buffers since start-up (Audit M4).
pub fn stream_events_dropped_total() -> u64 {
    STREAM_EVENTS_DROPPED.load(Ordering::Relaxed)
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

/// Per-stream buffer caps: `SOLIDB_STREAM_MAX_BUFFER_EVENTS` and
/// `SOLIDB_STREAM_MAX_BUFFER_BYTES`.
fn buffer_limits() -> (usize, usize) {
    static LIMITS: OnceLock<(usize, usize)> = OnceLock::new();
    *LIMITS.get_or_init(|| {
        (
            env_usize("SOLIDB_STREAM_MAX_BUFFER_EVENTS", DEFAULT_MAX_BUFFER_EVENTS),
            env_usize("SOLIDB_STREAM_MAX_BUFFER_BYTES", DEFAULT_MAX_BUFFER_BYTES),
        )
    })
}

/// Cheap upper-ish estimate of a document's in-memory footprint, without
/// serialising it.
fn approx_size(v: &Value) -> usize {
    match v {
        Value::Null | Value::Bool(_) | Value::Number(_) => 16,
        Value::String(s) => 24 + s.len(),
        Value::Array(a) => 24 + a.iter().map(approx_size).sum::<usize>(),
        Value::Object(o) => {
            32 + o
                .iter()
                .map(|(k, v)| 24 + k.len() + approx_size(v))
                .sum::<usize>()
        }
    }
}

pub struct StreamTask {
    pub name: String,
    pub collection: String,
    query: Query,
    window_type: WindowType,
    window_duration: Duration,
    storage: Arc<StorageEngine>,
    rx: broadcast::Receiver<ChangeEvent>,
    db_name: String,
    /// Who the stream runs as: the creator, or an anonymous non-admin for a
    /// definition that recorded no one (Audit H6).
    principal: QueryPrincipal,

    // State
    // Buffer of (timestamp, document, approx bytes) for sliding window support
    buffer: VecDeque<(DateTime<Utc>, Value, usize)>,
    buffer_bytes: usize,
    max_buffer_events: usize,
    max_buffer_bytes: usize,
    /// Events dropped since the last window was processed; logged once per window.
    dropped_in_window: u64,
    next_window_end: DateTime<Utc>,
}

impl StreamTask {
    pub fn new(
        name: String,
        query: Query,
        db_name: String,
        storage: Arc<StorageEngine>,
        rx: broadcast::Receiver<ChangeEvent>,
        principal: QueryPrincipal,
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

        let next_window_end = Utc::now()
            .checked_add_signed(duration)
            .ok_or_else(|| DbError::ExecutionError("Window end out of range".to_string()))?;
        let (max_buffer_events, max_buffer_bytes) = buffer_limits();

        Ok(Self {
            name,
            collection,
            query,
            window_type,
            window_duration: duration,
            storage,
            rx,
            db_name,
            principal,
            buffer: VecDeque::new(),
            buffer_bytes: 0,
            max_buffer_events,
            max_buffer_bytes,
            dropped_in_window: 0,
            next_window_end,
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
                        // Advance window. Audit A1: computed in one step rather
                        // than a `while` loop with no await point, which a zero
                        // window turned into a worker that abort() cannot stop.
                        self.next_window_end = next_window_end_after(
                            self.next_window_end,
                            Utc::now(),
                            self.window_duration,
                        );

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
                    self.push_buffered(now, data);
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
                    self.clear_buffer();
                }
            }
        }

        // Opportunistic prune for sliding windows
        if matches!(self.window_type, WindowType::Sliding) {
            self.prune_older_than(now - self.window_duration);
        }
    }

    /// Append to the window buffer, dropping the oldest entries once the
    /// count or byte cap is reached (Audit M4).
    fn push_buffered(&mut self, ts: DateTime<Utc>, doc: Value) {
        let size = approx_size(&doc);
        self.buffer.push_back((ts, doc, size));
        self.buffer_bytes += size;
        while self.buffer.len() > self.max_buffer_events
            || (self.buffer_bytes > self.max_buffer_bytes && self.buffer.len() > 1)
        {
            let Some((_, _, sz)) = self.buffer.pop_front() else {
                break;
            };
            self.buffer_bytes = self.buffer_bytes.saturating_sub(sz);
            self.dropped_in_window += 1;
            STREAM_EVENTS_DROPPED.fetch_add(1, Ordering::Relaxed);
            if self.dropped_in_window == 1 {
                tracing::warn!(
                    "Stream {}: window buffer full ({} events / {} bytes cap), dropping oldest events",
                    self.name,
                    self.max_buffer_events,
                    self.max_buffer_bytes
                );
            }
        }
    }

    fn prune_older_than(&mut self, cutoff: DateTime<Utc>) {
        while let Some((ts, _, sz)) = self.buffer.front() {
            if *ts > cutoff {
                break;
            }
            self.buffer_bytes = self.buffer_bytes.saturating_sub(*sz);
            self.buffer.pop_front();
        }
    }

    fn clear_buffer(&mut self) {
        self.buffer.clear();
        self.buffer_bytes = 0;
    }

    /// The source collection's row policy, parsed, when it applies to this
    /// stream's principal. `Err` means a policy exists but does not parse:
    /// the caller must then admit nothing.
    fn source_row_policy(&self) -> Result<Option<crate::sdbql::ast::Expression>, ()> {
        if self.principal.can_admin {
            return Ok(None);
        }
        let Some(policy) = self
            .storage
            .get_database(&self.db_name)
            .ok()
            .and_then(|db| db.get_collection(&self.collection).ok())
            .and_then(|c| c.get_row_policy())
        else {
            return Ok(None);
        };
        let mut parser = crate::sdbql::parser::Parser::new(&policy).map_err(|_| ())?;
        parser.parse_expression().map(Some).map_err(|_| ())
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
                self.clear_buffer();
            }
            return Ok(());
        }
        if self.dropped_in_window > 0 {
            tracing::warn!(
                "Stream {}: {} events were dropped from this window by the buffer cap",
                self.name,
                self.dropped_in_window
            );
            self.dropped_in_window = 0;
        }

        let for_clause = &self.query.for_clauses[0];
        let var_name = &for_clause.variable;

        // Audit H6: evaluate as the stream's creator, not as the server, and
        // apply the source's row policy to buffered documents the way a scan
        // would — otherwise a stream copies rows its creator cannot read into
        // an unpoliced `_streams:` collection.
        let executor = QueryExecutor::with_database(&self.storage, self.db_name.clone())
            .with_principal(self.principal.clone());
        let row_policy = self.source_row_policy();

        // Build contexts from (timestamped) buffer. For sliding we already pruned opportunistically.
        let mut contexts: Vec<std::collections::HashMap<String, Value>> = Vec::new();

        for (_ts, doc, _) in &self.buffer {
            match &row_policy {
                Ok(None) => {}
                Ok(Some(expr)) => {
                    let mut row = std::collections::HashMap::new();
                    row.insert(self.collection.clone(), doc.clone());
                    row.insert("doc".to_string(), doc.clone());
                    row.insert(
                        "CURRENT_USER".to_string(),
                        Value::String(self.principal.user.clone()),
                    );
                    let visible = executor
                        .evaluate_expr_with_context(expr, &row)
                        .map(|v| crate::sdbql::executor::to_bool(&v))
                        .unwrap_or(false);
                    if !visible {
                        continue;
                    }
                }
                Err(()) => continue,
            }

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
                        let _ = coll.insert_or_replace(to_store);
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
                let _ = coll.insert_or_replace(summary);
            }
        }

        // Release the borrow of `self.storage` before mutating the buffer.
        drop(executor);

        // Tumbling: clear everything. Sliding: already prunes on ingest + here keep recent.
        // Old results are automatically expired via TTL index on "emitted_at" (created on first use).
        if matches!(self.window_type, WindowType::Tumbling) {
            self.clear_buffer();
        } else {
            // Final prune for safety on sliding
            self.prune_older_than(Utc::now() - self.window_duration);
        }

        Ok(())
    }
}

/// The first window boundary strictly after `now`, stepping from `end` by
/// whole windows. One division instead of a loop, and checked throughout.
fn next_window_end_after(
    end: DateTime<Utc>,
    now: DateTime<Utc>,
    window: Duration,
) -> DateTime<Utc> {
    if end > now {
        return end;
    }
    let fallback = || now.checked_add_signed(window).unwrap_or(now);
    let window_ms = window.num_milliseconds();
    if window_ms <= 0 {
        return fallback();
    }
    let behind_ms = (now - end).num_milliseconds().max(0);
    let steps = behind_ms / window_ms + 1;
    steps
        .checked_mul(window_ms)
        .and_then(Duration::try_milliseconds)
        .and_then(|d| end.checked_add_signed(d))
        .unwrap_or_else(fallback)
}

/// Parse a window size such as `"30s"`, `"5m"` or `"1h"`.
///
/// Audit A1: rejects zero, negative and over-[`MAX_WINDOW`] sizes, and builds
/// the value with chrono's checked constructors so no input can panic.
pub(crate) fn parse_duration(s: &str) -> DbResult<Duration> {
    let invalid = || DbError::ParseError(format!("Invalid duration '{}'", s));
    let s = s.trim();
    let (num, ctor): (&str, fn(i64) -> Option<Duration>) = if let Some(n) = s.strip_suffix('m') {
        (n, Duration::try_minutes)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, Duration::try_seconds)
    } else if let Some(n) = s.strip_suffix('h') {
        (n, Duration::try_hours)
    } else {
        return Err(DbError::ParseError(
            "Unknown duration unit (use s, m, h)".to_string(),
        ));
    };
    let value = num.trim().parse::<i64>().map_err(|_| invalid())?;
    if value <= 0 {
        return Err(DbError::ParseError(format!(
            "Window size must be positive, got '{}'",
            s
        )));
    }
    let duration = ctor(value).ok_or_else(invalid)?;
    let max = Duration::from_std(MAX_WINDOW).map_err(|_| invalid())?;
    if duration > max {
        return Err(DbError::ParseError(format!(
            "Window size '{}' exceeds the maximum of {}h",
            s,
            MAX_WINDOW.as_secs() / 3600
        )));
    }
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_accepts_units() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::seconds(30));
        assert_eq!(parse_duration("5m").unwrap(), Duration::minutes(5));
        assert_eq!(parse_duration("1h").unwrap(), Duration::hours(1));
        assert_eq!(parse_duration("24h").unwrap(), Duration::hours(24));
    }

    #[test]
    fn parse_duration_rejects_zero_negative_and_huge() {
        for bad in [
            "0s",
            "0m",
            "-5s",
            "-1h",
            "25h",
            "1441m",
            "86401s",
            "9999999999h",
            "9223372036854775807s",
            "abc",
            "10",
            "",
        ] {
            assert!(parse_duration(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn next_window_end_is_strictly_after_now() {
        let start = Utc::now();
        let w = Duration::seconds(10);
        // Not yet reached: unchanged.
        let future = start + Duration::seconds(5);
        assert_eq!(next_window_end_after(future, start, w), future);
        // Exactly at the boundary: one step.
        assert_eq!(next_window_end_after(start, start, w), start + w);
        // Far behind: lands on the grid, just past now.
        let now = start + Duration::seconds(95);
        let next = next_window_end_after(start, now, w);
        assert_eq!(next, start + Duration::seconds(100));
        assert!(next > now);
        // Degenerate window cannot loop.
        let z = next_window_end_after(start, now, Duration::zero());
        assert_eq!(z, now);
    }

    #[test]
    fn approx_size_grows_with_content() {
        let small = serde_json::json!({"a": 1});
        let big = serde_json::json!({"a": "x".repeat(1000)});
        assert!(approx_size(&big) > approx_size(&small) + 900);
    }
}
