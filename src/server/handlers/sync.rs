use super::system::AppState;
use crate::error::DbError;
use crate::sync::{
    protocol::Operation,
    session::{ChangeOperation, SyncChange, SyncSession},
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
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, DbError> {
    // Parse request fields from JSON
    let device_id = req
        .get("device_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("device_id is required".to_string()))?
        .to_string();

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

    let filter_query = req
        .get("filter_query")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

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

    // Store session in manager
    let session_manager = state.sync_session_manager.as_ref().ok_or_else(|| {
        DbError::InternalError("Sync session manager not initialized".to_string())
    })?;

    session_manager.register_session(session).await;

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
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, DbError> {
    let session_id = req
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("session_id is required".to_string()))?
        .to_string();

    // Verify session exists
    let session_manager = state.sync_session_manager.as_ref().ok_or_else(|| {
        DbError::InternalError("Sync session manager not initialized".to_string())
    })?;

    let session = session_manager
        .get_session(&session_id)
        .await
        .ok_or_else(|| DbError::BadRequest(format!("Session '{}' not found", session_id)))?;

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

    let limit = req
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(100);

    // Get session's subscriptions and last sequence
    let subscriptions = &session.subscriptions;
    let after_sequence = session.last_sequence;

    // Query sync log for entries after client's sequence
    let sync_log = state
        .replication_log
        .as_ref()
        .ok_or_else(|| DbError::InternalError("Replication log not initialized".to_string()))?;

    let log_entries = sync_log.get_entries_after(after_sequence, limit);

    // Filter by subscriptions (if any subscriptions are specified)
    let filtered: Vec<_> = log_entries
        .into_iter()
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

    // Convert LogEntry -> SyncChange
    let changes: Vec<SyncChange> = filtered.iter().map(log_entry_to_sync_change).collect();

    // Build response
    let has_more = changes.len() == limit;
    let max_seq = filtered
        .iter()
        .map(|e| e.sequence)
        .max()
        .unwrap_or(after_sequence);

    // Build server vector from the latest entries
    let mut server_vector = VersionVector::new();
    for entry in &filtered {
        let current = server_vector.get(&entry.node_id);
        if entry.sequence > current {
            server_vector.increment(&entry.node_id);
            // Set to actual sequence value
            while server_vector.get(&entry.node_id) < entry.sequence {
                server_vector.increment(&entry.node_id);
            }
        }
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

/// POST /_api/sync/push
/// Push changes from client to server
pub async fn push_changes(
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, DbError> {
    let session_id = req
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("session_id is required".to_string()))?
        .to_string();

    // Verify session exists
    let session_manager = state.sync_session_manager.as_ref().ok_or_else(|| {
        DbError::InternalError("Sync session manager not initialized".to_string())
    })?;

    let session = session_manager
        .get_session(&session_id)
        .await
        .ok_or_else(|| DbError::BadRequest(format!("Session '{}' not found", session_id)))?;

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

    let conflicts: Vec<serde_json::Value> = Vec::new();
    let mut accepted = 0;
    let rejected = 0;

    // Process each change
    for change in &changes {
        // TODO: Apply change to database and detect conflicts
        // For now, simulate acceptance
        accepted += 1;

        // Log to replication log if available
        if let Some(ref log) = state.replication_log {
            let operation = match change.operation {
                ChangeOperation::Insert => Operation::Insert,
                ChangeOperation::Update => Operation::Update,
                ChangeOperation::Delete => Operation::Delete,
            };

            let data_bytes = change
                .document_data
                .as_ref()
                .and_then(|d| serde_json::to_vec(d).ok());

            let entry = LogEntry {
                sequence: 0, // Auto-generated by log
                node_id: session.device_id.clone(),
                database: change.database.clone(),
                collection: change.collection.clone(),
                operation,
                key: change.document_key.clone(),
                data: data_bytes,
                timestamp: change.timestamp,
                origin_sequence: None,
            };

            let _ = log.append(entry);
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

    // Verify session exists
    let session_manager = state.sync_session_manager.as_ref().ok_or_else(|| {
        DbError::InternalError("Sync session manager not initialized".to_string())
    })?;

    let _session = session_manager
        .get_session(&session_id)
        .await
        .ok_or_else(|| DbError::BadRequest(format!("Session '{}' not found", session_id)))?;

    // Update session vector to reflect acknowledged state
    session_manager
        .update_session_vector(&session_id, &applied_vector)
        .await;

    Ok(Json(serde_json::json!({
        "success": true,
    })))
}

/// GET /_api/sync/conflicts
/// List unresolved conflicts for a session
pub async fn list_conflicts(
    State(state): State<AppState>,
    Query(params): Query<ConflictsQuery>,
) -> Result<Json<serde_json::Value>, DbError> {
    // Verify session exists
    let session_manager = state.sync_session_manager.as_ref().ok_or_else(|| {
        DbError::InternalError("Sync session manager not initialized".to_string())
    })?;

    let _session = session_manager
        .get_session(&params.session_id)
        .await
        .ok_or_else(|| DbError::BadRequest(format!("Session '{}' not found", params.session_id)))?;

    // TODO: Query conflict store for unresolved conflicts
    // For now, return empty list
    let conflicts: Vec<serde_json::Value> = vec![];

    Ok(Json(serde_json::json!({
        "conflicts": conflicts,
    })))
}

/// POST /_api/sync/resolve
/// Resolve a conflict manually
pub async fn resolve_conflict(
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, DbError> {
    let session_id = req
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("session_id is required".to_string()))?
        .to_string();

    let document_key = req
        .get("document_key")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("document_key is required".to_string()))?
        .to_string();

    let resolution = req
        .get("resolution")
        .and_then(|v| v.as_str())
        .ok_or_else(|| DbError::BadRequest("resolution is required".to_string()))?
        .to_string();

    let merged_data = req.get("merged_data").cloned();

    // Verify session exists
    let session_manager = state.sync_session_manager.as_ref().ok_or_else(|| {
        DbError::InternalError("Sync session manager not initialized".to_string())
    })?;

    let _session = session_manager
        .get_session(&session_id)
        .await
        .ok_or_else(|| DbError::BadRequest(format!("Session '{}' not found", session_id)))?;

    // Validate resolution value
    if !matches!(resolution.as_str(), "local" | "remote" | "merged") {
        return Err(DbError::BadRequest(
            "resolution must be 'local', 'remote', or 'merged'".to_string(),
        ));
    }

    // TODO: Apply resolution to conflict store
    // - "local": Keep server version
    // - "remote": Accept client version
    // - "merged": Apply merged data

    tracing::info!(
        "Resolving conflict for document {} with resolution {}",
        document_key,
        resolution
    );

    if resolution == "merged" && merged_data.is_none() {
        return Err(DbError::BadRequest(
            "merged_data is required when resolution is 'merged'".to_string(),
        ));
    }

    Ok(Json(serde_json::json!({
        "success": true,
        "document_key": document_key,
        "resolution": resolution,
    })))
}
