use super::super::system::{is_protected_collection, AppState};
use crate::error::DbError;
use crate::storage::http_client::get_http_client;
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Json,
};
use serde::Serialize;
use serde_json::Value;

// ==================== Structs ====================

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

// ==================== Handlers ====================

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
        let has_shard_suffix = name
            .rfind("_s")
            .map(|i| {
                // Check if what follows _s is a number
                name[i + 2..].chars().all(|c| c.is_ascii_digit())
            })
            .unwrap_or(false);

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

                    let client = get_http_client();
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
