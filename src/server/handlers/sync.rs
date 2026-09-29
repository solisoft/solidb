use super::system::AppState;
use crate::error::DbError;
use crate::server::auth::Claims;
use crate::storage::WriteActor;
use crate::sync::{
    conflict_store,
    protocol::Operation,
    session::{
        validate_device_id, ChangeOperation, SyncChange, SyncSession, SyncSessionManager,
        MAX_FILTER_QUERY_LEN, MAX_SESSIONS_PER_PRINCIPAL, MAX_SUBSCRIPTIONS, MAX_SUBSCRIPTION_LEN,
        MAX_TOTAL_SESSIONS,
    },
    LogEntry, VersionVector,
};
use axum::{
    extract::{Query, State},
    response::Json,
};
use serde::Deserialize;
use serde_json::Value;

/// Evaluate a simple filter expression against a document
///
/// The filter_query should be a simple SDBQL filter expression like:
/// - "doc.status == 'active'"
/// - "doc.user_id == @userId"
///
/// For now, we support only basic comparisons. Complex queries require
/// the full SDBQL executor which is too heavy for per-document filtering.
fn evaluate_simple_filter(filter_query: &str, doc: &Value) -> bool {
    // Parse the filter to extract the comparison
    // Format: "doc.field OP value" or "field OP value"

    // Try to evaluate using simple pattern matching
    // This is a simplified evaluator for common filter patterns

    let filter = filter_query.trim();

    // Skip empty filters
    if filter.is_empty() {
        return true;
    }

    // Try to parse simple comparisons: field == value, field != value, etc.
    let ops = ["==", "!=", ">=", "<=", ">", "<"];

    for op in ops {
        if let Some(pos) = filter.find(op) {
            let left = filter[..pos].trim();
            let right = filter[pos + op.len()..].trim();

            // Get the field value from the document
            let field_value = get_nested_field(doc, left);

            // Parse the right-hand side value
            let compare_value = parse_filter_value(right);

            // Perform comparison
            return match op {
                "==" => values_equal(&field_value, &compare_value),
                "!=" => !values_equal(&field_value, &compare_value),
                ">" => compare_numbers(&field_value, &compare_value) > 0,
                "<" => compare_numbers(&field_value, &compare_value) < 0,
                ">=" => compare_numbers(&field_value, &compare_value) >= 0,
                "<=" => compare_numbers(&field_value, &compare_value) <= 0,
                _ => true,
            };
        }
    }

    // If we can't parse the filter, default to true (include the document)
    true
}

/// Get a nested field from a JSON value
/// Supports: "doc.field", "doc.nested.field", "field"
fn get_nested_field(doc: &Value, path: &str) -> Value {
    let parts: Vec<&str> = path.split('.').collect();

    // Skip "doc" prefix if present
    let start = if parts.first() == Some(&"doc") { 1 } else { 0 };

    let mut current = doc;
    for part in parts.iter().skip(start) {
        match current.get(*part) {
            Some(v) => current = v,
            None => return Value::Null,
        }
    }
    current.clone()
}

/// Parse a filter value (right-hand side of comparison)
fn parse_filter_value(value: &str) -> Value {
    let v = value.trim();

    // String literal: 'value' or "value"
    if (v.starts_with('\'') && v.ends_with('\'')) || (v.starts_with('"') && v.ends_with('"')) {
        return Value::String(v[1..v.len() - 1].to_string());
    }

    // Boolean
    if v == "true" {
        return Value::Bool(true);
    }
    if v == "false" {
        return Value::Bool(false);
    }

    // Null
    if v == "null" {
        return Value::Null;
    }

    // Number
    if let Ok(n) = v.parse::<i64>() {
        return Value::Number(n.into());
    }
    if let Ok(n) = v.parse::<f64>() {
        return serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null);
    }

    // Default to string
    Value::String(v.to_string())
}

/// Compare two JSON values for equality
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::String(s1), Value::String(s2)) => s1 == s2,
        (Value::Number(n1), Value::Number(n2)) => {
            n1.as_f64().unwrap_or(0.0) == n2.as_f64().unwrap_or(0.0)
        }
        (Value::Bool(b1), Value::Bool(b2)) => b1 == b2,
        (Value::Null, Value::Null) => true,
        _ => a == b,
    }
}

