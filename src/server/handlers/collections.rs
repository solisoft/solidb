use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use crate::{
    error::DbError,
    sync::{LogEntry, Operation},
};
use super::system::{is_protected_collection, AppState};

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

#[derive(Debug, Deserialize)]
pub struct PruneRequest {
    pub older_than: String,
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

#[derive(Debug, Serialize)]
pub struct CollectionSummary {
    pub name: String,
    pub count: usize,
    #[serde(rename = "localCount", skip_serializing_if = "Option::is_none")]
    pub local_count: Option<usize>,
    #[serde(rename = "type")]
    pub collection_type: String,
    #[serde(rename = "shardConfig", skip_serializing_if = "Option::is_none")]
    pub shard_config: Option<crate::sharding::coordinator::CollectionShardConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<crate::storage::CollectionStats>,
}

#[derive(Debug, Serialize)]
pub struct ListCollectionsResponse {
    pub collections: Vec<CollectionSummary>,
}

#[derive(Debug, Deserialize, serde::Serialize, Clone)]
pub struct UpdateCollectionPropertiesRequest {
    /// Collection type: "document", "edge", or "blob"
    #[serde(rename = "type")]
    pub type_: Option<String>,
    /// Number of shards (updating this triggers rebalance)
    #[serde(rename = "numShards", alias = "num_shards")]
    pub num_shards: Option<u16>,
    /// Replication factor (optional, default: 1 = no replicas)
    #[serde(rename = "replicationFactor", alias = "replication_factor")]
    pub replication_factor: Option<u16>,
    /// Whether to propagate this update to other nodes (default: true)
    #[serde(default)]
    pub propagate: Option<bool>,
    /// JSON Schema for validation (optional)
    #[serde(rename = "schema")]
    pub schema: Option<serde_json::Value>,
    /// Validation mode: "off", "strict", or "lenient"
    #[serde(rename = "validationMode")]
    pub validation_mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CollectionPropertiesResponse {
    pub name: String,
    pub status: String,
    #[serde(rename = "shardConfig")]
    pub shard_config: crate::sharding::coordinator::CollectionShardConfig,
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

pub async fn list_collections(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    headers: HeaderMap,
) -> Result<Json<ListCollectionsResponse>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let names = database.list_collections();

    // Get auth token from request headers to forward to remote nodes
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let mut collections = Vec::new();
    let local_id = state
        .cluster_manager
        .as_ref()
        .map(|m| m.local_node_id())
        .unwrap_or_else(|| "local".to_string());

    for name in names {
        // Skip internal physical shards (ending with _s{id})
        let has_shard_suffix = name.rfind("_s").map(|i| {
            // Check if what follows _s is a number
            name[i + 2..].chars().all(|c| c.is_digit(10))
        }).unwrap_or(false);

        if !is_protected_collection(&db_name, &name) && !has_shard_suffix {
            let coll = database.get_collection(&name)?;
            let shard_config = coll.get_shard_config();

            // Calculate total count (cluster-wide if sharded)
            let (count, local_count, shard_table) = if let Some(ref config) = shard_config {
                if config.num_shards > 0 {
                    let total = if let Some(ref coordinator) = state.shard_coordinator {
                        // Use coordinator to get aggregated count from all shards
                        // Pass auth header to authenticate internal requests
                        coordinator
                            .get_total_count(&db_name, &name, Some(auth_header.clone()))
                            .await
                            .ok()
                    } else {
                        None
                    }
                    .unwrap_or_else(|| coll.count());

                    // For sharded collections, local count is sum of local physical shards
                    // We need to check which shards are assigned to this node
                    let table = state
                        .shard_coordinator
                        .as_ref()
                        .and_then(|c| c.get_shard_table(&db_name, &name));

                    let local = if let Some(ref table) = table {
                        let mut sum = 0;
                        for (shard_id, assignment) in &table.assignments {
                            if assignment.primary_node == local_id
                                || assignment.replica_nodes.contains(&local_id)
                            {
                                let physical_name = format!("{}_s{}", name, shard_id);
                                if let Ok(shard_coll) = database.get_collection(&physical_name) {
                                    sum += shard_coll.count();
                                }
                            }
                        }
                        sum
                    } else {
                        coll.count()
                    };

                    (total, Some(local), table)
                } else {
                    (coll.count(), None, None)
                }
            } else {
                (coll.count(), None, None)
            };

            let collection_type = coll.get_type();

            // Get stats - if sharded, aggregate from shards
            let stats = if let Some(ref config) = shard_config {
                if config.num_shards > 0 {
                    // Aggregate stats from all shards
                    let mut total_sst_files_size = 0;
                    let mut total_live_data_size = 0;
                    let mut total_num_sst_files = 0;
                    let mut total_memtable_size = 0;
                    let mut total_chunk_count = 0;

                    let client = reqwest::Client::new();
                    let secret = state.cluster_secret();

                    for shard_id in 0..config.num_shards {
                        let physical_name = format!("{}_s{}", name, shard_id);

                        // Check if we are the PRIMARY for this shard (not just replica)
                        // Only count from primaries to avoid double-counting disk usage
                        let is_primary_local = if let Some(ref table) = shard_table {
                            if let Some(assignment) = table.assignments.get(&shard_id) {
                                assignment.primary_node == local_id
                                    || assignment.primary_node == "local"
                            } else {
                                false
                            }
                        } else {
                            // No shard table - check if collection exists locally
                            database.get_collection(&physical_name).is_ok()
                        };

                        if is_primary_local {
                            // Use local stats
                            if let Ok(shard_coll) = database.get_collection(&physical_name) {
                                let s = shard_coll.stats();
                                total_sst_files_size += s.disk_usage.sst_files_size;
                                total_live_data_size += s.disk_usage.live_data_size;
                                total_num_sst_files += s.disk_usage.num_sst_files;
                                total_memtable_size += s.disk_usage.memtable_size;
                                total_chunk_count += s.chunk_count;
                            }
                        } else {
                            // Query remote node for stats
                            if let Some(ref table) = shard_table {
                                if let Some(assignment) = table.assignments.get(&shard_id) {
                                    if let Some(ref mgr) = state.cluster_manager {
                                        // Try primary first, then replicas
                                        let mut nodes_to_try =
                                            vec![assignment.primary_node.clone()];
                                        nodes_to_try.extend(assignment.replica_nodes.clone());

                                        for node_id in &nodes_to_try {
                                            if let Some(addr) = mgr.get_node_api_address(node_id) {
                                                let url = format!("http://{}/_api/database/{}/collection/{}/stats?local=true", addr, db_name, physical_name);

                                                let mut req = client
                                                    .get(&url)
                                                    .header("X-Cluster-Secret", &secret)
                                                    .timeout(std::time::Duration::from_secs(2));

                                                // Forward user's auth token
                                                if !auth_header.is_empty() {
                                                    req = req.header("Authorization", &auth_header);
                                                }

                                                match req.send().await {
                                                    Ok(res) if res.status().is_success() => {
                                                        if let Ok(json) =
                                                            res.json::<serde_json::Value>().await
                                                        {
                                                            total_chunk_count += json
                                                                .get("chunk_count")
                                                                .and_then(|v| v.as_u64())
                                                                .unwrap_or(0)
                                                                as usize;
                                                            if let Some(disk) =
                                                                json.get("disk_usage")
                                                            {
                                                                total_sst_files_size += disk
                                                                    .get("sst_files_size")
                                                                    .and_then(|v| v.as_u64())
                                                                    .unwrap_or(0);
                                                                total_live_data_size += disk
                                                                    .get("live_data_size")
                                                                    .and_then(|v| v.as_u64())
                                                                    .unwrap_or(0);
                                                                total_num_sst_files += disk
                                                                    .get("num_sst_files")
                                                                    .and_then(|v| v.as_u64())
                                                                    .unwrap_or(0);
                                                                total_memtable_size += disk
                                                                    .get("memtable_size")
                                                                    .and_then(|v| v.as_u64())
                                                                    .unwrap_or(0);
                                                            }
                                                        }
                                                        break; // Got stats, no need to try other nodes
                                                    }
                                                    _ => {
                                                        // This node failed, try next
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    crate::storage::CollectionStats {
                        name: name.clone(),
                        document_count: count,
                        chunk_count: total_chunk_count,
                        disk_usage: crate::storage::DiskUsage {
                            sst_files_size: total_sst_files_size,
                            live_data_size: total_live_data_size,
                            num_sst_files: total_num_sst_files,
                            memtable_size: total_memtable_size,
                        },
                    }
                } else {
                    coll.stats()
                }
            } else {
                coll.stats()
            };

            collections.push(CollectionSummary {
                name,
                count,
                local_count,
                collection_type,
                shard_config,
                stats: Some(stats),
            });
        }
    }

    // Sort by name for consistent UI
    collections.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(ListCollectionsResponse { collections }))
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
                    let client = reqwest::Client::new();
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

/// Get document count for a collection (used for cluster-wide aggregation)
pub async fn get_collection_count(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, DbError> {
    // Get auth token from request headers to forward to remote nodes
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let count = if let Some(ref coordinator) = state.shard_coordinator {
        match coordinator
            .get_total_count(&db_name, &coll_name, auth_header)
            .await
        {
            Ok(c) => c,
            Err(_) => {
                // Fallback to local count if cluster aggregation fails
                let database = state.storage.get_database(&db_name)?;
                let collection = database.get_collection(&coll_name)?;
                collection.count()
            }
        }
    } else {
        let database = state.storage.get_database(&db_name)?;
        let collection = database.get_collection(&coll_name)?;
        collection.count()
    };

    Ok(Json(serde_json::json!({
        "count": count
    })))
}

/// Recount documents from actual RocksDB data (bypasses cache)
/// Useful for debugging replication consistency
pub async fn recount_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let cached_count = collection.count();
    let actual_count = collection.recount_documents();

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "cached_count": cached_count,
        "actual_count": actual_count,
        "match": cached_count == actual_count,
        "status": "recounted"
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

pub async fn update_collection_properties(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(payload): Json<UpdateCollectionPropertiesRequest>,
) -> Result<Json<CollectionPropertiesResponse>, DbError> {
    tracing::info!(
        "update_collection_properties called: db={}, coll={}, payload={:?}",
        db_name,
        coll_name,
        payload
    );

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Update collection type if specified
    if let Some(new_type) = &payload.type_ {
        collection.set_type(new_type)?;
        tracing::info!(
            "Updated collection type for {}/{} to {}",
            db_name,
            coll_name,
            new_type
        );
    }

    // Get existing config or create new one if not sharded yet
    let mut config = collection
        .get_shard_config()
        .unwrap_or_else(|| crate::sharding::coordinator::CollectionShardConfig::default());

    tracing::info!("Current config before update: {:?}", config);

    let old_num_shards = config.num_shards;
    let mut shard_count_changed = false;

    // Get healthy node count for capping shard/replica values
    let healthy_node_count = if let Some(ref coordinator) = state.shard_coordinator {
        let count = coordinator.get_node_addresses().len();
        tracing::info!("Coordinator reports {} nodes", count);
        count
    } else {
        tracing::info!("No coordinator, using 1 node");
        1
    };

    // Update num_shards if specified
    if let Some(mut num_shards) = payload.num_shards {
        if num_shards < 1 {
            return Err(DbError::BadRequest(
                "Number of shards must be >= 1".to_string(),
            ));
        }

        // Cap num_shards to the number of healthy nodes
        tracing::info!(
            "Shard update check: requested={}, available_nodes={}",
            num_shards,
            healthy_node_count
        );

        if num_shards as usize > healthy_node_count {
            tracing::warn!(
                "Requested {} shards but only {} nodes available, capping to {}",
                num_shards,
                healthy_node_count,
                healthy_node_count
            );
            num_shards = healthy_node_count as u16;
        }

        if num_shards != config.num_shards {
            tracing::info!(
                "Updating num_shards for {}.{} from {} to {}",
                db_name,
                coll_name,
                config.num_shards,
                num_shards
            );
            config.num_shards = num_shards;
            shard_count_changed = true;
        } else {
            tracing::info!("num_shards unchanged ({})", num_shards);
        }
    } else {
        tracing::warn!("Update payload missing num_shards. Valid keys: numShards, num_shards");
    }

    // Update replication_factor if specified
    if let Some(mut rf) = payload.replication_factor {
        if rf < 1 {
            return Err(DbError::BadRequest(
                "Replication factor must be >= 1".to_string(),
            ));
        }

        // Cap replication_factor to the number of healthy nodes
        if rf as usize > healthy_node_count {
            tracing::warn!(
                "Requested replication factor {} but only {} nodes available, capping to {}",
                rf,
                healthy_node_count,
                healthy_node_count
            );
            rf = healthy_node_count as u16;
        }

        config.replication_factor = rf;
    }

    tracing::info!("Saving config: {:?}", config);

    // Save updated config
    collection.set_shard_config(&config)?;

    tracing::info!("Config saved successfully");

    // Trigger rebalance if shard count changed
    if shard_count_changed {
        if let Some(ref coordinator) = state.shard_coordinator {
            tracing::info!(
                "Shard count changed from {} to {} for {}/{}, triggering rebalance",
                old_num_shards,
                config.num_shards,
                db_name,
                coll_name
            );
            // Spawn rebalance as background task to avoid blocking the response
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                if let Err(e) = coordinator.rebalance().await {
                    tracing::error!("Failed to trigger rebalance: {}", e);
                }
            });
        }
    }

    // Broadcast metadata update to other cluster nodes to ensure consistency
    // This prevents "split brain" where only the coordinator node knows the new config
    let propagate = payload.propagate.unwrap_or(true);

    if propagate {
        if let Some(ref manager) = state.cluster_manager {
            let my_node_id = manager.local_node_id();
            let secret = state.cluster_secret();
            let client = reqwest::Client::new();

            // Clone payload and set propagate = false
            let mut forward_payload = payload.clone();
            forward_payload.propagate = Some(false);

            for member in manager.state().get_all_members() {
                if member.node.id == my_node_id {
                    continue;
                }

                let address = &member.node.api_address;
                let url = format!(
                    "http://{}/_api/database/{}/collection/{}/properties",
                    address, db_name, coll_name
                );

                tracing::info!(
                    "Propagating config update to node {} ({})",
                    member.node.id,
                    address
                );

                // Spawn background task for propagation to avoid latency
                let client = client.clone();
                let payload = forward_payload.clone();
                let secret = secret.clone();
                let url = url.clone();

                tokio::spawn(async move {
                    match client
                        .put(&url)
                        .header("X-Cluster-Secret", &secret)
                        .header("X-Shard-Direct", "true") // Bylass auth check
                        .json(&payload)
                        .send()
                        .await
                    {
                        Ok(res) => {
                            if !res.status().is_success() {
                                tracing::warn!(
                                    "Failed to propagate config to {}: {}",
                                    url,
                                    res.status()
                                );
                            } else {
                                tracing::debug!("Successfully propagated config to {}", url);
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to send propagation request to {}: {}", url, e);
                        }
                    }
                });
            }
        }
    }

    Ok(Json(CollectionPropertiesResponse {
        name: coll_name,
        status: if shard_count_changed {
            "updated_rebalancing".to_string()
        } else {
            "updated".to_string()
        },
        shard_config: config,
    }))
}

pub async fn get_collection_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((db_name, coll_name)): Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let mut stats = collection.stats();
    let collection_type = collection.get_type();

    // For sharded collections, try to get aggregated count
    if let Some(ref coordinator) = state.shard_coordinator {
        let auth_header = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        if let Ok(total) = coordinator
            .get_total_count(&db_name, &coll_name, auth_header)
            .await
        {
            stats.document_count = total;
        }
    }

    // Check if this is a local-only request (to prevent infinite recursion when aggregating)
    let _local_only = params.get("local").map(|v| v == "true").unwrap_or(false);

    // Get shard configuration
    let shard_config = collection.get_shard_config();
    let is_sharded = shard_config
        .as_ref()
        .map(|c| c.num_shards > 0)
        .unwrap_or(false);

    // Build sharding stats
    let sharding_stats = if let Some(config) = &shard_config {
        serde_json::json!({
            "enabled": is_sharded,
            "num_shards": config.num_shards,
            "shard_key": config.shard_key,
            "replication_factor": config.replication_factor
        })
    } else {
        serde_json::json!({
            "enabled": false,
            "num_shards": 0,
            "shard_key": null,
            "replication_factor": 1
        })
    };

    // Build cluster distribution info
    let cluster_stats = if let Some(ref coordinator) = state.shard_coordinator {
        let all_nodes = coordinator.get_node_addresses();
        let total_nodes = all_nodes.len();
        let _my_address = coordinator.my_address();

        // For sharded collections, calculate shard distribution with doc counts
        let shard_distribution = if is_sharded {
            let config = shard_config.as_ref().unwrap();

            // Use total document count / num_shards as approximation
            // Scanning all docs is too expensive and blocks the server
            let total_docs = stats.document_count;
            let docs_per_shard = if config.num_shards > 0 {
                total_docs / config.num_shards as usize
            } else {
                total_docs
            };

            let mut shards_info: Vec<serde_json::Value> = Vec::new();

            for shard_id in 0..config.num_shards {
                let mut nodes_for_shard: Vec<String> = Vec::new();

                if total_nodes > 0 {
                    let primary_idx = (shard_id as usize) % total_nodes;
                    let primary_node = all_nodes.get(primary_idx).cloned().unwrap_or_default();
                    nodes_for_shard.push(primary_node);

                    // Replica nodes
                    for r in 1..config.replication_factor {
                        let replica_idx = (primary_idx + r as usize) % total_nodes;
                        if replica_idx != primary_idx {
                            let replica_node =
                                all_nodes.get(replica_idx).cloned().unwrap_or_default();
                            nodes_for_shard.push(replica_node);
                        }
                    }
                }

                shards_info.push(serde_json::json!({
                    "shard_id": shard_id,
                    "nodes": nodes_for_shard,
                    "document_count": docs_per_shard  // Approximate
                }));
            }

            serde_json::to_value(shards_info).unwrap_or(serde_json::json!([]))
        } else {
            // Non-sharded: single "shard" with all docs
            serde_json::json!([{
                "shard_id": 0,
                "nodes": all_nodes.clone(),
                "document_count": stats.document_count
            }])
        };

        serde_json::json!({
            "cluster_mode": true,
            "total_nodes": total_nodes,
            "nodes": all_nodes,
            "shards": shard_distribution
        })
    } else {
        serde_json::json!({
            "cluster_mode": false,
            "total_nodes": 1,
            "nodes": [],
            "distribution": {}
        })
    };

    // Calculate local document count (documents stored on this node's shards)
    // For non-sharded collections, local = total (all replicated)
    // For sharded collections, use total count as approximation
    // (Scanning all docs is too expensive and blocks the server)
    let local_document_count = stats.document_count;

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "type": collection_type,
        "document_count": stats.document_count,
        "local_document_count": local_document_count,
        "disk_usage": {
            "sst_files_size": stats.disk_usage.sst_files_size,
            "live_data_size": stats.disk_usage.live_data_size,
            "num_sst_files": stats.disk_usage.num_sst_files,
            "memtable_size": stats.disk_usage.memtable_size,
            "total_size": stats.disk_usage.sst_files_size + stats.disk_usage.memtable_size
        },
        "sharding": sharding_stats,
        "cluster": cluster_stats
    })))
}
