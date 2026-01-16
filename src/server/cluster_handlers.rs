//! Cluster management handlers for multi-node SoliDB deployments.
//!
//! This module provides HTTP and WebSocket handlers for:
//! - Cluster status monitoring
//! - Node management (add/remove nodes)
//! - Shard rebalancing
//! - System monitoring

use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query as AxumQuery, State,
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};

use crate::cluster::stats::NodeBasicStats;
use crate::error::DbError;

use super::handlers::{AppState, AuthParams};

// ==================== Cluster Status Types ====================

#[derive(Debug, Serialize)]
pub struct PeerStatusResponse {
    pub address: String,
    pub is_connected: bool,
    pub last_seen_secs_ago: u64,
    pub replication_lag: u64,
    pub stats: Option<NodeBasicStats>,
}

#[derive(Debug, Serialize)]
pub struct NodeStats {
    pub database_count: usize,
    pub collection_count: usize,
    pub document_count: u64,
    pub storage_bytes: u64,
    pub uptime_secs: u64,
    pub memory_used_mb: u64,
    pub memory_total_mb: u64,
    pub cpu_usage_percent: f32,
    pub request_count: u64,
    // New stats
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
    pub system_load_avg: f64,
    pub total_file_count: u64,
    pub total_chunk_count: u64,
    pub total_sst_size: u64,
    pub total_memtable_size: u64,
    pub total_live_size: u64,
}

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

/// Generate cluster status data (shared between HTTP and WebSocket handlers)
pub(crate) fn generate_cluster_status(
    state: &AppState,
    sys: &mut sysinfo::System,
) -> ClusterStatusResponse {
    use std::sync::atomic::Ordering;

    let node_id = state.storage.node_id().to_string();
    let data_dir = state.storage.data_dir().to_string();

    let replication_port = if let Some(ref manager) = state.cluster_manager {
        let addr = manager.get_local_address();
        addr.split(':')
            .last()
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
    let database_count = databases.len();

    let mut collection_count = 0;
    let mut document_count: u64 = 0;
    let mut total_file_count: u64 = 0;
    let mut total_chunk_count: u64 = 0;
    let mut total_sst_size: u64 = 0;
    let mut total_memtable_size: u64 = 0;
    let mut total_live_size: u64 = 0;

    for db_name in &databases {
        if let Ok(db) = state.storage.get_database(db_name) {
            let coll_names = db.list_collections();
            collection_count += coll_names.len();
            for coll_name in coll_names {
                if let Ok(coll) = db.get_collection(&coll_name) {
                    let stats = coll.stats();
                    document_count += stats.document_count as u64;
                    total_file_count += stats.disk_usage.num_sst_files;
                    total_chunk_count += stats.chunk_count as u64;
                    total_sst_size += stats.disk_usage.sst_files_size;
                    total_memtable_size += stats.disk_usage.memtable_size;
                    total_live_size += stats.disk_usage.live_data_size;
                }
            }
        }
    }

    // Storage size (approximate from data directory)
    let storage_bytes = get_dir_size(&data_dir).unwrap_or(0);

    // Uptime
    let uptime_secs = state.startup_time.elapsed().as_secs();

    // Memory and CPU usage
    sys.refresh_memory();
    let pid = sysinfo::get_current_pid().ok();

    let (memory_used_mb, cpu_usage_percent) = if let Some(p) = pid {
        sys.refresh_process(p);
        sys.process(p)
            .map(|proc| (proc.memory() / (1024 * 1024), proc.cpu_usage()))
            .unwrap_or((0, 0.0))
    } else {
        (0, 0.0)
    };

    let memory_total_mb = sys.total_memory() / (1024 * 1024);

    // Request count
    let request_count = state.request_counter.load(Ordering::Relaxed);

    // Network I/O - use separate Networks struct (sysinfo 0.30 API)
    let networks = sysinfo::Networks::new_with_refreshed_list();
    let mut network_rx_bytes = 0u64;
    let mut network_tx_bytes = 0u64;
    for (_, network) in &networks {
        network_rx_bytes += network.total_received();
        network_tx_bytes += network.total_transmitted();
    }

    // System Load
    let system_load_avg = sysinfo::System::load_average().one;

    let stats = NodeStats {
        database_count,
        collection_count,
        document_count,
        storage_bytes,
        uptime_secs,
        memory_used_mb,
        memory_total_mb,
        cpu_usage_percent,
        request_count,
        network_rx_bytes,
        network_tx_bytes,
        system_load_avg,
        total_file_count,
        total_chunk_count,
        total_sst_size,
        total_memtable_size,
        total_live_size,
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
                stats: m.stats.clone(),
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

/// Get cluster status via HTTP
pub async fn cluster_status(State(state): State<AppState>) -> Json<ClusterStatusResponse> {
    use sysinfo::System;
    // For single HTTP request, we create a new system.
    // Note: CPU usage might be inaccurate (0.0) for single requests without a previous refresh.
    // If accurate CPU is needed on HTTP, we'd need to sleep/refresh, but avoiding blocking is better.
    let mut sys = System::new();
    Json(generate_cluster_status(&state, &mut sys))
}

/// WebSocket handler for real-time cluster status updates
pub async fn cluster_status_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_cluster_ws(socket, state))
}

/// Handle the WebSocket connection for cluster status
async fn handle_cluster_ws(mut socket: axum::extract::ws::WebSocket, state: AppState) {
    use axum::extract::ws::Message;
    use tokio::time::{interval, Duration};

    let mut ticker = interval(Duration::from_secs(1));

    // We use the shared system monitor from AppState to avoid expensive initialization
    // and to ensure CPU usage is calculated correctly (delta since last refresh).

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Generate status using shared logic and persistent sys
                let status = {
                    let mut sys = state.system_monitor.lock().unwrap();
                    generate_cluster_status(&state, &mut *sys)
                };

                let json = match serde_json::to_string(&status) {
                    Ok(j) => j,
                    Err(_) => continue,
                };

                if socket.send(Message::Text(json.into())).await.is_err() {
                    break; // Client disconnected
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(data))) => {
                        // Respond to ping with pong
                        if socket.send(Message::Pong(data)).await.is_err() {
                            break;
                        }
                    }
                    _ => {} // Ignore other messages
                }
            }
        }
    }
}