/// Compare two JSON values numerically
/// Returns -1, 0, or 1 like strcmp
fn compare_numbers(a: &Value, b: &Value) -> i32 {
    let a_num = match a {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    };
    let b_num = match b {
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    };

    if a_num < b_num {
        -1
    } else if a_num > b_num {
        1
    } else {
        0
    }
}

/// Convert a LogEntry from the replication log to a SyncChange for the client
fn log_entry_to_sync_change(entry: &LogEntry) -> SyncChange {
    SyncChange {
        database: entry.database.clone(),
        collection: entry.collection.clone(),
        document_key: entry.key.clone(),
        operation: match entry.operation {
            Operation::Insert => ChangeOperation::Insert,
            Operation::Update => ChangeOperation::Update,
            Operation::Delete => ChangeOperation::Delete,
            // Map other operations to appropriate types
            Operation::CreateCollection
            | Operation::DeleteCollection
            | Operation::TruncateCollection
            | Operation::CreateDatabase
            | Operation::DeleteDatabase
            | Operation::ColumnarInsert
            | Operation::ColumnarCreateCollection => ChangeOperation::Insert,
            Operation::ColumnarDelete
            | Operation::ColumnarDropCollection
            | Operation::ColumnarTruncate => ChangeOperation::Delete,
            _ => ChangeOperation::Update,
        },
        document_data: entry
            .data
            .as_ref()
            .and_then(|d| serde_json::from_slice(d).ok()),
        vector: VersionVector::with_node(&entry.node_id, entry.sequence),
        timestamp: entry.timestamp,
        is_delta: false,
        delta_patch: None,
        parent_vectors: vec![],
    }
}

/// Maximum number of log entries one pull may return (audit H3).
const MAX_PULL_LIMIT: usize = 1000;
const DEFAULT_PULL_LIMIT: usize = 100;

/// Clamp a client-supplied pull `limit` to `1..=MAX_PULL_LIMIT`.
fn clamp_pull_limit(requested: Option<u64>) -> usize {
    requested
        .map(|n| n.min(MAX_PULL_LIMIT as u64) as usize)
        .unwrap_or(DEFAULT_PULL_LIMIT)
        .max(1)
}

/// Whether a replication-log entry may be handed to a sync client at all,
/// before any per-database permission check.
///
/// The credential tier (`_admins`, `_api_keys`, `_env`, `_roles`,
/// `_user_roles`) is never readable by name, and the sync log carries its
/// rows verbatim — password and key hashes included (audit H3).
fn servable_to_sync_client(entry: &LogEntry) -> bool {
    !crate::storage::is_protected_collection(&entry.collection)
}

/// Look up a session and check it belongs to the caller.
///
/// A session is bound to the principal that registered it (audit M2). One
/// owned by somebody else is reported exactly like a missing one, so session
/// ids cannot be probed.
async fn owned_session(
    state: &AppState,
    session_id: &str,
    claims: &Claims,
) -> Result<SyncSession, DbError> {
    let session_manager = get_session_manager(state)?;
    let not_found = || DbError::BadRequest(format!("Session '{}' not found", session_id));
    let session = session_manager
        .get_session(session_id)
        .await
        .ok_or_else(not_found)?;
    if session.user_id.as_deref() != Some(claims.sub.as_str()) {
        return Err(not_found());
    }
    Ok(session)
}

fn get_session_manager(state: &AppState) -> Result<&SyncSessionManager, DbError> {
    state
        .sync_session_manager
        .as_deref()
        .ok_or_else(|| DbError::InternalError("Sync session manager not initialized".to_string()))
}

// ==================== Request/Response Types ====================

