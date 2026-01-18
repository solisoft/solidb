use super::system::AppState;
use crate::error::DbError;
use crate::sync::{LogEntry, Operation};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateDatabaseRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct CreateDatabaseResponse {
    pub name: String,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ListDatabasesResponse {
    pub databases: Vec<String>,
}

pub async fn create_database(
    State(state): State<AppState>,
    Json(req): Json<CreateDatabaseRequest>,
) -> Result<Json<CreateDatabaseResponse>, DbError> {
    state.storage.create_database(req.name.clone())?;

    // Record to replication log
    // Record to replication log
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: req.name.clone(),
            collection: "".to_string(),
            operation: Operation::CreateDatabase,
            key: "".to_string(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    // Auto-create _scripts collection for the new database
    if let Ok(db) = state.storage.get_database(&req.name) {
        if db.create_collection("_scripts".to_string(), None).is_ok() {
            // Record _scripts creation to replication log
            if let Some(ref log) = state.replication_log {
                let metadata = serde_json::json!({
                    "type": "document",
                    "shardConfig": None::<serde_json::Value>
                });

                let entry = LogEntry {
                    sequence: 0,
                    node_id: "".to_string(),
                    database: req.name.clone(),
                    collection: "_scripts".to_string(),
                    operation: Operation::CreateCollection,
                    key: "".to_string(),
                    data: serde_json::to_vec(&metadata).ok(),
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                    origin_sequence: None,
                };
                let _ = log.append(entry);
            }
        }
    }

    Ok(Json(CreateDatabaseResponse {
        name: req.name,
        status: "created".to_string(),
    }))
}

pub async fn list_databases(State(state): State<AppState>) -> Json<ListDatabasesResponse> {
    let databases = state.storage.list_databases();
    Json(ListDatabasesResponse { databases })
}

pub async fn delete_database(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<StatusCode, DbError> {
    state.storage.delete_database(&name)?;

    // Record to replication log
    // Record to replication log
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: name.clone(),
            collection: "".to_string(),
            operation: Operation::DeleteDatabase,
            key: "".to_string(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    Ok(StatusCode::NO_CONTENT)
}