/// Get the size of a directory in bytes (recursive)
fn get_dir_size(path: &str) -> std::io::Result<u64> {
    let mut size = 0u64;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            size += get_dir_size(entry.path().to_str().unwrap_or(""))?;
        } else {
            size += metadata.len();
        }
    }
    Ok(size)
}

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

// ==================== System Monitoring WebSocket ====================

pub async fn monitor_ws_handler(
    ws: WebSocketUpgrade,
    AxumQuery(params): AxumQuery<AuthParams>,
    State(state): State<AppState>,
) -> Response {
    if crate::server::auth::AuthService::validate_token(&params.token).is_err() {
        return Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Body::empty())
            .expect("Valid status code should not fail")
            .into_response();
    }

    ws.on_upgrade(|socket| handle_monitor_socket(socket, state))
}

async fn handle_monitor_socket(mut socket: WebSocket, state: AppState) {
    use std::sync::atomic::Ordering;

    tracing::info!("Monitor WS: Client connected");

    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));

    loop {
        // Wait for next tick
        interval.tick().await;

        let stats = {
            let mut sys = state.system_monitor.lock().unwrap();

            // Refresh specific stats
            sys.refresh_cpu();
            sys.refresh_memory();

            let cpu = sys.global_cpu_info().cpu_usage();
            let mem_used = sys.used_memory();
            let mem_total = sys.total_memory();
            let up = sysinfo::System::uptime();
            let name = sysinfo::System::name().unwrap_or_else(|| "Unknown".to_string());
            let version =
                sysinfo::System::kernel_version().unwrap_or_else(|| "Unknown".to_string());
            let host = sysinfo::System::host_name().unwrap_or_else(|| "Unknown".to_string());
            let cores = sys.cpus().len();

            serde_json::json!({
                "cpu_usage": cpu,
                "memory_usage": mem_used,
                "memory_total": mem_total,
                "uptime": up,
                "os_name": name,
                "os_version": version,
                "hostname": host,
                "num_cpus": cores,
                "pid": std::process::id(),
                "active_scripts": state.script_stats.active_scripts.load(Ordering::Relaxed),
                "active_ws": state.script_stats.active_ws.load(Ordering::Relaxed)
            })
        };

        let msg = match serde_json::to_string(&stats) {
            Ok(s) => s,
            Err(_) => continue,
        };

        if socket.send(Message::Text(msg.into())).await.is_err() {
            // Client disconnected
            break;
        }
    }
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
        let route_key = if config.shard_key == "_key" {
            key.clone()
        } else {
            key.clone()
        };

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
                    // Delete ONLY successfully migrated documents from source
                    let _ = physical_coll.delete_batch(&successful_keys);
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
