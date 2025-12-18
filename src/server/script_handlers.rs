//! HTTP handlers for Lua script management and execution

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Json},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

use super::handlers::AppState;
use crate::error::DbError;
use crate::scripting::{Script, ScriptContext, ScriptEngine};
use crate::sync::{Operation, LogEntry};

/// System collection for storing scripts
pub const SCRIPTS_COLLECTION: &str = "_scripts";

// ==================== Request/Response Types ====================

#[derive(Debug, Deserialize)]
pub struct CreateScriptRequest {
    /// Human-readable name for the script
    pub name: String,
    /// URL path pattern (e.g., "hello" or "users/:id")
    pub path: String,
    /// HTTP methods this script handles (e.g., ["GET", "POST"])
    pub methods: Vec<String>,
    /// The Lua source code
    pub code: String,
    /// Optional description
    pub description: Option<String>,
    /// Target collection (optional)
    pub collection: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateScriptResponse {
    pub id: String,
    pub name: String,
    pub path: String,
    pub methods: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ListScriptsResponse {
    pub scripts: Vec<ScriptSummary>,
}

#[derive(Debug, Serialize)]
pub struct ScriptSummary {
    pub id: String,
    pub name: String,
    pub path: String,
    pub methods: Vec<String>,
    pub description: Option<String>,
    pub database: String,
    pub collection: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteScriptResponse {
    pub deleted: bool,
}

// ==================== Script Management Handlers ====================

/// Create a new Lua script
pub async fn create_script_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<CreateScriptRequest>,
) -> Result<Json<CreateScriptResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Ensure _scripts collection exists
    if db.get_collection(SCRIPTS_COLLECTION).is_err() {
        db.create_collection(SCRIPTS_COLLECTION.to_string(), None)?;
    }

    let collection = db.get_collection(SCRIPTS_COLLECTION)?;

    // Generate unique ID based on db/collection/path
    let id = if let Some(col) = &req.collection {
        format!("{}_{}_{}", db_name, col, sanitize_path_to_key(&req.path))
    } else {
        format!("{}_{}", db_name, sanitize_path_to_key(&req.path))
    };
    
    let now = chrono::Utc::now().to_rfc3339();

    // Check if script with same path already exists
    if collection.get(&id).is_ok() {
        return Err(DbError::BadRequest(format!(
            "Script with path '{}' already exists in this scope",
            req.path
        )));
    }

    let script = Script {
        key: id.clone(),
        name: req.name.clone(),
        methods: req.methods.clone(),
        path: req.path.clone(),
        database: db_name.clone(),
        collection: req.collection.clone(),
        code: req.code,
        description: req.description,
        created_at: now.clone(),
        updated_at: now.clone(),
    };

    let doc_value = serde_json::to_value(&script)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;

    collection.insert(doc_value.clone())?;

    tracing::info!("Lua script '{}' created for path '{}' in db '{}'", req.name, req.path, db_name);

    // Record write for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(), // Auto-filled
            database: db_name.clone(),
            collection: SCRIPTS_COLLECTION.to_string(),
            operation: Operation::Insert,
            key: id.clone(),
            data: serde_json::to_vec(&doc_value).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    Ok(Json(CreateScriptResponse {
        id,
        name: req.name,
        path: req.path,
        methods: req.methods,
        created_at: now,
    }))
}

/// List scripts for a specific database
pub async fn list_scripts_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
) -> Result<Json<ListScriptsResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Return empty if collection doesn't exist
    let collection = match db.get_collection(SCRIPTS_COLLECTION) {
        Ok(c) => c,
        Err(DbError::CollectionNotFound(_)) => {
            return Ok(Json(ListScriptsResponse { scripts: vec![] }));
        }
        Err(e) => return Err(e),
    };

    let mut scripts = Vec::new();
    for doc in collection.scan(None) {
        let script: Script = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted script data".to_string()))?;

        // Filter by database
        if script.database == db_name {
            scripts.push(ScriptSummary {
                id: script.key,
                name: script.name,
                path: script.path,
                methods: script.methods,
                description: script.description,
                database: script.database,
                collection: script.collection,
                created_at: script.created_at,
                updated_at: script.updated_at,
            });
        }
    }

    Ok(Json(ListScriptsResponse { scripts }))
}

/// Get a specific script
pub async fn get_script_handler(
    State(state): State<AppState>,
    Path((db_name, script_id)): Path<(String, String)>,
) -> Result<Json<Script>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let collection = db.get_collection(SCRIPTS_COLLECTION)?;

    let doc = collection.get(&script_id)?;
    let script: Script = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted script data".to_string()))?;

    Ok(Json(script))
}

/// Update a script
pub async fn update_script_handler(
    State(state): State<AppState>,
    Path((db_name, script_id)): Path<(String, String)>,
    Json(req): Json<CreateScriptRequest>,
) -> Result<Json<Script>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let collection = db.get_collection(SCRIPTS_COLLECTION)?;

    // Get existing script to preserve sensitive fields
    let existing_doc = collection.get(&script_id)?;
    let existing: Script = serde_json::from_value(existing_doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted script data".to_string()))?;

    // We don't allow changing database or collection effectively changing ID logic
    // So we persist existing database/collection
    let script = Script {
        key: script_id.clone(),
        name: req.name,
        methods: req.methods,
        path: req.path,
        database: existing.database,
        collection: existing.collection,
        code: req.code,
        description: req.description,
        created_at: existing.created_at,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };

    let doc_value = serde_json::to_value(&script)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;

    collection.update(&script_id, doc_value.clone())?;

    tracing::info!("Lua script '{}' updated", script_id);

    // Record write for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: db_name.clone(),
            collection: SCRIPTS_COLLECTION.to_string(),
            operation: Operation::Update,
            key: script_id.clone(),
            data: serde_json::to_vec(&doc_value).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    Ok(Json(script))
}

