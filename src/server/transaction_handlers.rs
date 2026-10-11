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
use crate::storage::query_cache;
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

    // Commit transaction. It re-reads every touched document and ends in a
    // synced RocksDB write, so keep it off the async workers.
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || storage.commit_transaction(tx_id))
        .await
        .map_err(|e| DbError::InternalError(format!("Commit task failed: {}", e)))??;

    // Invalidate query cache since committed data is now visible
    query_cache::get_query_cache().invalidate_all();

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

/// Resolve a collection for a transactional document operation.
///
/// SEC-179: these handlers used to discard the `{db}` path segment and pass the
/// bare collection name to `StorageEngine::get_collection`, which resolves a
/// column family by literal name and then falls back to `_system:{name}`. A
/// principal scoped to one database could therefore write into `_system`
/// collections — authorization had already run against the `{db}` it ignored —
/// while ordinary collections were unreachable, because `{db}:{collection}` was
/// never tried.
///
/// Going through `Database` applies the `{db}:` prefix and the
/// credential-collection guard, exactly as every non-transactional handler does.
///
/// SEC-180: that guard is only the *first* of the three tiers in
/// `storage::protected`. `get_collection` rejects the five credential
/// collections and nothing else, so `_scripts`, `_services`, `_triggers`,
/// `_views`, `_graphs`, `_config`, `_rag_pipelines` and `_jobs` stayed writable
/// by name through here — and `_scripts` is how Lua gets installed for the
/// service router to execute. Write paths resolve through
/// `get_collection_for_write`, which runs `check_write_access` for all three
/// tiers against the caller.
///
/// The actor is always the *caller*, never `WriteActor::Server`: the collection
/// name arrived over the wire.
fn collection_for_write_in_database(
    state: &AppState,
    db_name: &str,
    coll_name: &str,
    actor: crate::storage::WriteActor,
) -> Result<crate::storage::Collection, DbError> {
    state
        .storage
        .get_database(db_name)?
        .get_collection_for_write(coll_name, actor)
}

pub async fn insert_document_tx(
    State(state): State<AppState>,
    claims: Option<axum::Extension<crate::server::auth::Claims>>,
    Path((db_name, tx_id_str, coll_name)): Path<(String, String, String)>,
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

    // Scoped to the database named in the path (SEC-179), and resolved through
    // the write getter so all three protection tiers apply (SEC-180).
    let collection = collection_for_write_in_database(
        &state,
        &db_name,
        &coll_name,
        crate::server::handlers::query::write_actor_from_claims(claims.as_deref()),
    )?;

    // Perform transactional insert
    let wal = tx_manager.wal().clone();
    let lock_manager = tx_manager.lock_manager().clone();
    let doc = collection.insert_tx(&mut tx, &wal, &lock_manager, data)?;

    Ok(Json(doc.to_value()))
}

/// `GET …/transaction/{tx_id}/document/{collection}/{key}`: the document as
/// the transaction sees it — its own uncommitted insert or update, missing
/// after its own delete, otherwise the committed document.
pub async fn get_document_tx(
    State(state): State<AppState>,
    Path((db_name, tx_id_str, coll_name, key)): Path<(String, String, String, String)>,
) -> Result<Json<Value>, DbError> {
    let tx_id_value: u64 = tx_id_str
        .strip_prefix("tx:")
        .unwrap_or(&tx_id_str)
        .parse()
        .map_err(|_| DbError::InvalidDocument("Invalid transaction ID".to_string()))?;
    let tx_id = TransactionId::from_u64(tx_id_value);
    let tx_manager = state.storage.transaction_manager()?;
    let tx_arc = tx_manager.get(tx_id)?;
    let tx = tx_arc.read().unwrap();

    // Scoped to the database named in the path (SEC-179), as a plain read.
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    let lock_manager = tx_manager.lock_manager().clone();
    match collection.get_tx_view(&tx, &lock_manager, &key)? {
        Some(doc) => Ok(Json(doc.to_value())),
        None => Err(DbError::DocumentNotFound(key)),
    }
}

