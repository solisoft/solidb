use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use crate::{
    error::DbError,
    sdbql::{parse, BodyClause, Query, QueryExecutor},
    server::response::ApiResponse,
    storage::StorageEngine,
};
use super::system::AppState;
use super::documents::get_transaction_id;

// ==================== Constants ====================

/// Default query execution timeout (30 seconds)
const QUERY_TIMEOUT_SECS: u64 = 30;

/// Default slow query threshold in milliseconds (100ms)
/// Queries taking longer than this will be logged to _slow_queries collection
const SLOW_QUERY_THRESHOLD_MS: f64 = 100.0;

// ==================== Structs ====================

#[derive(Debug, Deserialize)]
pub struct ExecuteQueryRequest {
    pub query: String,
    #[serde(default, alias = "bindVars")]
    pub bind_vars: std::collections::HashMap<String, Value>,
    #[serde(default = "default_batch_size", alias = "batchSize")]
    pub batch_size: usize,
}

fn default_batch_size() -> usize {
    1000
}

#[derive(Debug, Serialize)]
pub struct ExecuteQueryResponse {
    pub result: Vec<Value>,
    pub count: usize,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub cached: bool,
    #[serde(rename = "extra")]
    pub execution_time_ms: f64,
    #[serde(rename = "inserted")]
    pub documents_inserted: usize,
    #[serde(rename = "updated")]
    pub documents_updated: usize,
    #[serde(rename = "deleted")]
    pub documents_removed: usize,
}

// ==================== Helper Functions ====================

/// Check if a query is potentially long-running (contains mutations or range iterations)
#[inline]
fn is_long_running_query(query: &Query) -> bool {
    query.body_clauses.iter().any(|clause| match clause {
        BodyClause::Insert(_) | BodyClause::Update(_) | BodyClause::Remove(_) => true,
        // All FOR loops should use spawn_blocking because:
        // 1. Range expressions (source_expression.is_some()) can be large
        // 2. Collection scans might trigger scatter-gather with blocking HTTP calls
        BodyClause::For(_) => true,
        _ => false,
    })
}

/// Log slow query to _slow_queries collection (async, non-blocking)
fn log_slow_query(
    storage: Arc<StorageEngine>,
    db_name: String,
    query_text: String,
    execution_time_ms: f64,
    results_count: usize,
    documents_inserted: usize,
    documents_updated: usize,
    documents_removed: usize,
) {
    // Only log if query exceeds threshold
    if execution_time_ms < SLOW_QUERY_THRESHOLD_MS {
        return;
    }

    // Spawn background task to avoid blocking the response
    tokio::spawn(async move {
        let slow_query_coll = format!("{}:_slow_queries", db_name);

        // Get or create the _slow_queries collection
        let collection = match storage.get_collection(&slow_query_coll) {
            Ok(coll) => coll,
            Err(_) => {
                // Collection doesn't exist, try to create it
                if let Ok(db) = storage.get_database(&db_name) {
                    if db
                        .create_collection("_slow_queries".to_string(), None)
                        .is_err()
                    {
                        // Might fail if another request created it concurrently, try again
                        match storage.get_collection(&slow_query_coll) {
                            Ok(coll) => coll,
                            Err(e) => {
                                tracing::warn!("Failed to get _slow_queries collection: {}", e);
                                return;
                            }
                        }
                    } else {
                        match storage.get_collection(&slow_query_coll) {
                            Ok(coll) => coll,
                            Err(e) => {
                                tracing::warn!("Failed to get _slow_queries collection: {}", e);
                                return;
                            }
                        }
                    }
                } else {
                    return;
                }
            }
        };

        // Create log entry
        let log_entry = serde_json::json!({
            "query": query_text,
            "execution_time_ms": execution_time_ms,
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "results_count": results_count,
            "mutations": {
                "inserted": documents_inserted,
                "updated": documents_updated,
                "removed": documents_removed
            }
        });

        if let Err(e) = collection.insert(log_entry) {
            tracing::warn!("Failed to log slow query: {}", e);
        }
    });
}