#[derive(Debug, Deserialize)]
pub struct RegisterSessionRequest {
    pub device_id: String,
    pub api_key: String,
    pub subscriptions: Option<Vec<String>>,
    pub filter_query: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SyncPullRequest {
    pub session_id: String,
    pub client_vector: VersionVector,
    pub limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct SyncPushRequest {
    pub session_id: String,
    pub changes: Vec<SyncChange>,
    pub client_vector: VersionVector,
}

#[derive(Debug, Deserialize)]
pub struct SyncAckRequest {
    pub session_id: String,
    pub applied_vector: VersionVector,
}

#[derive(Debug, Deserialize)]
pub struct ConflictsQuery {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ResolveConflictRequest {
    pub session_id: String,
    pub document_key: String,
    pub resolution: String, // "local" | "remote" | "merged"
    pub merged_data: Option<serde_json::Value>,
}

// ==================== Sync Session Handlers ====================

/// POST /_api/sync/session
/// Register a new sync session for offline-first synchronization
pub async fn register_sync_session(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<Claims>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, DbError> {
    // Devices pull from the sync log; a node with no peer only writes it once
    // one of them shows up (`SyncLog::defer_until_needed`).
    if let Some(log) = &state.replication_log {
        log.activate("offline-sync session");
    }
    // Parse request fields from JSON
    let device_id = req
        .get("device_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("device_id is required".to_string()))?
        .to_string();
    validate_device_id(&device_id).map_err(DbError::BadRequest)?;

    let api_key = req
        .get("api_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("api_key is required".to_string()))?
        .to_string();

    let subscriptions: Vec<String> = req
        .get("subscriptions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if subscriptions.len() > MAX_SUBSCRIPTIONS {
        return Err(DbError::BadRequest(format!(
            "at most {} subscriptions per session",
            MAX_SUBSCRIPTIONS
        )));
    }
    for sub in &subscriptions {
        if sub.len() > MAX_SUBSCRIPTION_LEN {
            return Err(DbError::BadRequest(format!(
                "subscription names are limited to {} bytes",
                MAX_SUBSCRIPTION_LEN
            )));
        }
        // Pull drops these anyway; refusing here tells the client why.
        if crate::storage::is_protected_collection(sub) {
            return Err(crate::storage::protected_collection_error(sub));
        }
    }

    let filter_query = req
        .get("filter_query")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    if filter_query
        .as_ref()
        .is_some_and(|f| f.len() > MAX_FILTER_QUERY_LEN)
    {
        return Err(DbError::BadRequest(format!(
            "filter_query is limited to {} bytes",
            MAX_FILTER_QUERY_LEN
        )));
    }

    // Get cluster secret for HMAC signing
    let cluster_secret = state.cluster_secret();
    let secret_bytes = cluster_secret.as_bytes();

    // Create new sync session with HMAC-signed session ID
    let mut session = if secret_bytes.is_empty() {
        // No cluster secret configured - use simple session ID (development mode)
        let session_id = format!("{}-{}", device_id, uuid::Uuid::new_v4());
        SyncSession::new(session_id, device_id.clone(), api_key)
    } else {
        // Use secure HMAC-signed session ID (production mode)
        SyncSession::new_secure(&device_id, &api_key, secret_bytes)
    };
    let session_id = session.session_id.clone();
    session.subscriptions = subscriptions;
    session.filter_query = filter_query;
    // Bind the session to its creator; every other endpoint checks this.
    session.user_id = Some(claims.sub.clone());

    get_session_manager(&state)?
        .register_session_bounded(session, MAX_SESSIONS_PER_PRINCIPAL, MAX_TOTAL_SESSIONS)
        .await
        .map_err(|e| DbError::RateLimited(e, 60))?;

    // Get server vector (for now, return empty - would be fetched from sync state)
    let server_vector = VersionVector::new();

    // Build capabilities response
    let capabilities = serde_json::json!({
        "delta_sync": true,
        "crdt_types": true,
        "compression": true,
        "max_batch_size": 1048576, // 1MB
    });

    Ok(Json(serde_json::json!({
        "session_id": session_id,
        "server_vector": server_vector,
        "capabilities": capabilities,
    })))
}

/// POST /_api/sync/pull
/// Pull changes from server to client
pub async fn pull_changes(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<crate::server::auth::Claims>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, DbError> {
    let session_id = req
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("session_id is required".to_string()))?
        .to_string();

    let session_manager = get_session_manager(&state)?;
    let session = owned_session(&state, &session_id, &claims).await?;

    // Verify session ID signature if cluster secret is configured
    let cluster_secret = state.cluster_secret();
    if !cluster_secret.is_empty()
        && !SyncSession::verify_session_id(&session_id, &session.api_key, cluster_secret.as_bytes())
    {
        return Err(DbError::BadRequest("Invalid session signature".to_string()));
    }

    // Parse client vector (used for conflict detection)
    let _client_vector: VersionVector = req
        .get("client_vector")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_else(VersionVector::new);

    let limit = clamp_pull_limit(req.get("limit").and_then(|v| v.as_u64()));

    // Get session's subscriptions and last sequence
    let subscriptions = &session.subscriptions;
    let after_sequence = session.last_sequence;

    // Query sync log for entries after client's sequence
    let sync_log = state
        .replication_log
        .as_ref()
        .ok_or_else(|| DbError::InternalError("Replication log not initialized".to_string()))?;

    sync_log.activate("offline-sync pull");
    let log_entries = sync_log.get_entries_after(after_sequence, limit);

    // Cursor and paging come from what was *read*, not what survives the
    // filters below: otherwise a session whose filters drop a whole page
    // never advances past it.
    let has_more = log_entries.len() == limit;
    let max_seq = log_entries
        .iter()
        .map(|e| e.sequence)
        .max()
        .unwrap_or(after_sequence);

    // Filter by subscriptions (if any subscriptions are specified), and never
    // serve the credential tier.
    let filtered: Vec<_> = log_entries
        .into_iter()
        .filter(servable_to_sync_client)
        .filter(|e| subscriptions.is_empty() || subscriptions.contains(&e.collection))
        .collect();

    // Apply filter query if specified (for partial sync)
    let filter_query = &session.filter_query;
    let filtered: Vec<_> = if let Some(ref filter) = filter_query {
        filtered
            .into_iter()
            .filter(|entry| {
                // Always include deletes (we need to propagate deletions)
                if entry.operation == Operation::Delete {
                    return true;
                }

                // Parse document data and apply filter
                entry
                    .data
                    .as_ref()
                    .and_then(|d| serde_json::from_slice::<Value>(d).ok())
                    .map(|doc| evaluate_simple_filter(filter, &doc))
                    .unwrap_or(true) // Include if we can't parse
            })
            .collect()
    } else {
        filtered
    };

    // Only hand out changes from databases the caller can read. The sync log
    // spans every database on the node; without this filter any sync session
    // receives all of them.
    let permissions =
        crate::server::AuthorizationService::get_effective_permissions(&claims, &state).await?;
    let scoped = claims.scoped_databases.as_deref();
    let mut allowed_dbs: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let filtered: Vec<_> = filtered
        .into_iter()
        .filter(|e| {
            *allowed_dbs.entry(e.database.clone()).or_insert_with(|| {
                crate::server::authz_middleware::enforce_raw(
                    &permissions,
                    crate::server::PermissionAction::Read,
                    Some(&e.database),
                    scoped,
                    &claims.sub,
                )
            })
        })
        .collect();

    // Convert LogEntry -> SyncChange
    let changes: Vec<SyncChange> = filtered.iter().map(log_entry_to_sync_change).collect();

    // Build server vector from the latest entries
    let mut server_vector = VersionVector::new();
    for entry in &filtered {
        // A max-merge, not an increment loop: that was O(sequence) per entry,
        // millions of iterations on a long-lived node.
        server_vector.merge(&VersionVector::with_node(&entry.node_id, entry.sequence));
    }

    // Update session with the new sequence
    session_manager
        .update_session_sequence(&session_id, max_seq)
        .await;
    session_manager
        .update_session_vector(&session_id, &server_vector)
        .await;

    // Conflicts would be detected if client_vector has concurrent changes
    // For now, return empty conflicts list (conflict detection happens on push)
    let conflicts: Vec<serde_json::Value> = vec![];

    Ok(Json(serde_json::json!({
        "changes": changes,
        "server_vector": server_vector,
        "has_more": has_more,
        "conflicts": conflicts,
    })))
}

/// Write a single pushed change into storage.
///
/// The collection is resolved through the write-tier guard with the caller's
/// identity (audit C2): a push is a client write like any other, so
/// `_scripts`, `_triggers`, the credential tier and — without Admin — `_jobs`
/// are refused before anything is created. A missing collection is created
/// on demand under the same rule as the document API
/// (`SOLIDB_AUTO_CREATE_COLLECTIONS`). A missing database is created only
/// when `can_create_database` says the caller holds the instance-level Admin
/// that `POST /_api/database` requires.
///
/// A delta change carries an RFC 6902 patch in `delta_patch`, applied to the
/// stored document. It needs a base: a patch for a document the server does
/// not have is refused, and the client should push the full document.
fn apply_sync_change(
    state: &AppState,
    change: &SyncChange,
    actor: WriteActor,
    can_create_database: bool,
) -> Result<(), DbError> {
    if change.is_delta && change.operation == ChangeOperation::Delete {
        return Err(DbError::BadRequest(
            "a delete cannot be a delta".to_string(),
        ));
    }

    // Refuse the protected tiers before anything is created on their behalf.
    crate::storage::check_write_access(&change.collection, actor)?;

    let is_write = matches!(
        change.operation,
        ChangeOperation::Insert | ChangeOperation::Update
    );

    let db = match state.storage.get_database(&change.database) {
        Ok(db) => db,
        Err(DbError::CollectionNotFound(_)) if is_write && can_create_database => {
            match state.storage.create_database(change.database.clone()) {
                Ok(()) => {
                    // Replicate the creation like `POST /_api/database` does,
                    // or peers receive documents for a database they lack.
                    if let Some(ref log) = state.replication_log {
                        log.append(LogEntry::new_op(
                            change.database.clone(),
                            "",
                            Operation::CreateDatabase,
                            "",
                            None,
                        ));
                    }
                }
                // Lost a race with a concurrent creator.
                Err(DbError::CollectionAlreadyExists(_)) => {}
                Err(e) => return Err(e),
            }
            state.storage.get_database(&change.database)?
        }
        Err(e) => return Err(e),
    };
    let collection = match db.get_collection_for_write(&change.collection, actor) {
        Ok(coll) => coll,
        Err(DbError::CollectionNotFound(_))
            if is_write && crate::storage::cf_ops::auto_create_enabled() =>
        {
            crate::storage::cf_ops::record_autocreate();
            if let Err(e) = db.create_collection(change.collection.clone(), None) {
                // Lost a race with a concurrent creator: fine, fetch below.
                if !matches!(e, DbError::CollectionAlreadyExists(_)) {
                    return Err(e);
                }
            }
            db.get_collection_for_write(&change.collection, actor)?
        }
        Err(e) => return Err(e),
    };

    match change.operation {
        ChangeOperation::Insert | ChangeOperation::Update if change.is_delta => {
            let patch: crate::sync::delta::JsonPatch = change
                .delta_patch
                .clone()
                .ok_or_else(|| {
                    DbError::BadRequest(format!(
                        "delta change for '{}' has no delta_patch",
                        change.document_key
                    ))
                })
                .and_then(|v| {
                    serde_json::from_value(v).map_err(|e| {
                        DbError::BadRequest(format!("delta_patch is not a JSON Patch: {}", e))
                    })
                })?;
            collection.patch_document(&change.document_key, &patch)?;
        }
        ChangeOperation::Insert | ChangeOperation::Update => {
            let data = change.document_data.clone().ok_or_else(|| {
                DbError::BadRequest(format!(
                    "change for '{}' has no document_data",
                    change.document_key
                ))
            })?;
            collection.upsert_batch(vec![(change.document_key.clone(), data)])?;
        }
        ChangeOperation::Delete => {
            // A delete for a key that is already gone is the desired end state,
            // not a failure — clients retry pushes after a dropped connection.
            if let Err(e) = collection.delete(&change.document_key) {
                if !matches!(e, DbError::DocumentNotFound(_)) {
                    return Err(e);
                }
            }
        }
    }

    Ok(())
}

/// Record an applied sync change in the replication log, under this node's id
/// and clock (audit H9: never the client's device id or timestamp — peers key
/// their dedupe and pull cursors on it). Returns the sequence it was given, or
/// the current one when there is no log.
fn log_sync_change(state: &AppState, change: &SyncChange) -> u64 {
    let Some(ref log) = state.replication_log else {
        return 0;
    };
    let operation = match change.operation {
        ChangeOperation::Insert => Operation::Insert,
        ChangeOperation::Update => Operation::Update,
        ChangeOperation::Delete => Operation::Delete,
    };
    let data_bytes = change
        .document_data
        .as_ref()
        .and_then(|d| serde_json::to_vec(d).ok());
    log.append(LogEntry::new_op(
        change.database.clone(),
        change.collection.clone(),
        operation,
        change.document_key.clone(),
        data_bytes,
    ))
}

/// POST /_api/sync/push
/// Push changes from client to server
pub async fn push_changes(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<crate::server::auth::Claims>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, DbError> {
    let session_id = req
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("session_id is required".to_string()))?
        .to_string();

    let session_manager = get_session_manager(&state)?;
    let session = owned_session(&state, &session_id, &claims).await?;

    // Verify session ID signature if cluster secret is configured
    let cluster_secret = state.cluster_secret();
    if !cluster_secret.is_empty()
        && !SyncSession::verify_session_id(&session_id, &session.api_key, cluster_secret.as_bytes())
    {
        return Err(DbError::BadRequest("Invalid session signature".to_string()));
    }

    // Parse changes
    let changes: Vec<SyncChange> = req
        .get("changes")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let client_vector: VersionVector = req
        .get("client_vector")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_else(VersionVector::new);

    let write_actor = crate::server::handlers::query::write_actor_from_claims(Some(&claims));

    // Per-database write permission, resolved once per distinct database.
    let permissions =
        crate::server::AuthorizationService::get_effective_permissions(&claims, &state).await?;
    let scoped = claims.scoped_databases.as_deref();
    // Audit C2: creating a database on demand takes the same instance-level
    // Admin as `POST /_api/database`, not just Write on the (absent) database.
    let can_create_database = crate::server::authz_middleware::enforce_raw(
        &permissions,
        crate::server::PermissionAction::Admin,
        None,
        scoped,
        &claims.sub,
    );
    let mut writable_dbs: std::collections::HashMap<String, bool> =
        std::collections::HashMap::new();

    let mut conflicts: Vec<serde_json::Value> = Vec::new();
    let mut accepted = 0;
    let mut rejected = 0;

    // Process each change
    for change in &changes {
        let writable = *writable_dbs
            .entry(change.database.clone())
            .or_insert_with(|| {
                crate::server::authz_middleware::enforce_raw(
                    &permissions,
                    crate::server::PermissionAction::Write,
                    Some(&change.database),
                    scoped,
                    &claims.sub,
                )
            });
        if !writable {
            rejected += 1;
            continue;
        }

        // A concurrent edit is held for resolution instead of applied. The
        // check only applies to documents this server has synced before and to
        // changes that carry a vector; everything else is last-write-wins.
        // See `sync::conflict_store`.
        if let Ok(db) = state.storage.get_database(&change.database) {
            match conflict_store::detect(&db, state.storage.node_id(), &session.device_id, change) {
                Ok(Some(info)) => {
                    match conflict_store::store_conflict(
                        &db,
                        &session_id,
                        &claims.sub,
                        &session.device_id,
                        change,
                        &info,
                    ) {
                        Ok(row) => {
                            let can_read = crate::server::authz_middleware::enforce_raw(
                                &permissions,
                                crate::server::PermissionAction::Read,
                                Some(&change.database),
                                scoped,
                                &claims.sub,
                            );
                            conflicts.push(conflict_store::client_view(&row, can_read));
                        }
                        Err(e) => {
                            tracing::warn!("sync push: could not store a conflict: {}", e);
                            rejected += 1;
                        }
                    }
                    continue;
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::warn!("sync push: conflict check failed: {}", e);
                    rejected += 1;
                    continue;
                }
            }
        }

        // Apply the change to storage.
        //
        // This handler used to count the change as accepted without writing
        // anything: a client got {"accepted": N} back and the document never
        // existed. `pull` reads from the replication log rather than storage,
        // so push→pull still round-tripped and hid it.
        //
        // Semantics match the cluster replication worker (`sync/worker.rs`):
        // an upsert, so the last write to arrive wins. Deliberately *not* a
        // timestamp comparison against the stored document — `change.timestamp`
        // is the client's HLC while `_updated_at` is set by this server's wall
        // clock, and dropping writes on that comparison would silently discard
        // data from any client whose clock runs behind.
        if let Err(e) = apply_sync_change(&state, change, write_actor, can_create_database) {
            tracing::warn!(
                "sync push: failed to apply {:?} on {}/{} key {}: {}",
                change.operation,
                change.database,
                change.collection,
                change.document_key,
                e
            );
            rejected += 1;
            continue;
        }
        accepted += 1;

        let seq = log_sync_change(&state, change);
        if let Ok(db) = state.storage.get_database(&change.database) {
            if let Err(e) = conflict_store::record_write(&db, change, seq, &session.device_id) {
                tracing::warn!("sync push: could not record the write: {}", e);
            }
        }
    }

    // Update server's version vector
    let mut server_vector = client_vector.clone();
    // Increment server counter
    server_vector.increment(&session.device_id);

    // Update session vector
    session_manager
        .update_session_vector(&session_id, &server_vector)
        .await;

    Ok(Json(serde_json::json!({
        "server_vector": server_vector,
        "conflicts": conflicts,
        "accepted": accepted,
        "rejected": rejected,
    })))
}

/// POST /_api/sync/ack
/// Acknowledge receipt of changes
pub async fn acknowledge_changes(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<Claims>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, DbError> {
    let session_id = req
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("session_id is required".to_string()))?
        .to_string();

    let applied_vector: VersionVector = req
        .get("applied_vector")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_else(VersionVector::new);

    let _session = owned_session(&state, &session_id, &claims).await?;

    // Update session vector to reflect acknowledged state
    get_session_manager(&state)?
        .update_session_vector(&session_id, &applied_vector)
        .await;

    Ok(Json(serde_json::json!({
        "success": true,
    })))
}

/// Every open conflict of `session_id` across databases, with the database it
/// lives in.
fn session_conflicts(state: &AppState, session_id: &str) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    for name in state.storage.list_databases() {
        if let Ok(db) = state.storage.get_database(&name) {
            for row in conflict_store::open_conflicts(&db, session_id) {
                out.push((name.clone(), row));
            }
        }
    }
    out
}

/// GET /_api/sync/conflicts
/// List unresolved conflicts for a session
///
/// A push that touches a document another device (or an ordinary write) changed
/// after the pushing client's last pull is not applied: it is held here until
/// [`resolve_conflict`] settles it. See `sync::conflict_store` for the rule.
pub async fn list_conflicts(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<Claims>,
    Query(params): Query<ConflictsQuery>,
) -> Result<Json<serde_json::Value>, DbError> {
    let _session = owned_session(&state, &params.session_id, &claims).await?;
    let permissions =
        crate::server::AuthorizationService::get_effective_permissions(&claims, &state).await?;
    let scoped = claims.scoped_databases.as_deref();
    let allowed = |db: &str, action| {
        crate::server::authz_middleware::enforce_raw(
            &permissions,
            action,
            Some(db),
            scoped,
            &claims.sub,
        )
    };

    let conflicts: Vec<Value> = session_conflicts(&state, &params.session_id)
        .into_iter()
        // Resolving writes to the database, so listing needs the same right.
        .filter(|(db, _)| allowed(db, crate::server::PermissionAction::Write))
        .map(|(db, row)| {
            let can_read = allowed(&db, crate::server::PermissionAction::Read);
            conflict_store::client_view(&row, can_read)
        })
        .collect();

    Ok(Json(serde_json::json!({
        "conflicts": conflicts,
        "count": conflicts.len(),
    })))
}

/// POST /_api/sync/resolve
/// Resolve a conflict manually
///
/// `resolution` is `local` (keep the server's document, drop the client's
/// change), `remote` (apply the client's change) or `merged` (store
/// `merged_data` as the document). `database` and `collection` are optional and
/// only needed when the same `document_key` is in conflict in more than one
/// place.
pub async fn resolve_conflict(
    State(state): State<AppState>,
    axum::Extension(claims): axum::Extension<Claims>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, DbError> {
    let text = |name: &str| req.get(name).and_then(|v| v.as_str()).map(str::to_string);
    let session_id = text("session_id")
        .ok_or_else(|| DbError::BadRequest("session_id is required".to_string()))?;
    let document_key = text("document_key")
        .ok_or_else(|| DbError::BadRequest("document_key is required".to_string()))?;
    let resolution = text("resolution")
        .ok_or_else(|| DbError::BadRequest("resolution is required".to_string()))?;
    let merged_data = req.get("merged_data").cloned();

    let _session = owned_session(&state, &session_id, &claims).await?;

    if !matches!(resolution.as_str(), "local" | "remote" | "merged") {
        return Err(DbError::BadRequest(
            "resolution must be 'local', 'remote', or 'merged'".to_string(),
        ));
    }
    if resolution == "merged" && !merged_data.as_ref().is_some_and(Value::is_object) {
        return Err(DbError::BadRequest(
            "merged_data (an object) is required when resolution is 'merged'".to_string(),
        ));
    }

    let (want_db, want_coll) = (text("database"), text("collection"));
    let mut candidates: Vec<(String, Value)> = session_conflicts(&state, &session_id)
        .into_iter()
        .filter(|(db, row)| {
            row.get("document_key").and_then(Value::as_str) == Some(document_key.as_str())
                && want_db.as_deref().is_none_or(|d| d == db)
                && want_coll
                    .as_deref()
                    .is_none_or(|c| row.get("collection").and_then(Value::as_str) == Some(c))
        })
        .collect();
    let (db_name, row) = match candidates.len() {
        0 => {
            return Err(DbError::DocumentNotFound(format!(
                "no open conflict for document '{}' in this session",
                document_key
            )))
        }
        1 => candidates.remove(0),
        n => {
            return Err(DbError::BadRequest(format!(
                "{} conflicts match document '{}'; pass 'database' and 'collection'",
                n, document_key
            )))
        }
    };

    let permissions =
        crate::server::AuthorizationService::get_effective_permissions(&claims, &state).await?;
    if !crate::server::authz_middleware::enforce_raw(
        &permissions,
        crate::server::PermissionAction::Write,
        Some(&db_name),
        claims.scoped_databases.as_deref(),
        &claims.sub,
    ) {
        return Err(DbError::Forbidden(format!(
            "Write permission on '{}' is required to resolve this conflict",
            db_name
        )));
    }

    let change: SyncChange = serde_json::from_value(row.get("change").cloned().unwrap_or_default())
        .map_err(|e| DbError::InternalError(format!("stored conflict is unreadable: {}", e)))?;
    let device = row
        .get("device")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let conflict_id = row
        .get("_key")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let actor = crate::server::handlers::query::write_actor_from_claims(Some(&claims));

    // What to write, if anything.
    let to_apply = match resolution.as_str() {
        "local" => None,
        "remote" => Some(change.clone()),
        _ => Some(SyncChange {
            operation: ChangeOperation::Update,
            document_data: merged_data,
            is_delta: false,
            delta_patch: None,
            ..change.clone()
        }),
    };
    let db = state.storage.get_database(&db_name)?;
    let seq = match &to_apply {
        Some(applied) => {
            apply_sync_change(&state, applied, actor, false)?;
            let seq = log_sync_change(&state, applied);
            conflict_store::record_write(&db, applied, seq, &device)?;
            seq
        }
        None => {
            // Keeping the server's copy: note that the client has now settled
            // against everything up to here, so its next push is not a conflict.
            let seq = state
                .replication_log
                .as_ref()
                .map_or(0, |l| l.current_sequence());
            let kept = SyncChange {
                operation: ChangeOperation::Update,
                ..change.clone()
            };
            // The document may be gone; there is then nothing to record.
            let _ = conflict_store::record_write(&db, &kept, seq, &device);
            seq
        }
    };
    conflict_store::remove_conflict(&db, &conflict_id)?;

    Ok(Json(serde_json::json!({
        "success": true,
        "document_key": document_key,
        "resolution": resolution,
        "conflict_id": conflict_id,
        "sequence": seq,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pull_limit_is_clamped() {
        assert_eq!(clamp_pull_limit(None), DEFAULT_PULL_LIMIT);
        assert_eq!(clamp_pull_limit(Some(0)), 1);
        assert_eq!(clamp_pull_limit(Some(50)), 50);
        assert_eq!(clamp_pull_limit(Some(1_000_000_000)), MAX_PULL_LIMIT);
        assert_eq!(clamp_pull_limit(Some(u64::MAX)), MAX_PULL_LIMIT);
    }

    #[test]
    fn credential_collections_are_not_servable() {
        let entry = |coll: &str| LogEntry::new_op("_system", coll, Operation::Insert, "k", None);
        for coll in ["_admins", "_api_keys", "_env", "_roles", "_user_roles"] {
            assert!(!servable_to_sync_client(&entry(coll)), "{}", coll);
        }
        assert!(servable_to_sync_client(&entry("orders")));
    }
}