/// Delete a script
pub async fn delete_script_handler(
    State(state): State<AppState>,
    Path((db_name, script_id)): Path<(String, String)>,
) -> Result<Json<DeleteScriptResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let collection = db.get_collection(SCRIPTS_COLLECTION)?;

    collection.delete(&script_id)?;

    tracing::info!("Lua script '{}' deleted", script_id);

    // Record write for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: db_name.clone(),
            collection: SCRIPTS_COLLECTION.to_string(),
            operation: Operation::Delete,
            key: script_id.clone(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    Ok(Json(DeleteScriptResponse { deleted: true }))
}

// ==================== Script Execution Handler ====================

/// Execute a Lua script based on the URL path
pub async fn execute_script_handler(
    State(state): State<AppState>,
    method: axum::http::Method,
    axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
    headers: axum::http::HeaderMap,
    body: Option<Json<Value>>,
) -> Result<impl IntoResponse, DbError> {
    // Extract the path after /api/custom/:db/:collection/
    let uri_path = uri.path().to_string();
    let prefix = "/api/custom/";
    let remaining = uri_path.strip_prefix(prefix).unwrap_or(&uri_path);
    
    // Parse db/path
    let parts: Vec<&str> = remaining.splitn(2, '/').collect();
    if parts.len() < 2 {
        return Err(DbError::BadRequest("Invalid custom API path. Expected /api/custom/:db/:path".to_string()));
    }
    
    let db_name = parts[0];
    let script_path = parts[1];
    
    // Find matching script
    let script = find_script_for_scoped_path(&state, db_name, script_path, method.as_str())?;

    // Build context
    let query_params: HashMap<String, String> = uri
        .query()
        .map(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .into_owned()
                .collect()
        })
        .unwrap_or_default();

    let headers_map: HashMap<String, String> = headers
        .iter()
        .filter_map(|(k, v)| {
            v.to_str()
                .ok()
                .map(|v| (k.as_str().to_string(), v.to_string()))
        })
        .collect();

    let context = ScriptContext {
        method: method.to_string(),
        path: script_path.to_string(),
        query_params,
        params: extract_path_params(&script.path, script_path),
        headers: headers_map,
        body: body.map(|b| b.0),
    };

    // Execute script
    let engine = ScriptEngine::new(state.storage.clone());
    
    // Auto-select DB in Lua context using the path's db_name
    let result = engine.execute(&script, db_name, &context).await?;

    Ok((StatusCode::from_u16(result.status).unwrap_or(StatusCode::OK), Json(result.body)))
}

// ==================== Helper Functions ====================

/// Convert a URL path to a valid document key
fn sanitize_path_to_key(path: &str) -> String {
    path.replace('/', "_")
        .replace(':', "_")
        .replace('*', "_")
        .trim_matches('_')
        .to_string()
}

/// Find a script that matches the given path and method within a scope
fn find_script_for_scoped_path(
    state: &AppState,
    db_name: &str,
    path: &str,
    method: &str,
) -> Result<Script, DbError> {
    let db = state.storage.get_database(db_name)?;
    let collection = db.get_collection(SCRIPTS_COLLECTION)?;

    // Scan is inefficient but works for MVP. Indexing usage would be better.
    for doc in collection.scan(None) {
        let script: Script = match serde_json::from_value(doc.to_value()) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Check scope
        if script.database != db_name {
            continue;
        }

        // Check if method matches
        if !script
            .methods
            .iter()
            .any(|m| m.eq_ignore_ascii_case(method))
        {
            continue;
        }

        // Check if path matches (simple matching for now)
        if path_matches(&script.path, path) {
            return Ok(script);
        }
    }

    Err(DbError::DocumentNotFound(format!(
        "No script found for {} {} in {}",
        method, path, db_name
    )))
}

/// Check if a script path pattern matches the actual path
fn path_matches(pattern: &str, path: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();

    if pattern_parts.len() != path_parts.len() {
        return false;
    }

    for (p, actual) in pattern_parts.iter().zip(path_parts.iter()) {
        if p.starts_with(':') {
            // Parameter - matches anything
            continue;
        }
        if *p != *actual {
            return false;
        }
    }

    true
}

/// Extract parameters from the path based on the pattern
fn extract_path_params(pattern: &str, path: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    let pattern_parts: Vec<&str> = pattern.split('/').collect();
    let path_parts: Vec<&str> = path.split('/').collect();

    if pattern_parts.len() != path_parts.len() {
        return params;
    }

    for (p, actual) in pattern_parts.iter().zip(path_parts.iter()) {
        if let Some(name) = p.strip_prefix(':') {
            params.insert(name.to_string(), actual.to_string());
        }
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_matches() {
        assert!(path_matches("hello", "hello"));
        assert!(path_matches("users/:id", "users/123"));
        assert!(path_matches("api/v1/:resource", "api/v1/posts"));
        assert!(!path_matches("hello", "world"));
        assert!(!path_matches("users/:id", "users/123/posts"));
    }

    #[test]
    fn test_extract_params() {
        let params = extract_path_params("users/:id", "users/123");
        assert_eq!(params.get("id").unwrap(), "123");

        let params = extract_path_params("posts/:id/comments/:cid", "posts/10/comments/5");
        assert_eq!(params.get("id").unwrap(), "10");
        assert_eq!(params.get("cid").unwrap(), "5");
    }

    #[test]
    fn test_sanitize_path() {
        assert_eq!(sanitize_path_to_key("hello"), "hello");
        assert_eq!(sanitize_path_to_key("users/:id"), "users__id");
        assert_eq!(sanitize_path_to_key("/api/test"), "api_test");
    }
}