// ==================== Handlers ====================

pub async fn execute_query(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ExecuteQueryRequest>,
) -> Result<ApiResponse<ExecuteQueryResponse>, DbError> {
    // Check for transaction context
    if let Some(tx_id) = get_transaction_id(&headers) {
        // Execute transactional SDBQL query
        use crate::sdbql::ast::BodyClause;

        let query = parse(&req.query)?;

        // Get transaction manager
        let tx_manager = state.storage.transaction_manager()?;
        let tx_arc = tx_manager.get(tx_id)?;
        let mut tx = tx_arc
            .write()
            .map_err(|_| DbError::InternalError("Transaction lock poisoned".into()))?;
        let wal = tx_manager.wal();

        // Check if query contains mutation operations
        let has_mutations = query.body_clauses.iter().any(|clause| {
            matches!(
                clause,
                BodyClause::Insert(_) | BodyClause::Update(_) | BodyClause::Remove(_)
            )
        });

        if !has_mutations {
            // No mutations - just execute normally (read operations)
            // No mutations - just execute normally (read operations)
            let executor = if req.bind_vars.is_empty() {
                QueryExecutor::with_database(&state.storage, db_name)
            } else {
                QueryExecutor::with_database_and_bind_vars(&state.storage, db_name, req.bind_vars)
            };

            let results = executor.execute(&query)?;
            return Ok(ApiResponse::new(
                ExecuteQueryResponse {
                    result: results.clone(),
                    count: results.len(),
                    has_more: false,
                    id: None,
                    cached: false,
                    execution_time_ms: 0.0,
                    documents_inserted: 0,
                    documents_updated: 0,
                    documents_removed: 0,
                },
                &headers,
            ));
        }

        // For mutation operations, execute transactionally
        let executor = if req.bind_vars.is_empty() {
            QueryExecutor::with_database(&state.storage, db_name.clone())
        } else {
            QueryExecutor::with_database_and_bind_vars(
                &state.storage,
                db_name.clone(),
                req.bind_vars.clone(),
            )
        };

        // Execute body clauses manually to intercept mutations
        let mut initial_bindings = std::collections::HashMap::new();

        // Merge bind variables
        for (key, value) in &req.bind_vars {
            initial_bindings.insert(format!("@{}", key), value.clone());
        }

        // Process LET clauses
        for let_clause in &query.let_clauses {
            let value =
                executor.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
            initial_bindings.insert(let_clause.variable.clone(), value);
        }

        let mut rows: Vec<std::collections::HashMap<String, Value>> =
            vec![initial_bindings.clone()];
        let mut mutation_count = 0;

        // Process body clauses in order
        for clause in &query.body_clauses {
            match clause {
                BodyClause::For(for_clause) => {
                    let mut new_rows = Vec::new();
                    for ctx in &rows {
                        let docs = if let Some(ref expr) = for_clause.source_expression {
                            let value = executor.evaluate_expr_with_context(expr, ctx)?;
                            match value {
                                Value::Array(arr) => arr,
                                other => vec![other],
                            }
                        } else {
                            let source_name = for_clause
                                .source_variable
                                .as_ref()
                                .unwrap_or(&for_clause.collection);
                            if let Some(value) = ctx.get(source_name) {
                                match value {
                                    Value::Array(arr) => arr.clone(),
                                    other => vec![other.clone()],
                                }
                            } else {
                                // Scan collection - check if sharded
                                let full_coll_name =
                                    format!("{}:{}", db_name, for_clause.collection);
                                let collection = state.storage.get_collection(&full_coll_name)?;
                                let shard_config = collection.get_shard_config();

                                if let (Some(config), Some(coordinator)) =
                                    (shard_config, &state.shard_coordinator)
                                {
                                    // Sharded collection - use scatter-gather
                                    // Execute async operation in blocking context
                                    let coordinator_clone = coordinator.clone();
                                    let db_name_owned = db_name.to_string();
                                    let coll_name_owned = for_clause.collection.clone();
                                    let config_clone = config.clone();

                                    match tokio::task::block_in_place(|| {
                                        tokio::runtime::Handle::current().block_on(async {
                                            coordinator_clone
                                                .scan_all_shards(
                                                    &db_name_owned,
                                                    &coll_name_owned,
                                                    &config_clone,
                                                )
                                                .await
                                        })
                                    }) {
                                        Ok(docs) => {
                                            docs.into_iter().map(|d| d.to_value()).collect()
                                        }
                                        Err(e) => {
                                            eprintln!("Scatter-gather failed: {:?}, using local shards only", e);
                                            collection
                                                .scan(None)
                                                .into_iter()
                                                .map(|d| d.to_value())
                                                .collect()
                                        }
                                    }
                                } else {
                                    // Non-sharded or no coordinator - local scan
                                    collection
                                        .scan(None)
                                        .into_iter()
                                        .map(|d| d.to_value())
                                        .collect()
                                }
                            }
                        };

                        for doc in docs {
                            let mut new_ctx = ctx.clone();
                            new_ctx.insert(for_clause.variable.clone(), doc);
                            new_rows.push(new_ctx);
                        }
                    }
                    rows = new_rows;
                }
                BodyClause::Let(let_clause) => {
                    for ctx in &mut rows {
                        let value =
                            executor.evaluate_expr_with_context(&let_clause.expression, ctx)?;
                        ctx.insert(let_clause.variable.clone(), value);
                    }
                }
                BodyClause::Filter(filter_clause) => {
                    rows.retain(|ctx| {
                        executor
                            .evaluate_filter_with_context(&filter_clause.expression, ctx)
                            .unwrap_or(false)
                    });
                }
                BodyClause::Insert(insert_clause) => {
                    let full_coll_name = format!("{}:{}", db_name, insert_clause.collection);
                    let collection = state.storage.get_collection(&full_coll_name)?;

                    for ctx in &rows {
                        let doc_value =
                            executor.evaluate_expr_with_context(&insert_clause.document, ctx)?;
                        collection.insert_tx(&mut tx, wal, doc_value)?;
                        mutation_count += 1;
                    }
                }
                BodyClause::Update(update_clause) => {
                    let full_coll_name = format!("{}:{}", db_name, update_clause.collection);
                    let collection = state.storage.get_collection(&full_coll_name)?;

                    for ctx in &rows {
                        let selector_value =
                            executor.evaluate_expr_with_context(&update_clause.selector, ctx)?;
                        let key = match &selector_value {
                            Value::String(s) => s.clone(),
                            Value::Object(obj) => obj.get("_key")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .ok_or_else(|| DbError::ExecutionError(
                                    "UPDATE: selector object must have a _key field".to_string()
                                ))?,
                            _ => return Err(DbError::ExecutionError(
                                "UPDATE: selector must be a string key or an object with _key field".to_string()
                            )),
                        };

                        let changes_value =
                            executor.evaluate_expr_with_context(&update_clause.changes, ctx)?;
                        collection.update_tx(&mut tx, wal, &key, changes_value)?;
                        mutation_count += 1;
                    }
                }
                BodyClause::Remove(remove_clause) => {
                    let full_coll_name = format!("{}:{}", db_name, remove_clause.collection);
                    let collection = state.storage.get_collection(&full_coll_name)?;

                    for ctx in &rows {
                        let selector_value =
                            executor.evaluate_expr_with_context(&remove_clause.selector, ctx)?;
                        let key = match &selector_value {
                            Value::String(s) => s.clone(),
                            Value::Object(obj) => obj.get("_key")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .ok_or_else(|| DbError::ExecutionError(
                                    "REMOVE: selector object must have a _key field".to_string()
                                ))?,
                            _ => return Err(DbError::ExecutionError(
                                "REMOVE: selector must be a string key or an object with _key field".to_string()
                            )),
                        };

                        collection.delete_tx(&mut tx, wal, &key)?;
                        mutation_count += 1;
                    }
                }
                _ => {}
            }
        }

        // Return mutation result
        return Ok(ApiResponse::new(
            ExecuteQueryResponse {
                result: vec![serde_json::json!({
                    "mutationCount": mutation_count,
                    "message": format!("{} operation(s) staged in transaction. Commit to apply changes.", mutation_count)
                })],
                count: 1,
                has_more: false,
                id: None,
                cached: false,
                execution_time_ms: 0.0,
                documents_inserted: 0, // Transactional mutations are not counted until commit
                documents_updated: 0,
                documents_removed: 0,
            },
            &headers,
        ));
    }

    // Non-transactional execution (existing logic)
    let query = parse(&req.query)?;

    // Handle CREATE STREAM clause
    if let Some(ref _create_stream) = query.create_stream_clause {
        if let Some(manager) = &state.stream_manager {
            match manager.create_stream(&db_name, query) {
                Ok(_name) => {
                    return Ok(ApiResponse::new(
                        ExecuteQueryResponse {
                            result: Vec::new(),
                            count: 0,
                            has_more: false,
                            id: None,
                            cached: false,
                            execution_time_ms: 0.0,
                            documents_inserted: 0,
                            documents_updated: 0,
                            documents_removed: 0,
                        },
                        &headers,
                    ));
                }
                Err(e) => return Err(e),
            }
        } else {
            return Err(DbError::OperationNotSupported(
                "Stream processing not enabled".to_string(),
            ));
        }
    }

    let batch_size = req.batch_size;

    // Clone db_name and query text for slow query logging (before they're moved)
    let db_name_for_logging = db_name.clone();
    let query_text_for_logging = req.query.clone();

    // Only use spawn_blocking for potentially long-running queries
    // (mutations or range iterations). Simple reads run directly.
    let (query_result, execution_time_ms) = if is_long_running_query(&query) {
        let storage = state.storage.clone();
        let bind_vars = req.bind_vars.clone();
        let replication_log = state.replication_log.clone();
        let shard_coordinator = state.shard_coordinator.clone();
        let is_scatter_gather = headers.contains_key("X-Scatter-Gather");

        // Apply timeout to prevent DoS from long-running queries
        match tokio::time::timeout(
            std::time::Duration::from_secs(QUERY_TIMEOUT_SECS),
            tokio::task::spawn_blocking(move || {
                let mut executor = if bind_vars.is_empty() {
                    QueryExecutor::with_database(&storage, db_name)
                } else {
                    QueryExecutor::with_database_and_bind_vars(&storage, db_name, bind_vars)
                };

                // Add replication service for mutation logging
                if let Some(ref log) = replication_log {
                    executor = executor.with_replication(log);
                }

                // Inject shard coordinator for scatter-gather (if not already a sub-query)
                if !is_scatter_gather {
                    if let Some(coord) = shard_coordinator {
                        executor = executor.with_shard_coordinator(coord);
                    }
                }

                let start = std::time::Instant::now();
                let result = executor.execute_with_stats(&query)?;
                let execution_time_ms = start.elapsed().as_secs_f64() * 1000.0;
                Ok::<_, DbError>((result, execution_time_ms))
            }),
        )
        .await
        {
            Ok(join_result) => join_result
                .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??,
            Err(_) => {
                return Err(DbError::BadRequest(format!(
                    "Query execution timeout: exceeded {} seconds",
                    QUERY_TIMEOUT_SECS
                )))
            }
        }
    } else {
        let mut executor = if req.bind_vars.is_empty() {
            QueryExecutor::with_database(&state.storage, db_name)
        } else {
            QueryExecutor::with_database_and_bind_vars(&state.storage, db_name, req.bind_vars)
        };

        // Add replication service for mutation logging
        if let Some(ref log) = state.replication_log {
            executor = executor.with_replication(log);
        }

        // Inject shard coordinator for scatter-gather (if not already a sub-query)
        if !headers.contains_key("X-Scatter-Gather") {
            if let Some(coordinator) = state.shard_coordinator.clone() {
                executor = executor.with_shard_coordinator(coordinator);
            }
        }

        let start = std::time::Instant::now();
        let result = executor.execute_with_stats(&query)?;
        let execution_time_ms = start.elapsed().as_secs_f64() * 1000.0;
        (result, execution_time_ms)
    };

    let total_count = query_result.results.len();
    let mutations = &query_result.mutations;

    // Log slow query if it exceeds threshold (async, non-blocking)
    log_slow_query(
        state.storage.clone(),
        db_name_for_logging,
        query_text_for_logging,
        execution_time_ms,
        total_count,
        mutations.documents_inserted,
        mutations.documents_updated,
        mutations.documents_removed,
    );

    if total_count > batch_size {
        let cursor_id = state.cursor_store.store(query_result.results, batch_size);
        let (first_batch, has_more) = state
            .cursor_store
            .get_next_batch(&cursor_id)
            .unwrap_or((vec![], false));

        Ok(ApiResponse::new(
            ExecuteQueryResponse {
                result: first_batch,
                count: total_count,
                has_more,
                id: if has_more { Some(cursor_id) } else { None },
                cached: false,
                execution_time_ms,
                documents_inserted: mutations.documents_inserted,
                documents_updated: mutations.documents_updated,
                documents_removed: mutations.documents_removed,
            },
            &headers,
        ))
    } else {
        Ok(ApiResponse::new(
            ExecuteQueryResponse {
                result: query_result.results,
                count: total_count,
                has_more: false,
                id: None,
                cached: false,
                execution_time_ms,
                documents_inserted: mutations.documents_inserted,
                documents_updated: mutations.documents_updated,
                documents_removed: mutations.documents_removed,
            },
            &headers,
        ))
    }
}