pub async fn update_document_tx(
    State(state): State<AppState>,
    claims: Option<axum::Extension<crate::server::auth::Claims>>,
    Path((db_name, tx_id_str, coll_name, key)): Path<(String, String, String, String)>,
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

    // Scoped to the database named in the path (SEC-179), and resolved through
    // the write getter so all three protection tiers apply (SEC-180).
    let collection = collection_for_write_in_database(
        &state,
        &db_name,
        &coll_name,
        crate::server::handlers::query::write_actor_from_claims(claims.as_deref()),
    )?;

    // Perform transactional update
    let wal = tx_manager.wal().clone();
    let lock_manager = tx_manager.lock_manager().clone();
    let doc = collection.update_tx(&mut tx, &wal, &lock_manager, &key, data)?;

    Ok(Json(doc.to_value()))
}

pub async fn delete_document_tx(
    State(state): State<AppState>,
    claims: Option<axum::Extension<crate::server::auth::Claims>>,
    Path((db_name, tx_id_str, coll_name, key)): Path<(String, String, String, String)>,
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

    // Scoped to the database named in the path (SEC-179), and resolved through
    // the write getter so all three protection tiers apply (SEC-180).
    let collection = collection_for_write_in_database(
        &state,
        &db_name,
        &coll_name,
        crate::server::handlers::query::write_actor_from_claims(claims.as_deref()),
    )?;

    // Perform transactional delete
    let wal = tx_manager.wal().clone();
    let lock_manager = tx_manager.lock_manager().clone();
    collection.delete_tx(&mut tx, &wal, &lock_manager, &key)?;

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
    axum::Extension(claims): axum::Extension<crate::server::auth::Claims>,
    Json(req): Json<ExecuteSdbqlTransactionalRequest>,
) -> Result<Json<Value>, DbError> {
    use crate::sdbql::{parse, QueryExecutor};

    let query = parse(&req.query)?;

    // The authz middleware only required Read for the /query suffix; upgrade
    // to Write when the transactional query mutates.
    if query.has_mutations() {
        crate::server::authz_middleware::enforce(
            &claims,
            &state,
            crate::server::authorization::PermissionAction::Write,
            Some(&db_name),
        )
        .await?;
    }

    // Parse transaction ID
    let tx_id_value: u64 = tx_id_str
        .strip_prefix("tx:")
        .unwrap_or(&tx_id_str)
        .parse()
        .map_err(|_| DbError::InvalidDocument("Invalid transaction ID".to_string()))?;
    let tx_id = TransactionId::from_u64(tx_id_value);

    let tx_manager = state.storage.transaction_manager()?;
    let tx_arc = tx_manager.get(tx_id)?;

    // The query runs on the ordinary executor, so every clause it supports
    // (JOIN, COLLECT, graph traversals, windows, SORT/LIMIT, RETURN...) works
    // here too. Its writes are staged on the transaction instead of applied,
    // and land at commit. Collections are resolved through the executor's
    // write getter under the caller's principal, so the database scoping
    // (SEC-179) and the three protection tiers (SEC-180) apply as for /cursor.
    let mut executor = QueryExecutor::with_database_and_bind_vars(
        &state.storage,
        db_name.clone(),
        req.bind_vars.clone(),
    )
    .with_principal(crate::server::handlers::query::principal_from_claims(
        &claims,
    ))
    .with_timeout(std::time::Duration::from_secs(30));
    let mutates = query.has_mutations();
    // Only document writes can be staged; anything else would apply at once
    // and survive a rollback.
    if query.has_unstageable_writes() {
        return Err(DbError::OperationNotSupported(
            "streams, materialized views and state-changing functions cannot run \
             inside a transaction; they would apply immediately and survive a rollback"
                .to_string(),
        ));
    }
    if mutates {
        executor = executor.with_transaction(crate::sdbql::executor::TxWriter {
            tx: tx_arc,
            wal: tx_manager.wal().clone(),
            locks: tx_manager.lock_manager().clone(),
        });
    }

    let outcome = executor.execute_with_stats(&query)?;

    if !mutates {
        return Ok(Json(serde_json::json!({"result": outcome.results})));
    }
    let staged = outcome.mutations.total();
    Ok(Json(serde_json::json!({
        "result": outcome.results,
        "mutationCount": staged,
        "message": format!("{} operation(s) staged in transaction. Commit to apply changes.", staged)
    })))
}
