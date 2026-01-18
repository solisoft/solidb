use super::super::system::{is_protected_collection, AppState};
use crate::{
    error::DbError,
    sync::{LogEntry, Operation},
};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Json,
};
use serde::Deserialize;
use serde_json::Value;

// ==================== Structs ====================

#[derive(Debug, Deserialize)]
pub struct PruneRequest {
    pub older_than: String,
}

// ==================== Handlers ====================

pub async fn truncate_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, DbError> {
    // Protect system collections
    if is_protected_collection(&db_name, &coll_name) {
        return Err(DbError::BadRequest(format!(
            "Cannot truncate protected system collection: {}",
            coll_name
        )));
    }

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check if this is a direct shard truncate request (internal)
    let is_shard_direct = headers.contains_key("X-Shard-Direct");

    // Save shard config before truncating (truncate may clear it)
    let saved_shard_config = collection.get_shard_config();

    // For sharded collections, also truncate all physical shards on this node
    let mut total_count = 0usize;
    if let Some(ref shard_config) = saved_shard_config {
        if shard_config.num_shards > 0 {
            // Get nodes for remote truncation (only for non-direct requests)
            let remote_nodes: Vec<(String, String)> = if !is_shard_direct {
                if let Some(ref mgr) = state.cluster_manager {
                    let my_id = mgr.local_node_id();
                    mgr.state()
                        .get_all_members()
                        .into_iter()
                        .filter(|m| m.node.id != my_id)
                        .map(|m| (m.node.id.clone(), m.node.api_address.clone()))
                        .collect()
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            // Truncate physical shards locally
            for shard_id in 0..shard_config.num_shards {
                let physical_name = format!("{}_s{}", coll_name, shard_id);
                if let Ok(shard_coll) = database.get_collection(&physical_name) {
                    let c = shard_coll.clone();
                    if let Ok(count) = tokio::task::spawn_blocking(move || c.truncate())
                        .await
                        .map_err(|e| DbError::InternalError(format!("Task error: {}", e)))?
                    {
                        total_count += count;
                    }
                }
            }

            // Truncate physical shards on remote nodes
            if !remote_nodes.is_empty() {
                let client = reqwest::Client::new();
                let secret = state.cluster_secret();
                let auth_header = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                for shard_id in 0..shard_config.num_shards {
                    let physical_name = format!("{}_s{}", coll_name, shard_id);

                    for (_node_id, addr) in &remote_nodes {
                        let url = format!(
                            "http://{}/_api/database/{}/collection/{}/truncate",
                            addr, db_name, physical_name
                        );
                        let mut req = client
                            .put(&url)
                            .header("X-Shard-Direct", "true")
                            .header("X-Cluster-Secret", &secret)
                            .timeout(std::time::Duration::from_secs(10));

                        if !auth_header.is_empty() {
                            req = req.header("Authorization", &auth_header);
                        }

                        let _ = req.send().await;
                    }
                }
            }
        }
    }

    // Also truncate the logical collection (may have some data or metadata)
    let coll = collection.clone();
    let count = tokio::task::spawn_blocking(move || coll.truncate())
        .await
        .map_err(|e| DbError::InternalError(format!("Task error: {}", e)))??;
    total_count += count;

    // Restore shard config after truncating
    if let Some(config) = saved_shard_config.clone() {
        let _ = collection.set_shard_config(&config);
    }

    // Record to replication log (only for non-direct requests to avoid duplicate logging)
    if !is_shard_direct {
        if let Some(ref log) = state.replication_log {
            let entry = LogEntry {
                sequence: 0,
                node_id: "".to_string(),
                database: db_name.clone(),
                collection: coll_name.clone(),
                operation: Operation::TruncateCollection,
                key: "".to_string(),
                data: None,
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            };
            let _ = log.append(entry);
        }
    }

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "deleted": total_count,
        "status": "truncated"
    })))
}

pub async fn compact_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    collection.compact();

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "status": "compacted"
    })))
}

/// Repair sharded collection by removing misplaced documents
pub async fn repair_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    if let Some(coordinator) = state.shard_coordinator {
        let report = coordinator
            .repair_collection(&db_name, &coll_name)
            .await
            .map_err(DbError::InternalError)?;

        Ok(Json(serde_json::json!({
            "status": "repaired",
            "report": report
        })))
    } else {
        Err(DbError::InternalError(
            "Shard coordinator not available".to_string(),
        ))
    }
}

pub async fn prune_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(payload): Json<PruneRequest>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Parse timestamp
    let dt = chrono::DateTime::parse_from_rfc3339(&payload.older_than).map_err(|_| {
        DbError::BadRequest("Invalid timestamp format (ISO8601 required)".to_string())
    })?;

    let timestamp_ms = dt.timestamp_millis();
    if timestamp_ms < 0 {
        return Err(DbError::BadRequest(
            "Timestamp cannot be negative".to_string(),
        ));
    }

    let count = collection.prune_older_than(timestamp_ms as u64)?;

    Ok(Json(serde_json::json!({ "deleted": count })))
}
