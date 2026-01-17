use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::Json,
};
use serde_json::Value;
use std::collections::HashMap;
use crate::error::DbError;
use super::system::AppState;

/// Format size in human-readable format
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Get detailed sharding information including per-shard document counts, disk sizes, and node assignments
pub async fn get_sharding_details(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let shard_config = collection.get_shard_config();

    let is_sharded = shard_config
        .as_ref()
        .map(|c| c.num_shards > 0)
        .unwrap_or(false);

    if !is_sharded {
        // Not a sharded collection
        let stats = collection.stats();
        return Ok(Json(serde_json::json!({
            "database": db_name,
            "collection": coll_name,
            "type": collection.get_type(),
            "sharded": false,
            "total_documents": stats.document_count,
            "total_size": stats.disk_usage.sst_files_size + stats.disk_usage.memtable_size,
            "shards": []
        })));
    }

    let config = shard_config.unwrap();

    // Get cluster nodes info
    let (nodes, healthy_nodes, node_id_to_address) =
        if let Some(ref coordinator) = state.shard_coordinator {
            let all_node_ids = coordinator.get_node_ids();
            let my_node_id = coordinator.my_node_id();

            // Build node ID to address mapping
            let mut id_to_addr: HashMap<String, String> = HashMap::new();
            if let Some(ref mgr) = state.cluster_manager {
                for member in mgr.state().get_all_members() {
                    id_to_addr.insert(member.node.id.clone(), member.node.api_address.clone());
                }
            }

            // Get healthy nodes from cluster manager
            let healthy = if let Some(ref mgr) = state.cluster_manager {
                mgr.get_healthy_nodes()
            } else {
                vec![my_node_id.clone()]
            };

            (all_node_ids, healthy, id_to_addr)
        } else {
            (
                vec!["local".to_string()],
                vec!["local".to_string()],
                HashMap::new(),
            )
        };

    // Get shard table for assignments
    let shard_table = if let Some(ref coordinator) = state.shard_coordinator {
        coordinator.get_shard_table(&db_name, &coll_name)
    } else {
        None
    };

    let mut shards_info: Vec<serde_json::Value> = Vec::new();
    let mut total_documents = 0u64;
    let mut total_size = 0u64;

    // Get my node ID to check if shard is local
    let my_node_id = if let Some(ref coordinator) = state.shard_coordinator {
        coordinator.my_node_id()
    } else {
        "local".to_string()
    };

    // Query each physical shard for actual stats
    // Use scatter-gather to query remote nodes when shard isn't local
    let client = reqwest::Client::new();
    let secret = state.cluster_secret();

    // Get auth token from request headers to forward to remote nodes
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    for shard_id in 0..config.num_shards {
        let physical_coll_name = format!("{}_s{}", coll_name, shard_id);

        // Get assignment info first
        let (primary_node, replica_nodes) = if let Some(ref table) = shard_table {
            if let Some(assignment) = table.assignments.get(&shard_id) {
                (
                    assignment.primary_node.clone(),
                    assignment.replica_nodes.clone(),
                )
            } else {
                ("unknown".to_string(), vec![])
            }
        } else {
            // Fall back to computing assignment based on modulo
            let num_nodes = nodes.len();
            if num_nodes > 0 {
                let primary_idx = (shard_id as usize) % num_nodes;
                let primary = nodes.get(primary_idx).cloned().unwrap_or_default();
                let mut replicas = Vec::new();
                for r in 1..config.replication_factor {
                    let replica_idx = (primary_idx + r as usize) % num_nodes;
                    if replica_idx != primary_idx {
                        if let Some(n) = nodes.get(replica_idx) {
                            replicas.push(n.clone());
                        }
                    }
                }
                (primary, replicas)
            } else {
                ("local".to_string(), vec![])
            }
        };

        // Get stats - either local or remote
        let mut stats_result: Option<(u64, u64, u64)> = None;

        // 1. Try Primary Node (Local or Remote)
        if primary_node == my_node_id || primary_node == "local" {
            // Local primary
            if let Ok(physical_coll) = database.get_collection(&physical_coll_name) {
                let stats = physical_coll.stats();
                stats_result = Some((
                    stats.document_count as u64,
                    stats.chunk_count as u64,
                    stats.disk_usage.sst_files_size + stats.disk_usage.memtable_size,
                ));
            }
        } else if let Some(primary_addr) = node_id_to_address.get(&primary_node) {
            // Remote primary
            let scheme =
                std::env::var("SOLIDB_CLUSTER_SCHEME").unwrap_or_else(|_| "http".to_string());
            let url = format!(
                "{}://{}/_api/database/{}/collection/{}/stats",
                scheme, primary_addr, db_name, physical_coll_name
            );
            let mut req = client
                .get(&url)
                .header("X-Cluster-Secret", &secret)
                .timeout(std::time::Duration::from_secs(3));

            if !auth_header.is_empty() {
                req = req.header("Authorization", &auth_header);
            }

            if let Ok(res) = req.send().await {
                if res.status().is_success() {
                    if let Ok(body) = res.json::<serde_json::Value>().await {
                        let count = body
                            .get("document_count")
                            .and_then(|c| c.as_u64())
                            .unwrap_or(0);
                        let chunk_count = body
                            .get("chunk_count")
                            .and_then(|c| c.as_u64())
                            .unwrap_or(0);
                        let disk = body.get("disk_usage");
                        let size = disk
                            .map(|d| {
                                let sst = d
                                    .get("sst_files_size")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let mem =
                                    d.get("memtable_size").and_then(|v| v.as_u64()).unwrap_or(0);
                                sst + mem
                            })
                            .unwrap_or(0);
                        stats_result = Some((count, chunk_count, size));
                    }
                }
            }
        }

        // 2. Fallback to Replicas if Primary failed
        if stats_result.is_none() {
            for replica_node in &replica_nodes {
                if replica_node == &my_node_id {
                    // Local replica
                    if let Ok(physical_coll) = database.get_collection(&physical_coll_name) {
                        let stats = physical_coll.stats();
                        stats_result = Some((
                            stats.document_count as u64,
                            stats.chunk_count as u64,
                            stats.disk_usage.sst_files_size + stats.disk_usage.memtable_size,
                        ));
                        break;
                    }
                } else if let Some(replica_addr) = node_id_to_address.get(replica_node) {
                    // Remote replica
                    let scheme = std::env::var("SOLIDB_CLUSTER_SCHEME")
                        .unwrap_or_else(|_| "http".to_string());
                    let url = format!(
                        "{}://{}/_api/database/{}/collection/{}/stats",
                        scheme, replica_addr, db_name, physical_coll_name
                    );
                    let mut req = client
                        .get(&url)
                        .header("X-Cluster-Secret", &secret)
                        .timeout(std::time::Duration::from_secs(2));

                    if !auth_header.is_empty() {
                        req = req.header("Authorization", &auth_header);
                    }

                    if let Ok(res) = req.send().await {
                        if res.status().is_success() {
                            if let Ok(body) = res.json::<serde_json::Value>().await {
                                let count = body
                                    .get("document_count")
                                    .and_then(|c| c.as_u64())
                                    .unwrap_or(0);
                                let chunk_count = body
                                    .get("chunk_count")
                                    .and_then(|c| c.as_u64())
                                    .unwrap_or(0);
                                let disk = body.get("disk_usage");
                                let size = disk
                                    .and_then(|d| {
                                        let sst = d
                                            .get("sst_files_size")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let mem = d
                                            .get("memtable_size")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        Some(sst + mem)
                                    })
                                    .unwrap_or(0);
                                stats_result = Some((count, chunk_count, size));
                                break;
                            }
                        }
                    }
                }
            }
        }

        let (doc_count, chunk_count, disk_size) = stats_result.unwrap_or((0, 0, 0));
        let fetch_failed = stats_result.is_none();

        let primary_healthy = healthy_nodes.contains(&primary_node);
        let primary_address = node_id_to_address
            .get(&primary_node)
            .cloned()
            .unwrap_or_else(|| primary_node.clone());

        // Build replica info with health status
        let replicas_info: Vec<serde_json::Value> = replica_nodes
            .iter()
            .map(|node_id| {
                let is_healthy = healthy_nodes.contains(node_id);
                let address = node_id_to_address
                    .get(node_id)
                    .cloned()
                    .unwrap_or_else(|| node_id.clone());
                serde_json::json!({
                    "node_id": node_id,
                    "address": address,
                    "healthy": is_healthy
                })
            })
            .collect();

        total_documents += doc_count;
        total_size += disk_size;

        // Status checks - distinguish between dead node and syncing shard
        let shard_status = if !primary_healthy {
            "dead" // Node is actually unhealthy
        } else if fetch_failed && primary_node != my_node_id {
            "syncing" // Node is healthy but shard data not available yet
        } else {
            "healthy"
        };

        shards_info.push(serde_json::json!({
            "shard_id": shard_id,
            "physical_collection": physical_coll_name,
            "document_count": doc_count,
            "chunk_count": chunk_count,
            "disk_size": disk_size,
            "disk_size_formatted": format_size(disk_size),
            "primary": {
                "node_id": primary_node,
                "address": primary_address,
                "healthy": primary_healthy
            },
            "replicas": replicas_info,
            "status": shard_status,
            "fetch_failed": fetch_failed
        }));
    }

    // Build node summary with actual status from cluster state
    // NodeId -> (PrimaryDocs, ReplicaDocs, PrimarySize, ReplicaSize, HasFailedFetch)
    struct NodeStat {
        primary_docs: u64,
        replica_docs: u64,
        primary_chunks: u64,
        replica_chunks: u64,
        primary_size: u64,
        replica_size: u64,
        has_failed: bool,
    }

    let mut node_stats: HashMap<String, NodeStat> = HashMap::new();

    // First, initialize with all primary nodes from assignment to ensure we track them even if fetch failed
    for shard in &shards_info {
        // Track Primary Stats
        if let Some(primary) = shard
            .get("primary")
            .and_then(|p| p.get("node_id"))
            .and_then(|n| n.as_str())
        {
            let doc_count = shard
                .get("document_count")
                .and_then(|d| d.as_u64())
                .unwrap_or(0);
            let chunk_count = shard
                .get("chunk_count")
                .and_then(|c| c.as_u64())
                .unwrap_or(0);
            let disk_size = shard.get("disk_size").and_then(|d| d.as_u64()).unwrap_or(0);
            let fetch_failed = shard
                .get("fetch_failed")
                .and_then(|f| f.as_bool())
                .unwrap_or(false);

            let entry = node_stats.entry(primary.to_string()).or_insert(NodeStat {
                primary_docs: 0,
                replica_docs: 0,
                primary_chunks: 0,
                replica_chunks: 0,
                primary_size: 0,
                replica_size: 0,
                has_failed: false,
            });

            entry.primary_docs += doc_count;
            entry.primary_chunks += chunk_count;
            entry.primary_size += disk_size;
            if fetch_failed {
                entry.has_failed = true;
            }
        }

        // Track Replica Stats
        if let Some(replicas) = shard.get("replicas").and_then(|r| r.as_array()) {
            for replica in replicas {
                if let Some(replica_node) = replica.get("node_id").and_then(|n| n.as_str()) {
                    // Replicas have the same doc count/disk size as the primary (they're copies)
                    let doc_count = shard
                        .get("document_count")
                        .and_then(|d| d.as_u64())
                        .unwrap_or(0);
                    let chunk_count = shard
                        .get("chunk_count")
                        .and_then(|c| c.as_u64())
                        .unwrap_or(0);
                    let disk_size = shard.get("disk_size").and_then(|d| d.as_u64()).unwrap_or(0);

                    let entry = node_stats
                        .entry(replica_node.to_string())
                        .or_insert(NodeStat {
                            primary_docs: 0,
                            replica_docs: 0,
                            primary_chunks: 0,
                            replica_chunks: 0,
                            primary_size: 0,
                            replica_size: 0,
                            has_failed: false,
                        });

                    entry.replica_docs += doc_count;
                    entry.replica_chunks += chunk_count;
                    entry.replica_size += disk_size;
                }
            }
        }
    }

    // ALSO include all cluster members - not just those in shard assignments
    // This ensures returning nodes that haven't been assigned shards yet still appear
    if let Some(ref mgr) = state.cluster_manager {
        for member in mgr.state().get_all_members() {
            // Add to node_stats if not already present
            node_stats
                .entry(member.node.id.clone())
                .or_insert(NodeStat {
                    primary_docs: 0,
                    replica_docs: 0,
                    primary_chunks: 0,
                    replica_chunks: 0,
                    primary_size: 0,
                    replica_size: 0,
                    has_failed: false,
                });
        }
    }

    let nodes_summary: Vec<serde_json::Value> = node_stats
        .iter()
        .map(|(node_id, stats)| {
            let is_healthy = healthy_nodes.contains(node_id);
            let address = node_id_to_address
                .get(node_id)
                .cloned()
                .unwrap_or_else(|| node_id.clone());
            let shard_count = shards_info
                .iter()
                .filter(|s| {
                    s.get("primary")
                        .and_then(|p| p.get("node_id"))
                        .and_then(|n| n.as_str())
                        == Some(node_id)
                })
                .count();

            // Count replica shards for this node
            let replica_count = shards_info
                .iter()
                .filter(|s| {
                    if let Some(replicas) = s.get("replicas").and_then(|r| r.as_array()) {
                        replicas
                            .iter()
                            .any(|r| r.get("node_id").and_then(|n| n.as_str()) == Some(node_id))
                    } else {
                        false
                    }
                })
                .count();

            // Get actual node status from cluster manager
            let mut status = if let Some(ref mgr) = state.cluster_manager {
                if let Some(member) = mgr.state().get_member(node_id) {
                    match member.status {
                        crate::cluster::state::NodeStatus::Syncing => "syncing",
                        crate::cluster::state::NodeStatus::Active => "healthy",
                        crate::cluster::state::NodeStatus::Joining => "joining",
                        crate::cluster::state::NodeStatus::Suspected => "suspected",
                        crate::cluster::state::NodeStatus::Dead => "dead",
                        crate::cluster::state::NodeStatus::Leaving => "leaving",
                    }
                } else if is_healthy {
                    // Node id not in cluster state but marked healthy (shouldn't happen normally)
                    "healthy"
                } else {
                    // Node was removed from cluster - mark as dead
                    "dead"
                }
            } else if is_healthy {
                "healthy"
            } else {
                // No cluster manager and not healthy - assume dead
                "dead"
            };

            // Override status if fetch failed - node is healthy but missing data
            if status == "healthy" && stats.has_failed {
                status = "syncing"; // Node is healthy but missing data, needs sync
            }

            let total_docs = stats.primary_docs + stats.replica_docs;
            let total_chunks = stats.primary_chunks + stats.replica_chunks;
            let total_size = stats.primary_size + stats.replica_size;

            serde_json::json!({
                "node_id": node_id,
                "address": address,
                "healthy": is_healthy,
                "status": status,
                "primary_shards": shard_count,
                "replica_shards": replica_count,
                "document_count": total_docs,
                "chunk_count": total_chunks,
                "primary_docs": stats.primary_docs,
                "replica_docs": stats.replica_docs,
                "primary_chunks": stats.primary_chunks,
                "replica_chunks": stats.replica_chunks,
                "disk_size": total_size,
                "primary_size": stats.primary_size,
                "replica_size": stats.replica_size,
                "disk_size_formatted": format_size(total_size)
            })
        })
        .collect();

    let mut nodes_sorted = nodes_summary;
    nodes_sorted.sort_by(|a, b| {
        let addr_a = a.get("address").and_then(|s| s.as_str()).unwrap_or("");
        let addr_b = b.get("address").and_then(|s| s.as_str()).unwrap_or("");
        addr_a.cmp(addr_b)
    });

    Ok(Json(serde_json::json!({
        "database": db_name,
        "collection": coll_name,
        "type": collection.get_type(),
        "sharded": true,
        "config": {
            "num_shards": config.num_shards,
            "shard_key": config.shard_key,
            "replication_factor": config.replication_factor
        },
        "total_documents": total_documents,
        // Calculate total chunks for the collection (sum of primary chunks)
        "total_chunks": node_stats.values().map(|s| s.primary_chunks).sum::<u64>(),
        "total_size": total_size,
        "total_size_formatted": format_size(total_size),
        "cluster": {
            "total_nodes": nodes.len(),
            "healthy_nodes": healthy_nodes.len()
        },
        "nodes": nodes_sorted,
        "shards": shards_info
    })))
}
