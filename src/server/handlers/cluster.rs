use super::system::{get_dir_size, AppState};
use crate::{error::DbError, sync::NodeStats};
use axum::{
    extract::{Json, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

// ==================== Cluster Info ====================

#[derive(Debug, Serialize)]
pub struct ClusterInfoResponse {
    pub node_id: String,
    pub is_cluster_mode: bool,
    pub cluster_config: Option<ClusterConfigInfo>,
    // System Stats
    pub cpu_usage: f32,
    pub memory_usage: u64,
    pub memory_total: u64,
    pub uptime: u64,
    pub os_name: String,
    pub os_version: String,
    pub hostname: String,
    pub num_cpus: usize,
}

#[derive(Debug, Serialize)]
pub struct ClusterConfigInfo {
    pub node_id: String,
    pub peers: Vec<String>,
    pub replication_port: u16,
}

pub async fn cluster_info(State(state): State<AppState>) -> Json<ClusterInfoResponse> {
    let node_id = state.storage.node_id().to_string();
    let is_cluster_mode = state.storage.is_cluster_mode();

    let cluster_config = state.storage.cluster_config().map(|c| ClusterConfigInfo {
        node_id: c.node_id.clone(),
        peers: c.peers.clone(),
        replication_port: c.replication_port,
    });

    // Collect System Stats
    let (cpu_usage, memory_usage, memory_total, uptime, os_name, os_version, hostname, num_cpus) = {
        let mut sys = state.system_monitor.lock().unwrap();

        // Refresh specific stats
        sys.refresh_cpu();
        sys.refresh_memory();

        let cpu = sys.global_cpu_info().cpu_usage();
        let mem_used = sys.used_memory();
        let mem_total = sys.total_memory();
        let up = sysinfo::System::uptime();
        let name = sysinfo::System::name().unwrap_or_else(|| "Unknown".to_string());
        let version = sysinfo::System::kernel_version().unwrap_or_else(|| "Unknown".to_string());
        let host = sysinfo::System::host_name().unwrap_or_else(|| "Unknown".to_string());
        let cores = sys.cpus().len();

        (cpu, mem_used, mem_total, up, name, version, host, cores)
    };

    Json(ClusterInfoResponse {
        node_id,
        is_cluster_mode,
        cluster_config,
        cpu_usage,
        memory_usage,
        memory_total,
        uptime,
        os_name,
        os_version,
        hostname,
        num_cpus,
    })
}

// ==================== Cluster Status ====================

#[derive(Debug, Serialize)]
pub struct ClusterStatusResponse {
    pub node_id: String,
    pub status: String,
    pub replication_port: u16,
    pub current_sequence: u64,
    pub log_entries: usize,
    pub peers: Vec<PeerStatusResponse>,
    pub data_dir: String,
    pub stats: NodeStats,
}

#[derive(Debug, Serialize)]
pub struct PeerStatusResponse {
    pub address: String,
    pub is_connected: bool,
    pub last_seen_secs_ago: u64,
    pub replication_lag: u64,
    pub stats: Option<NodeStats>,
}

/// Generate cluster status data (shared between HTTP and WebSocket handlers)
pub fn generate_cluster_status(
    state: &AppState,
    sys: &mut sysinfo::System,
) -> ClusterStatusResponse {
    use std::sync::atomic::Ordering;

    let node_id = state.storage.node_id().to_string();
    let data_dir = state.storage.data_dir().to_string();

    let replication_port = if let Some(ref manager) = state.cluster_manager {
        let addr = manager.get_local_address();
        addr.split(':')
            .next_back()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(6746)
    } else {
        state
            .storage
            .cluster_config()
            .map(|c| c.replication_port)
            .unwrap_or(6746)
    };

    // Calculate stats
    let databases = state.storage.list_databases();
    let _database_count = databases.len();

    let mut collection_count = 0;
    let mut document_count: u64 = 0;

    for db_name in &databases {
        if let Ok(db) = state.storage.get_database(db_name) {
            let coll_names = db.list_collections();
            collection_count += coll_names.len();
            for coll_name in coll_names {
                if let Ok(coll) = db.get_collection(&coll_name) {
                    let stats = coll.stats();
                    document_count += stats.document_count as u64;
                }
            }
        }
    }

    // Storage size (approximate from data directory)
    let storage_bytes = get_dir_size(&data_dir).unwrap_or(0);

    // Uptime
    let _uptime_secs = state.startup_time.elapsed().as_secs();

    // Memory and CPU usage
    sys.refresh_memory();
    let pid = sysinfo::get_current_pid().ok();

    let (memory_used_bytes, cpu_usage_percent) = if let Some(p) = pid {
        sys.refresh_process(p);
        sys.process(p)
            .map(|proc| (proc.memory(), proc.cpu_usage()))
            .unwrap_or((0, 0.0))
    } else {
        (0, 0.0)
    };

    let _memory_total_mb = sys.total_memory() / (1024 * 1024);

    // Request count
    let _request_count = state.request_counter.load(Ordering::Relaxed);

    // Network I/O - use separate Networks struct (sysinfo 0.30 API)
    let _networks = sysinfo::Networks::new_with_refreshed_list();

    // System Load
    let _system_load_avg = sysinfo::System::load_average().one;

    let stats = NodeStats {
        cpu_usage: cpu_usage_percent,
        memory_used: memory_used_bytes,
        disk_used: storage_bytes,
        document_count,
        collections_count: collection_count as u32,
    };

    // Get live status from cluster manager and replication log
    if let Some(ref manager) = state.cluster_manager {
        let member_list = manager.state().get_all_members();

        let status = if member_list.iter().any(|m| {
            m.status == crate::cluster::state::NodeStatus::Active
                && m.node.id != manager.local_node_id()
        }) {
            "cluster".to_string()
        } else if member_list.len() > 1 {
            "cluster-connecting".to_string()
        } else {
            "cluster-ready".to_string()
        };

        let peers: Vec<PeerStatusResponse> = member_list
            .into_iter()
            .filter(|m| m.node.id != manager.local_node_id())
            .map(|m| PeerStatusResponse {
                address: m.node.address,
                is_connected: m.status == crate::cluster::state::NodeStatus::Active,
                last_seen_secs_ago: (chrono::Utc::now().timestamp_millis() as u64
                    - m.last_heartbeat)
                    / 1000,
                replication_lag: 0, // TODO: track actual lag
                stats: None,        // NodeBasicStats conversion to NodeStats not implemented yet
            })
            .collect();

        let (current_seq, count) = if let Some(log) = &state.replication_log {
            (log.current_sequence(), log.current_sequence())
        } else {
            (0, 0)
        };

        ClusterStatusResponse {
            node_id: manager.local_node_id(),
            status,
            replication_port,
            // TODO: We need to put actual logic based on sequence
            current_sequence: current_seq,
            log_entries: count as usize,
            peers,
            data_dir,
            stats,
        }
    } else {
        ClusterStatusResponse {
            node_id,
            status: "standalone".to_string(),
            replication_port,
            current_sequence: 0,
            log_entries: 0,
            peers: vec![],
            data_dir,
            stats,
        }
    }
}

pub async fn cluster_status(State(state): State<AppState>) -> Json<ClusterStatusResponse> {
    use sysinfo::System;
    // For single HTTP request, we create a new system.
    let mut sys = System::new();
    Json(generate_cluster_status(&state, &mut sys))
}

// ==================== Cluster Remove Node ====================

#[derive(Debug, Deserialize)]
pub struct RemoveNodeRequest {
    /// The address of the node to remove (e.g., "localhost:6775")
    pub node_address: String,
}

#[derive(Debug, Serialize)]
pub struct RemoveNodeResponse {
    pub success: bool,
    pub message: String,
    pub removed_node: String,
    pub remaining_nodes: Vec<String>,
}

/// Remove a node from the cluster and trigger rebalancing
pub async fn cluster_remove_node(
    State(state): State<AppState>,
    Json(req): Json<RemoveNodeRequest>,
) -> Result<Json<RemoveNodeResponse>, DbError> {
    let node_addr = req.node_address;

    // Get the shard coordinator
    let coordinator = state.shard_coordinator.as_ref().ok_or_else(|| {
        DbError::InternalError("Shard coordinator not available - not in cluster mode".to_string())
    })?;

    // Remove the node and trigger rebalancing
    coordinator.remove_node(&node_addr).await?;

    // Get remaining nodes
    let remaining = coordinator.get_node_addresses();

    Ok(Json(RemoveNodeResponse {
        success: true,
        message: format!("Node {} removed, rebalancing complete", node_addr),
        removed_node: node_addr,
        remaining_nodes: remaining,
    }))
}

// ==================== Cluster Rebalance ====================

#[derive(Debug, Serialize)]
pub struct RebalanceResponse {
    pub success: bool,
    pub message: String,
}

/// Trigger cluster rebalancing
pub async fn cluster_rebalance(
    State(state): State<AppState>,
) -> Result<Json<RebalanceResponse>, DbError> {
    let coordinator = state.shard_coordinator.as_ref().ok_or_else(|| {
        DbError::InternalError("Shard coordinator not available - not in cluster mode".to_string())
    })?;

    coordinator.rebalance().await?;

    Ok(Json(RebalanceResponse {
        success: true,
        message: "Rebalancing complete".to_string(),
    }))
}

/// Trigger cleanup of orphaned shard collections on this node
/// Called by cluster broadcast after resharding contraction
pub async fn cluster_cleanup(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<Vec<crate::sharding::coordinator::ShardTable>>>,
) -> Result<Json<serde_json::Value>, DbError> {
    // Verify cluster secret
    let secret = state.cluster_secret();
    let request_secret = headers
        .get("X-Cluster-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !secret.is_empty() && request_secret != secret {
        return Err(DbError::BadRequest("Invalid cluster secret".to_string()));
    }

    let coordinator = state
        .shard_coordinator
        .as_ref()
        .ok_or_else(|| DbError::InternalError("Shard coordinator not available".to_string()))?;

    // Update shard tables if provided
    if let Some(Json(tables)) = body {
        tracing::info!(
            "CLEANUP: Received {} updated shard tables from coordinator",
            tables.len()
        );
        for table in tables {
            coordinator.update_shard_table_cache(table);
        }
    }

    let cleaned = coordinator.cleanup_orphaned_shards().await?;

    Ok(Json(serde_json::json!({
        "success": true,
        "cleaned": cleaned
    })))
}

/// Handle reshard request for removed shards during contraction
/// Called by the coordinating node to have this node migrate data from a removed shard
#[derive(Debug, Deserialize)]
pub struct ReshardRequest {
    database: String,
    collection: String,
    old_shards: u16,
    new_shards: u16,
    removed_shard_id: u16,
}

pub async fn cluster_reshard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ReshardRequest>,
) -> Result<Json<serde_json::Value>, DbError> {
    // Verify cluster secret
    let secret = state.cluster_secret();
    let request_secret = headers
        .get("X-Cluster-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !secret.is_empty() && request_secret != secret {
        return Err(DbError::BadRequest("Invalid cluster secret".to_string()));
    }

    let coordinator = state
        .shard_coordinator
        .as_ref()
        .ok_or_else(|| DbError::InternalError("Shard coordinator not available".to_string()))?;

    tracing::info!(
        "RESHARD: Processing migration request for removed shard {}_s{} ({} -> {} shards)",
        request.collection,
        request.removed_shard_id,
        request.old_shards,
        request.new_shards
    );

    // Migrate documents from the removed shard to their new locations
    let physical_name = format!("{}_s{}", request.collection, request.removed_shard_id);

    let db = state.storage.get_database(&request.database)?;
    let physical_coll = match db.get_collection(&physical_name) {
        Ok(c) => c,
        Err(_) => {
            tracing::warn!(
                "RESHARD: Physical shard {} not found locally",
                physical_name
            );
            return Ok(Json(serde_json::json!({
                "success": true,
                "message": "Shard not found locally",
                "migrated": 0
            })));
        }
    };

    let main_coll = db.get_collection(&request.collection)?;
    let config = main_coll
        .get_shard_config()
        .ok_or_else(|| DbError::InternalError("Missing shard config".to_string()))?;

    // Get all documents from the removed shard
    let documents = physical_coll.all();
    let total_docs = documents.len();
    tracing::info!(
        "RESHARD: Migrating {} documents from removed shard {}",
        total_docs,
        physical_name
    );

    // Collect all documents with their new shard destinations
    let mut docs_to_move: Vec<(String, serde_json::Value)> = Vec::new();

    for doc in documents {
        let key = doc.key.clone();
        let route_key = key.clone();

        // Route to new shard
        let new_shard_id =
            crate::sharding::router::ShardRouter::route(&route_key, request.new_shards);

        // Only move if going to a different shard (which it should, since this shard is being removed)
        if new_shard_id != request.removed_shard_id {
            docs_to_move.push((key, doc.to_value()));
        }
    }

    if docs_to_move.is_empty() {
        return Ok(Json(serde_json::json!({
            "success": true,
            "message": "No documents to migrate",
            "migrated": 0
        })));
    }

    // Use upsert to insert into new shards (via coordinator)
    let mut migrated = 0;
    const BATCH_SIZE: usize = 1000;

    for batch in docs_to_move.chunks(BATCH_SIZE) {
        let batch_keyed: Vec<(String, serde_json::Value)> = batch.to_vec();

        // Use upsert via coordinator
        match coordinator
            .upsert_batch_to_shards(&request.database, &request.collection, &config, batch_keyed)
            .await
        {
            Ok(successful_keys) => {
                if !successful_keys.is_empty() {
                    // Cleanup successful keys from physical collection to avoid duplicates?
                    // Or this is a migration?
                    // If migration succeeded, we might delete from source if it was move?
                    // The original code was `physical_coll.delete_batch(&successful_keys)`.
                    // Now `delete_batch` takes Vec<String>.
                    let _ = physical_coll.delete_batch(successful_keys.clone());
                    migrated += successful_keys.len();
                }

                if successful_keys.len() < batch.len() {
                    tracing::warn!(
                        "RESHARD: Batch partial success ({}/{}) - kept failed docs in source",
                        successful_keys.len(),
                        batch.len()
                    );
                }
            }
            Err(e) => {
                tracing::error!("RESHARD: Batch migration failed: {}", e);
            }
        }
    }

    tracing::info!(
        "RESHARD: Migrated {} documents from removed shard {}",
        migrated,
        physical_name
    );

    Ok(Json(serde_json::json!({
        "success": true,
        "migrated": migrated
    })))
}

// ==================== Blob Distribution & Rebalancing ====================

#[derive(Debug, Serialize)]
pub struct BlobDistributionResponse {
    pub nodes: Vec<BlobNodeStats>,
    pub total_chunks: usize,
    pub total_bytes: u64,
    pub mean_chunks_per_node: f64,
    pub std_dev: f64,
    pub imbalance_ratio: f64,
    pub needs_rebalancing: bool,
    pub config: BlobRebalanceConfigInfo,
}

#[derive(Debug, Serialize)]
pub struct BlobNodeStats {
    pub node_id: String,
    pub chunk_count: usize,
    pub total_bytes: u64,
    pub collections: Vec<BlobCollectionStats>,
}

#[derive(Debug, Serialize)]
pub struct BlobCollectionStats {
    pub collection: String,
    pub chunk_count: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct BlobRebalanceConfigInfo {
    pub interval_secs: u64,
    pub imbalance_threshold: f64,
    pub min_chunks_to_rebalance: usize,
    pub batch_size: usize,
    pub enabled: bool,
}

pub async fn blob_distribution(
    State(state): State<AppState>,
) -> Result<Json<BlobDistributionResponse>, DbError> {
    let worker = state.blob_rebalance_worker.as_ref().ok_or_else(|| {
        DbError::InternalError(
            "Blob rebalance worker not available - not in cluster mode".to_string(),
        )
    })?;

    let config = worker.config();
    let all_stats = worker
        .collect_node_stats()
        .await
        .map_err(|e| DbError::InternalError(format!("Failed to collect blob stats: {}", e)))?;

    let metrics = worker
        .calculate_distribution_metrics(&all_stats)
        .map_err(|e| DbError::InternalError(format!("Failed to calculate metrics: {}", e)))?;

    let total_chunks: usize = all_stats.iter().map(|n| n.chunk_count).sum();
    let total_bytes: u64 = all_stats.iter().map(|n| n.total_bytes).sum();

    let imbalance_ratio = if metrics.mean_chunks > 0.0 {
        metrics.std_dev / metrics.mean_chunks
    } else {
        0.0
    };

    let needs_rebalancing = total_chunks >= config.min_chunks_to_rebalance
        && imbalance_ratio >= config.imbalance_threshold;

    let nodes: Vec<BlobNodeStats> = all_stats
        .into_iter()
        .map(|n| {
            let collections: Vec<BlobCollectionStats> = n
                .collections
                .iter()
                .map(|(name, stats)| BlobCollectionStats {
                    collection: name.clone(),
                    chunk_count: stats.chunk_count,
                    total_bytes: stats.total_bytes,
                })
                .collect();

            BlobNodeStats {
                node_id: n.node_id,
                chunk_count: n.chunk_count,
                total_bytes: n.total_bytes,
                collections,
            }
        })
        .collect();

    Ok(Json(BlobDistributionResponse {
        nodes,
        total_chunks,
        total_bytes,
        mean_chunks_per_node: metrics.mean_chunks,
        std_dev: metrics.std_dev,
        imbalance_ratio,
        needs_rebalancing,
        config: BlobRebalanceConfigInfo {
            interval_secs: config.interval_secs,
            imbalance_threshold: config.imbalance_threshold,
            min_chunks_to_rebalance: config.min_chunks_to_rebalance,
            batch_size: config.batch_size,
            enabled: config.enabled,
        },
    }))
}

#[derive(Debug, Deserialize)]
pub struct BlobRebalanceRequest {
    #[serde(default = "default_force")]
    pub force: bool,
}

fn default_force() -> bool {
    false
}

pub async fn blob_rebalance(
    State(state): State<AppState>,
    Json(request): Json<BlobRebalanceRequest>,
) -> Result<Json<serde_json::Value>, DbError> {
    let worker = state.blob_rebalance_worker.as_ref().ok_or_else(|| {
        DbError::InternalError(
            "Blob rebalance worker not available - not in cluster mode".to_string(),
        )
    })?;

    if request.force {
        worker
            .check_and_rebalance()
            .await
            .map_err(|e| DbError::InternalError(format!("Blob rebalance failed: {}", e)))?;
        Ok(Json(serde_json::json!({
            "success": true,
            "message": "Blob rebalancing triggered successfully"
        })))
    } else {
        let all_stats = worker
            .collect_node_stats()
            .await
            .map_err(|e| DbError::InternalError(format!("Failed to collect blob stats: {}", e)))?;

        let metrics = worker
            .calculate_distribution_metrics(&all_stats)
            .map_err(|e| DbError::InternalError(format!("Failed to calculate metrics: {}", e)))?;

        let total_chunks: usize = all_stats.iter().map(|n| n.chunk_count).sum();
        let config = worker.config();

        let imbalance_ratio = if metrics.mean_chunks > 0.0 {
            metrics.std_dev / metrics.mean_chunks
        } else {
            0.0
        };

        let needs_rebalancing = total_chunks >= config.min_chunks_to_rebalance
            && imbalance_ratio >= config.imbalance_threshold;

        Ok(Json(serde_json::json!({
            "success": true,
            "message": if needs_rebalancing {
                "Rebalancing recommended"
            } else {
                "No rebalancing needed"
            },
            "needs_rebalancing": needs_rebalancing,
            "total_chunks": total_chunks,
            "mean_chunks_per_node": metrics.mean_chunks,
            "std_dev": metrics.std_dev,
            "imbalance_ratio": imbalance_ratio,
            "threshold": config.imbalance_threshold,
            "min_chunks_required": config.min_chunks_to_rebalance
        })))
    }
}
