
// ==================== Transaction Handlers ====================

use axum::{
    extract::{Path, State},
   http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::handlers::AppState;
use crate::error::DbError;
use crate::transaction::{IsolationLevel, TransactionId};

#[derive(Debug, Deserialize)]
pub struct BeginTransactionRequest {
    #[serde(rename = "isolationLevel", default)]
    pub isolation_level: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BeginTransactionResponse {
    pub id: String,
    #[serde(rename = "isolationLevel")]
    pub isolation_level: String,
    pub status: String,
}

pub async fn begin_transaction(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<BeginTransactionRequest>,
) -> Result<Json<BeginTransactionResponse>, DbError> {
    // Ensure database exists
    let _ = state.storage.get_database(&db_name)?;
    
    // Initialize transaction manager if needed
    let tx_manager = state.storage.transaction_manager()?;
    
    // Parse isolation level
    let isolation_level = match req.isolation_level.as_deref() {
        Some("read_uncommitted") => IsolationLevel::ReadUncommitted,
        Some("read_committed") | None => IsolationLevel::ReadCommitted,
        Some("repeatable_read") => IsolationLevel::RepeatableRead,
        Some("serializable") => IsolationLevel::Serializable,
        Some(level) => {
            return Err(DbError::InvalidDocument(format!(
                "Unknown isolation level: {}",
                level
            )))
        }
    };
    
    // Begin transaction
    let tx_id = tx_manager.begin(isolation_level)?;
    
    Ok(Json(BeginTransactionResponse {
        id: tx_id.to_string(),
        isolation_level: format!("{:?}", isolation_level),
        status: "active".to_string(),
    }))
}

#[derive(Debug, Serialize)]
pub struct CommitTransactionResponse {
    pub id: String,
    pub status: String,
}

pub async fn commit_transaction(
    State(state): State<AppState>,
    Path((_db_name, tx_id_str)): Path<(String, String)>,
) -> Result<Json<CommitTransactionResponse>, DbError> {
    // Parse transaction ID
    let tx_id_value: u64 = tx_id_str
        .strip_prefix("tx:")
        .unwrap_or(&tx_id_str)
        .parse()
        .map_err(|_| DbError::InvalidDocument("Invalid transaction ID".to_string()))?;
    let tx_id = TransactionId::from_u64(tx_id_value);
    
   // Commit transaction
    state.storage.commit_transaction(tx_id)?;
    
    Ok(Json(CommitTransactionResponse {
        id: tx_id.to_string(),
        status: "committed".to_string(),
    }))
}

pub async fn rollback_transaction(
    State(state): State<AppState>,
    Path((_db_name, tx_id_str)): Path<(String, String)>,
) -> Result<Json<CommitTransactionResponse>, DbError> {
    // Parse transaction ID
    let tx_id_value: u64 = tx_id_str
        .strip_prefix("tx:")
        .unwrap_or(&tx_id_str)
        .parse()
        .map_err(|_| DbError::InvalidDocument("Invalid transaction ID".to_string()))?;
    let tx_id = TransactionId::from_u64(tx_id_value);
    
    // Rollback transaction
    state.storage.rollback_transaction(tx_id)?;
    
    Ok(Json(CommitTransactionResponse {
        id: tx_id.to_string(),
        status: "aborted".to_string(),
    }))
}

// Transaction document operations

pub async fn insert_document_tx(
    State(state): State<AppState>,
    Path((_db_name, tx_id_str, coll_name)): Path<(String, String, String)>,
    Json(data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    // Parse transaction ID
    let tx_id_value: u64 = tx_id_str
        .strip_prefix("tx:")
        .unwrap_or(&tx_id_str)
        .parse()
        .map_err(|_| DbError::InvalidDocument("Invalid transaction ID".to_string()))?;
    let tx_id = TransactionId::from_u64(tx_id_value);
    
    // Get transaction manager
    let tx_manager = state.storage.transaction_manager()?;
    
    // Get transaction
    let tx_arc = tx_manager.get(tx_id)?;
    let mut tx = tx_arc.write().unwrap();
    
    // Get collection
    let collection = state.storage.get_collection(&coll_name)?;
    
    // Perform transactional insert
    let wal = tx_manager.wal();
    let doc = collection.insert_tx(&mut tx, wal, data)?;
    
    Ok(Json(doc.to_value()))
}

pub async fn update_document_tx(
    State(state): State<AppState>,
    Path((_db_name, tx_id_str, coll_name, key)): Path<(String, String, String, String)>,
    Json(data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    // Parse transaction ID
    let tx_id_value: u64 = tx_id_str
        .strip_prefix("tx:")
        .unwrap_or(&tx_id_str)
        .parse()
        .map_err(|_| DbError::InvalidDocument("Invalid transaction ID".to_string()))?;
    let tx_id = TransactionId::from_u64(tx_id_value);
    
    // Get transaction manager
    let tx_manager = state.storage.transaction_manager()?;
    
    // Get transaction
    let tx_arc = tx_manager.get(tx_id)?;
    let mut tx = tx_arc.write().unwrap();
    
    // Get collection
    let collection = state.storage.get_collection(&coll_name)?;
    
    // Perform transactional update
    let wal = tx_manager.wal();
    let doc = collection.update_tx(&mut tx, wal, &key, data)?;
    
    Ok(Json(doc.to_value()))
}

pub async fn delete_document_tx(
    State(state): State<AppState>,
    Path((_db_name, tx_id_str, coll_name, key)): Path<(String, String, String, String)>,
) -> Result<StatusCode, DbError> {
    // Parse transaction ID
    let tx_id_value: u64 = tx_id_str
        .strip_prefix("tx:")
        .unwrap_or(&tx_id_str)
        .parse()
        .map_err(|_| DbError::InvalidDocument("Invalid transaction ID".to_string()))?;
    let tx_id = TransactionId::from_u64(tx_id_value);
    
    // Get transaction manager
    let tx_manager = state.storage.transaction_manager()?;
    
    // Get transaction
    let tx_arc = tx_manager.get(tx_id)?;
    let mut tx = tx_arc.write().unwrap();
    
    // Get collection
    let collection = state.storage.get_collection(&coll_name)?;
    
    // Perform transactional delete
    let wal = tx_manager.wal();
    collection.delete_tx(&mut tx, wal, &key)?;
    
    Ok(StatusCode::NO_CONTENT)
}

// Transactional SDBQL execution



#[derive(Debug, Deserialize)]
pub struct ExecuteSdbqlTransactionalRequest {
    pub query: String,
    #[serde(default)]
    pub bind_vars: std::collections::HashMap<String, Value>,
}

pub async fn execute_transactional_sdbql(
    State(state): State<AppState>,
    Path((db_name, tx_id_str)): Path<(String, String)>,
    Json(req): Json<ExecuteSdbqlTransactionalRequest>,
) -> Result<Json<Value>, DbError> {
    use crate::sdbql::{parse, QueryExecutor};
    use crate::sdbql::ast::BodyClause;
    
    // Parse transaction ID
    let tx_id_value: u64 = tx_id_str
        .strip_prefix("tx:")
        .unwrap_or(&tx_id_str)
        .parse()
        .map_err(|_| DbError::InvalidDocument("Invalid transaction ID".to_string()))?;
    let tx_id = TransactionId::from_u64(tx_id_value);
    
    // Get transaction manager
    let tx_manager = state.storage.transaction_manager()?;
    
    // Get transaction
    let tx_arc = tx_manager.get(tx_id)?;
    let mut tx = tx_arc.write().unwrap();
    let wal = tx_manager.wal();
    
    // Parse SDBQL query
    let query = parse(&req.query)?;
    
    // For transactional SDBQL, we need to intercept mutations (INSERT/UPDATE/REMOVE)
    // and execute them transactionally. For now, we support queries with a single mutation operation.
    
    // Check if query contains mutation operations
    let mut has_insert = false;
    let mut has_update = false;
    let mut has_remove = false;
    
    for clause in &query.body_clauses {
        match clause {
            BodyClause::Insert(_) => has_insert = true,
            BodyClause::Update(_) => has_update = true,
            BodyClause::Remove(_) => has_remove = true,
            _ => {}
        }
    }
    
    if !has_insert && !has_update && !has_remove {
        // No mutations - just execute normally (read operations)
        let executor = QueryExecutor::with_database_and_bind_vars(
            &state.storage,
            db_name.clone(),
            req.bind_vars.clone(),
        );
        let results = executor.execute(&query)?;
        return Ok(Json(serde_json::json!({"result": results})));
    }
    
    // For mutation operations, we need to handle them specially
    // Create executor and execute to get the data to mutate
    let executor = QueryExecutor::with_database_and_bind_vars(
        &state.storage,
        db_name.clone(),
        req.bind_vars.clone(),
    );
    
    // Execute body clauses manually to intercept mutations
    let mut initial_bindings = std::collections::HashMap::new();
    
    // Merge bind variables
    for (key, value) in &req.bind_vars {
        initial_bindings.insert(format!("@{}", key), value.clone());
    }
    
    // Process LET clauses
    for let_clause in &query.let_clauses {
        let value = executor.evaluate_expr_with_context(&let_clause.expression, &initial_bindings)?;
        initial_bindings.insert(let_clause.variable.clone(), value);
    }
    
    // Process body clauses to build row contexts (FOR, LET, FILTER)
    // Then apply mutations (INSERT/UPDATE/REMOVE) transactionally for each row
    
    let mut rows: Vec<std::collections::HashMap<String, Value>> = vec![initial_bindings.clone()];
    let mut mutation_count = 0;
    
    // Process body clauses in order
    for clause in &query.body_clauses {
        match clause {
            BodyClause::For(for_clause) => {
                // Build new rows by iterating over the source
                let mut new_rows = Vec::new();
                for ctx in &rows {
                    // Get documents from the FOR source
                    let docs = if let Some(ref expr) = for_clause.source_expression {
                        // Expression-based source (e.g., range, array)
                        let value = executor.evaluate_expr_with_context(expr, ctx)?;
                        match value {
                            Value::Array(arr) => arr,
                            other => vec![other],
                        }
                    } else {
                        // Collection source
                        let source_name = for_clause.source_variable.as_ref().unwrap_or(&for_clause.collection);
                        
                        // Check if it's a variable in context first
                        if let Some(value) = ctx.get(source_name) {
                            match value {
                                Value::Array(arr) => arr.clone(),
                                other => vec![other.clone()],
                            }
                        } else {
                            // It's a collection - scan it
                            let full_coll_name = format!("{}:{}", db_name, for_clause.collection);
                            let collection = state.storage.get_collection(&full_coll_name)?;
                            collection.scan(None).into_iter().map(|d| d.to_value()).collect()
                        }
                    };
                    
                    // Create new context for each document
                    for doc in docs {
                        let mut new_ctx = ctx.clone();
                        new_ctx.insert(for_clause.variable.clone(), doc);
                        new_rows.push(new_ctx);
                    }
                }
                rows = new_rows;
            }
            BodyClause::Let(let_clause) => {
                // Evaluate LET expression for each row
                for ctx in &mut rows {
                    let value = executor.evaluate_expr_with_context(&let_clause.expression, ctx)?;
                    ctx.insert(let_clause.variable.clone(), value);
                }
            }
            BodyClause::Filter(filter_clause) => {
                // Filter rows based on condition
                rows.retain(|ctx| {
                    executor.evaluate_filter_with_context(&filter_clause.expression, ctx).unwrap_or(false)
                });
            }
            BodyClause::Insert(insert_clause) => {
                // Get collection
                let full_coll_name = format!("{}:{}", db_name, insert_clause.collection);
                let collection = state.storage.get_collection(&full_coll_name)?;
                
                // Insert for each row context
                for ctx in &rows {
                    let doc_value = executor.evaluate_expr_with_context(&insert_clause.document, ctx)?;
                    collection.insert_tx(&mut tx, wal, doc_value)?;
                    mutation_count += 1;
                }
            }
            BodyClause::Update(update_clause) => {
                // Get collection
                let full_coll_name = format!("{}:{}", db_name, update_clause.collection);
                let collection = state.storage.get_collection(&full_coll_name)?;
                
                // Update for each row context
                for ctx in &rows {
                    let selector_value = executor.evaluate_expr_with_context(&update_clause.selector, ctx)?;
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
                    
                    let changes_value = executor.evaluate_expr_with_context(&update_clause.changes, ctx)?;
                    collection.update_tx(&mut tx, wal, &key, changes_value)?;
                    mutation_count += 1;
                }
            }
            BodyClause::Remove(remove_clause) => {
                // Get collection
                let full_coll_name = format!("{}:{}", db_name, remove_clause.collection);
                let collection = state.storage.get_collection(&full_coll_name)?;
                
                // Remove for each row context
                for ctx in &rows {
                    let selector_value = executor.evaluate_expr_with_context(&remove_clause.selector, ctx)?;
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
            BodyClause::Upsert(upsert_clause) => {
                let full_coll_name = format!("{}:{}", db_name, upsert_clause.collection);
                let collection = state.storage.get_collection(&full_coll_name)?;
                
                for ctx in &rows {
                    let search_value = executor.evaluate_expr_with_context(&upsert_clause.search, ctx)?;
                    
                    let mut found_doc_key: Option<String> = None;
                    // Simple key lookup logic
                    if let Some(s) = search_value.as_str() {
                        if collection.get(s).is_ok() { found_doc_key = Some(s.to_string()); }
                    } else if let Some(obj) = search_value.as_object() {
                        if let Some(k) = obj.get("_key").or_else(|| obj.get("_id")) {
                            if let Some(ks) = k.as_str() {
                                if collection.get(ks).is_ok() { found_doc_key = Some(ks.to_string()); }
                            }
                        }
                    }

                    if let Some(key) = found_doc_key {
                        let update_value = executor.evaluate_expr_with_context(&upsert_clause.update, ctx)?;
                        collection.update_tx(&mut tx, wal, &key, update_value)?;
                    } else {
                        let insert_value = executor.evaluate_expr_with_context(&upsert_clause.insert, ctx)?;
                        collection.insert_tx(&mut tx, wal, insert_value)?;
                    }
                    mutation_count += 1;
                }
            }
            // Graph traversal clauses - not yet supported in transactions
            BodyClause::GraphTraversal(_) | BodyClause::ShortestPath(_) => {
                return Err(DbError::ExecutionError(
                    "Graph traversals not yet supported in transactions".to_string()
                ));
            }
            // COLLECT clause - not yet supported in transactions
            BodyClause::Collect(_) => {
                return Err(DbError::ExecutionError(
                    "COLLECT aggregation not yet supported in transactions".to_string()
                ));
            }
        }
    }
    
    // Return success (mutations are staged in transaction)
    Ok(Json(serde_json::json!({
        "result": [],
        "mutationCount": mutation_count,
        "message": format!("{} operation(s) staged in transaction. Commit to apply changes.", mutation_count)
    })))
}