pub async fn explain_query(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    headers: HeaderMap,
    Json(req): Json<ExecuteQueryRequest>,
) -> Result<Json<crate::sdbql::QueryExplain>, DbError> {
    let query = parse(&req.query)?;

    // explain() is fast - no need for spawn_blocking
    let mut executor = if req.bind_vars.is_empty() {
        QueryExecutor::with_database(&state.storage, db_name)
    } else {
        QueryExecutor::with_database_and_bind_vars(&state.storage, db_name, req.bind_vars)
    };

    // Inject shard coordinator for explain (if not already a sub-query)
    if !headers.contains_key("X-Scatter-Gather") {
        if let Some(coordinator) = state.shard_coordinator.clone() {
            executor = executor.with_shard_coordinator(coordinator);
        }
    }

    let explain = executor.explain(&query)?;

    Ok(Json(explain))
}

pub async fn get_next_batch(
    State(state): State<AppState>,
    Path(cursor_id): Path<String>,
) -> Result<Json<ExecuteQueryResponse>, DbError> {
    if let Some((batch, has_more)) = state.cursor_store.get_next_batch(&cursor_id) {
        let count = batch.len();
        Ok(Json(ExecuteQueryResponse {
            result: batch,
            count,
            has_more,
            id: if has_more { Some(cursor_id) } else { None },
            cached: true,
            execution_time_ms: 0.0, // Cached results, no execution time
            documents_inserted: 0,  // Mutations already counted in first response
            documents_updated: 0,
            documents_removed: 0,
        }))
    } else {
        Err(DbError::DocumentNotFound(format!(
            "Cursor not found or expired: {}",
            cursor_id
        )))
    }
}

pub async fn delete_cursor(
    State(state): State<AppState>,
    Path(cursor_id): Path<String>,
) -> Result<StatusCode, DbError> {
    if state.cursor_store.delete(&cursor_id) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(DbError::DocumentNotFound(format!(
            "Cursor not found: {}",
            cursor_id
        )))
    }
}
