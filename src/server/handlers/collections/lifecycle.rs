use super::super::system::{is_protected_collection, AppState};
use crate::{
    error::DbError,
    storage::http_client::get_http_client,
    sync::{LogEntry, Operation},
};
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::{Deserialize, Serialize};

// ==================== Structs ====================

#[derive(Debug, Deserialize)]
pub struct CreateCollectionRequest {
    pub name: String,
    /// Collection type: "document" (default), "edge", or "blob"
    #[serde(rename = "type")]
    pub collection_type: Option<String>,
    /// Number of shards (optional - if not set, collection is not sharded)
    #[serde(rename = "numShards")]
    pub num_shards: Option<u16>,
    /// Field to use for sharding key (default: "_key")
    #[serde(rename = "shardKey")]
    pub shard_key: Option<String>,
    /// Replication factor (optional, default: 1 = no replicas)
    #[serde(rename = "replicationFactor")]
    pub replication_factor: Option<u16>,
    /// JSON Schema for validation (optional)
    #[serde(rename = "schema")]
    pub schema: Option<serde_json::Value>,
    /// Validation mode: "off", "strict", or "lenient"
    #[serde(rename = "validationMode", default = "default_validation_mode")]
    pub validation_mode: String,
}

fn default_validation_mode() -> String {
    "off".to_string()
}

#[derive(Debug, Serialize)]
pub struct CreateCollectionResponse {
    pub name: String,
    pub status: String,
    /// Number of shards (if sharded)
    #[serde(rename = "numShards", skip_serializing_if = "Option::is_none")]
    pub num_shards: Option<u16>,
    /// Shard key field (if sharded)
    #[serde(rename = "shardKey", skip_serializing_if = "Option::is_none")]
    pub shard_key: Option<String>,
    /// Replication factor (if sharded)
    #[serde(rename = "replicationFactor", skip_serializing_if = "Option::is_none")]
    pub replication_factor: Option<u16>,
}

// ==================== Handlers ====================

pub async fn create_collection(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<CreateCollectionRequest>,
) -> Result<Json<CreateCollectionResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    database.create_collection(req.name.clone(), req.collection_type.clone())?;

    let collection = database.get_collection(&req.name)?;

    // Parse validation mode
    let validation_mode = match req.validation_mode.to_lowercase().as_str() {
        "strict" => crate::storage::schema::SchemaValidationMode::Strict,
        "lenient" => crate::storage::schema::SchemaValidationMode::Lenient,
        _ => crate::storage::schema::SchemaValidationMode::Off,
    };

    // Set schema if provided
    if let Some(schema) = req.schema {
        collection.set_json_schema(crate::storage::schema::CollectionSchema::new(
            "default".to_string(),
            schema,
            validation_mode,
        ))?;
    }

    // Store sharding configuration if specified
    // Auto-configure sharding for blob collections OR use explicitly provided config
    let shard_config = if let Some(num_shards) = req.num_shards {
        // Explicit sharding configuration provided
        Some(crate::sharding::coordinator::CollectionShardConfig {
            num_shards,
            shard_key: req.shard_key.clone().unwrap_or_else(|| "_key".to_string()),
            replication_factor: req.replication_factor.unwrap_or(1),
        })
    } else if req.collection_type.as_deref() == Some("blob") {
        // Blob collections are NOT auto-sharded by default - users can explicitly shard them if needed
        // Chunks will be distributed across cluster for fault tolerance
        tracing::info!(
            "Blob collection {} will use cluster-wide chunk distribution",
            req.name
        );
        None
    } else {
        None
    };

    // Apply sharding configuration if present
    if let Some(config) = shard_config {
        // Store shard config in collection metadata
        collection.set_shard_config(&config)?;

        // Initialize sharding via coordinator if available
        if let Some(ref coordinator) = state.shard_coordinator {
            tracing::info!(
                "Initializing sharding for {}.{}: {:?}",
                db_name,
                req.name,
                config
            );

            // 1. Compute assignments (in-memory)
            coordinator
                .init_collection(&db_name, &req.name, &config)
                .map_err(|e| DbError::InternalError(format!("Failed to init sharding: {}", e)))?;

            // 2. Create physical shards (distributed)
            coordinator
                .create_shards(&db_name, &req.name)
                .await
                .map_err(|e| DbError::InternalError(format!("Failed to create shards: {}", e)))?;
        }
    }

    // Set persistence type if blob
    if let Some(ctype) = &req.collection_type {
        if ctype == "blob" {
            collection.set_type("blob")?;
        }
    }

    // Record to replication log
    if let Some(ref log) = state.replication_log {
        let metadata = serde_json::json!({
            "type": req.collection_type.clone().unwrap_or_else(|| "document".to_string()),
            "shardConfig": if let Some(num_shards) = req.num_shards {
                Some(serde_json::json!({
                    "num_shards": num_shards,
                    "shard_key": req.shard_key.clone().unwrap_or_else(|| "_key".to_string()),
                    "replication_factor": req.replication_factor.unwrap_or(1)
                }))
            } else {
                None::<serde_json::Value>
            }
        });

        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: db_name.clone(),
            collection: req.name.clone(),
            operation: Operation::CreateCollection,
            key: "".to_string(),
            data: serde_json::to_vec(&metadata).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    Ok(Json(CreateCollectionResponse {
        name: req.name,
        status: "created".to_string(),
        num_shards: req.num_shards,
        shard_key: req.shard_key,
        replication_factor: req.replication_factor,
    }))
}

pub async fn delete_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, DbError> {
    // Protect system collections
    if is_protected_collection(&db_name, &coll_name) {
        return Err(DbError::BadRequest(format!(
            "Cannot delete protected system collection: {}",
            coll_name
        )));
    }

    let database = state.storage.get_database(&db_name)?;

    // Check if this is a direct shard delete request (internal)
    let is_shard_direct = headers.contains_key("X-Shard-Direct");

    // For sharded collections, delete all physical shards (local and remote)
    if let Ok(collection) = database.get_collection(&coll_name) {
        if let Some(shard_config) = collection.get_shard_config() {
            if shard_config.num_shards > 0 && !is_shard_direct {
                // Get nodes for remote deletion
                let remote_nodes: Vec<(String, String)> =
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
                    };

                // Delete physical shards locally
                for shard_id in 0..shard_config.num_shards {
                    let physical_name = format!("{}_s{}", coll_name, shard_id);
                    let _ = database.delete_collection(&physical_name);
                }

                // Delete physical shards on remote nodes
                if !remote_nodes.is_empty() {
                    let client = get_http_client();
                    let secret = state.cluster_secret();

                    for shard_id in 0..shard_config.num_shards {
                        let physical_name = format!("{}_s{}", coll_name, shard_id);

                        for (_node_id, addr) in &remote_nodes {
                            let url = format!(
                                "http://{}/_api/database/{}/collection/{}",
                                addr, db_name, physical_name
                            );
                            let _ = client
                                .delete(&url)
                                .header("X-Shard-Direct", "true")
                                .header("X-Cluster-Secret", &secret)
                                .timeout(std::time::Duration::from_secs(10))
                                .send()
                                .await;
                        }
                    }
                }
            }
        }
    }

    // Delete the logical collection
    database.delete_collection(&coll_name)?;

    // Record to replication log
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(), // Log assigns it
            database: db_name.clone(),
            collection: coll_name.clone(),
            operation: Operation::DeleteCollection,
            key: "".to_string(),
            data: None,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    Ok(StatusCode::NO_CONTENT)
}
