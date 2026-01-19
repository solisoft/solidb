//! Shard healing operations
//!
//! This module handles shard healing when nodes fail or become unhealthy.

#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::result_large_err
)]

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

use crate::cluster::manager::ClusterManager;
use crate::sharding::coordinator::ShardTable;
use crate::storage::StorageEngine;
use crate::DbError;

/// Check if a node recently failed and came back online
/// This helps avoid using stale data from nodes that just recovered
pub fn was_recently_failed(
    recently_failed_nodes: &RwLock<HashMap<String, std::time::Instant>>,
    node_id: &str,
) -> bool {
    const RECENT_FAILURE_WINDOW_SECS: u64 = 300; // 5 minutes

    let recently_failed = recently_failed_nodes.read().unwrap();
    if let Some(failure_time) = recently_failed.get(node_id) {
        let elapsed = failure_time.elapsed();
        elapsed.as_secs() < RECENT_FAILURE_WINDOW_SECS
    } else {
        false
    }
}

/// Record that a node failed (called when failover occurs)
pub fn record_node_failure(
    recently_failed_nodes: &RwLock<HashMap<String, std::time::Instant>>,
    node_id: &str,
) {
    let mut recently_failed = recently_failed_nodes.write().unwrap();
    recently_failed.insert(node_id.to_string(), std::time::Instant::now());
    tracing::info!("Recorded node failure for {}", node_id);
}

/// Clear failure record when node is confirmed healthy
pub fn clear_node_failure(
    recently_failed_nodes: &RwLock<HashMap<String, std::time::Instant>>,
    node_id: &str,
) {
    let mut recently_failed = recently_failed_nodes.write().unwrap();
    recently_failed.remove(node_id);
    tracing::info!("Cleared failure record for {}", node_id);
}

/// Clean up old failure records
pub fn cleanup_old_failures(recently_failed_nodes: &RwLock<HashMap<String, std::time::Instant>>) {
    const MAX_AGE_SECS: u64 = 3600; // 1 hour
    let mut recently_failed = recently_failed_nodes.write().unwrap();
    let now = std::time::Instant::now();

    recently_failed
        .retain(|_, failure_time| now.duration_since(*failure_time).as_secs() < MAX_AGE_SECS);
}

/// Clear failure records for nodes that are currently healthy
pub fn clear_failures_for_healthy_nodes(
    recently_failed_nodes: &RwLock<HashMap<String, std::time::Instant>>,
    cluster_manager: &Option<Arc<ClusterManager>>,
) {
    if let Some(mgr) = cluster_manager {
        let healthy_nodes = mgr.get_healthy_nodes();
        let mut recently_failed = recently_failed_nodes.write().unwrap();

        for healthy_node in &healthy_nodes {
            if recently_failed.contains_key(healthy_node) {
                recently_failed.remove(healthy_node);
                tracing::info!("Cleared failure record for healthy node {}", healthy_node);
            }
        }
    }
}

/// Check if resharding should be paused due to cluster health issues
pub fn should_pause_resharding(
    cluster_manager: &Option<Arc<ClusterManager>>,
    recently_failed_nodes: &RwLock<HashMap<String, std::time::Instant>>,
) -> bool {
    if let Some(mgr) = cluster_manager {
        let healthy_nodes = mgr.get_healthy_nodes();
        let total_nodes = mgr.state().get_all_members().len();

        // Pause resharding if less than 50% of nodes are healthy
        if healthy_nodes.len() < total_nodes.div_ceil(2) {
            tracing::warn!(
                "RESHARD: Pausing resharding - only {}/{} nodes are healthy",
                healthy_nodes.len(),
                total_nodes
            );
            return true;
        }

        // Also check for recently failed nodes
        let recently_failed = recently_failed_nodes.read().unwrap();
        if recently_failed.len() > total_nodes / 2 {
            tracing::warn!(
                "RESHARD: Pausing resharding - {}/{} nodes recently failed",
                recently_failed.len(),
                total_nodes
            );
            return true;
        }
    }
    false
}

/// Copy shard data from a source node
pub async fn copy_shard_from_source(
    storage: &Arc<StorageEngine>,
    cluster_manager: &Option<Arc<ClusterManager>>,
    cluster_secret: &str,
    database: &str,
    physical_coll: &str,
    source_node: &str,
) -> Result<usize, DbError> {
    use base64::{engine::general_purpose, Engine as _};

    let mgr = cluster_manager
        .as_ref()
        .ok_or_else(|| DbError::InternalError("No cluster manager".to_string()))?;

    let source_addr = mgr
        .get_node_api_address(source_node)
        .ok_or_else(|| DbError::InternalError("Source node address not found".to_string()))?;

    // Step 1: Check Source Count using Metadata API
    let client = reqwest::Client::new();

    // Use standard Collection API to get metadata (count)
    let meta_url = format!(
        "http://{}/_api/database/{}/collection/{}",
        source_addr, database, physical_coll
    );
    let meta_res = client
        .get(&meta_url)
        .header("X-Cluster-Secret", cluster_secret)
        .header("X-Shard-Direct", "true")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let mut source_count = 0;
    let mut check_count = false;

    if let Ok(res) = meta_res {
        if res.status().is_success() {
            if let Ok(json) = res.json::<serde_json::Value>().await {
                if let Some(c) = json.get("count").and_then(|v| v.as_u64()) {
                    source_count = c as usize;
                    check_count = true;
                }
            }
        }
    }

    // Local Check & Prep
    let db = storage.get_database(database)?;
    let coll = match db.get_collection(physical_coll) {
        Ok(c) => c,
        Err(_) => {
            db.create_collection(physical_coll.to_string(), None)?;
            db.get_collection(physical_coll)?
        }
    };

    // Optimize: If counts match and NOT a blob collection (doc count doesn't track chunks), skip
    // For blob collections, we always resync if triggered to ensure chunks are present
    let is_blob = coll.get_type() == "blob";
    if check_count {
        let local_count = coll.count();
        if local_count == source_count && !is_blob {
            return Ok(0);
        }
        if local_count != source_count || is_blob {
            tracing::info!(
                "HEAL: Mismatch or Blob forced sync for {}/{} (Local: {}, Source: {}). Syncing.",
                database,
                physical_coll,
                local_count,
                source_count
            );
            let _ = coll.truncate();
        }
    }

    // Use EXPORT endpoint to stream all data (Docs + Blob Chunks)
    let scheme = std::env::var("SOLIDB_CLUSTER_SCHEME").unwrap_or_else(|_| "http".to_string());
    let url = format!(
        "{}://{}/_api/database/{}/collection/{}/export",
        scheme, source_addr, database, physical_coll
    );

    let mut resp = client
        .get(&url)
        .header("X-Cluster-Secret", cluster_secret)
        .header("X-Shard-Direct", "true")
        .timeout(std::time::Duration::from_secs(3600)) // Long timeout for large shards
        .send()
        .await
        .map_err(|e| DbError::InternalError(format!("Export request failed: {}", e)))?;

    if !resp.status().is_success() {
        let status = resp.status();
        tracing::error!("HEAL: Export failed - status: {}, url: {}", status, url);
        return Err(DbError::InternalError(format!(
            "Export failed with status {}",
            status
        )));
    }

    let mut batch_docs = Vec::with_capacity(1000);
    let mut total_copied = 0;
    let mut line_buffer = String::new();

    // Stream processing
    while let Ok(Some(chunk)) = resp.chunk().await {
        // Append chunk to buffer
        let chunk_str = String::from_utf8_lossy(&chunk);
        line_buffer.push_str(&chunk_str);

        // Process lines
        while let Some(pos) = line_buffer.find('\n') {
            let line: String = line_buffer.drain(..pos + 1).collect();
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            if let Ok(mut doc) = serde_json::from_str::<serde_json::Value>(line) {
                // Check if blob chunk
                let is_blob_chunk = doc
                    .get("_type")
                    .and_then(|t| t.as_str())
                    .map(|t| t == "blob_chunk")
                    .unwrap_or(false);

                if is_blob_chunk {
                    // Import Chunk immediately
                    if let (Some(key), Some(index), Some(data_b64)) = (
                        doc.get("_doc_key").and_then(|s| s.as_str()),
                        doc.get("_chunk_index").and_then(|n| n.as_u64()),
                        doc.get("_blob_data").and_then(|s| s.as_str()),
                    ) {
                        if let Ok(data) = general_purpose::STANDARD.decode(data_b64) {
                            if let Err(e) = coll.put_blob_chunk(key, index as u32, &data) {
                                tracing::error!(
                                    "HEAL: Failed to write chunk {} for {}: {}",
                                    index,
                                    key,
                                    e
                                );
                            }
                        }
                    }
                } else {
                    // Clean metadata (same as import)
                    if let Some(obj) = doc.as_object_mut() {
                        obj.remove("_database");
                        obj.remove("_collection");
                        obj.remove("_shardConfig");
                    }

                    // Prepare for batch upsert
                    let key = doc
                        .get("_key")
                        .and_then(|k| k.as_str())
                        .unwrap_or("")
                        .to_string();
                    if !key.is_empty() {
                        batch_docs.push((key, doc));
                    }
                }
            }
        }

        // Flush Batch if full
        if batch_docs.len() >= 1000 {
            let count = batch_docs.len();
            let batch_to_insert: Vec<(String, serde_json::Value)> = std::mem::take(&mut batch_docs);
            if let Err(e) = coll.upsert_batch(batch_to_insert) {
                tracing::error!("HEAL: Batch upsert failed: {}", e);
            } else {
                total_copied += count;
            }
        }
    }

    // Final Flush
    if !batch_docs.is_empty() {
        let count = batch_docs.len();
        if let Err(e) = coll.upsert_batch(batch_docs) {
            tracing::error!("HEAL: Final batch upsert failed: {}", e);
        } else {
            total_copied += count;
        }
    }

    tracing::info!(
        "HEAL: Copied {} docs (and associated chunks) to {}/{}",
        total_copied,
        database,
        physical_coll
    );
    Ok(total_copied)
}

/// Heal shards by creating new replicas when nodes are unhealthy
/// This maintains the replication factor when nodes fail
pub async fn heal_shards(
    storage: &Arc<StorageEngine>,
    cluster_manager: &Option<Arc<ClusterManager>>,
    shard_tables: &RwLock<HashMap<String, ShardTable>>,
    recently_failed_nodes: &RwLock<HashMap<String, std::time::Instant>>,
    is_rebalancing: &AtomicBool,
    my_node_id: &str,
    cluster_secret: &str,
) -> Result<usize, DbError> {
    // Skip healing if rebalancing is in progress to prevent data duplication
    if is_rebalancing.load(Ordering::SeqCst) {
        tracing::debug!("HEAL: Skipping - rebalancing in progress");
        return Ok(0);
    }

    let mgr = match cluster_manager {
        Some(m) => m,
        None => return Ok(0), // No cluster manager, nothing to heal
    };

    let healthy_nodes = mgr.get_healthy_nodes();
    if healthy_nodes.is_empty() {
        return Ok(0);
    }

    let mut healed_count = 0usize;

    tracing::debug!(
        "HEAL: Starting shard healing check. Healthy nodes: {:?}",
        healthy_nodes
    );

    // Get all shard tables
    let tables: Vec<(String, ShardTable)> = {
        let guard = shard_tables
            .read()
            .map_err(|_| DbError::InternalError("Lock poisoned".to_string()))?;
        guard.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };

    tracing::debug!("HEAL: Checking {} shard tables", tables.len());

    for (key, table) in &tables {
        let parts: Vec<&str> = key.split('.').collect();
        if parts.len() != 2 {
            continue;
        }
        let (database, collection) = (parts[0], parts[1]);

        // Get collection shard config for replication factor
        let replication_factor = if let Ok(db) = storage.get_database(database) {
            if let Ok(coll) = db.get_collection(collection) {
                coll.get_shard_config()
                    .map(|c| c.replication_factor)
                    .unwrap_or(1)
            } else {
                1
            }
        } else {
            1
        };

        for (shard_id, assignment) in &table.assignments {
            // Check if primary is unhealthy
            let primary_healthy = healthy_nodes.contains(&assignment.primary_node);

            // Count healthy replicas
            let healthy_replicas: Vec<&String> = assignment
                .replica_nodes
                .iter()
                .filter(|n| healthy_nodes.contains(*n))
                .collect();

            // Need (replication_factor - 1) replicas (primary is 1, replicas are the rest)
            let needed_replicas = (replication_factor as usize).saturating_sub(1);
            let current_replicas = healthy_replicas.len();

            tracing::debug!(
                "HEAL: Shard {}/{}/s{}: primary={} (healthy={}), replicas={:?} (healthy={:?}), needed={}, current={}",
                database, collection, shard_id,
                assignment.primary_node, primary_healthy,
                assignment.replica_nodes, healthy_replicas,
                needed_replicas, current_replicas
            );

            if !primary_healthy || current_replicas < needed_replicas {
                // Find a healthy node that doesn't already have this shard
                let nodes_with_shard: std::collections::HashSet<&String> = {
                    let mut set = std::collections::HashSet::new();
                    // Only add primary if it's healthy
                    if primary_healthy {
                        set.insert(&assignment.primary_node);
                    }
                    // Only add replicas that are healthy
                    for replica in &assignment.replica_nodes {
                        if healthy_nodes.contains(replica) {
                            set.insert(replica);
                        }
                    }
                    set
                };

                let available_nodes: Vec<&String> = healthy_nodes
                    .iter()
                    .filter(|n| !nodes_with_shard.contains(*n))
                    .collect();

                if available_nodes.is_empty() {
                    tracing::debug!("HEAL: No available nodes to heal shard {}/{}/s{} (all nodes already have this shard assigned)",
                        database, collection, shard_id);
                    continue;
                }

                // Pick a node (round-robin based on shard_id for distribution)
                let target_node =
                    available_nodes[*shard_id as usize % available_nodes.len()].clone();

                // Find a source node (healthy primary or replica)
                // Prefer nodes that haven't recently failed to avoid stale data
                let source_node = if primary_healthy
                    && !was_recently_failed(recently_failed_nodes, &assignment.primary_node)
                {
                    assignment.primary_node.clone()
                } else if let Some(replica) = healthy_replicas
                    .iter()
                    .find(|r| !was_recently_failed(recently_failed_nodes, r))
                {
                    (*replica).clone()
                } else if primary_healthy {
                    // Fallback to primary even if recently failed (better than no source)
                    tracing::warn!(
                        "HEAL: Using recently failed primary {} as source for {}/{}/s{}",
                        assignment.primary_node,
                        database,
                        collection,
                        shard_id
                    );
                    assignment.primary_node.clone()
                } else if let Some(replica) = healthy_replicas.first() {
                    // Fallback to any healthy replica
                    tracing::warn!(
                        "HEAL: Using recently failed replica {} as source for {}/{}/s{}",
                        replica,
                        database,
                        collection,
                        shard_id
                    );
                    (*replica).clone()
                } else {
                    tracing::warn!("HEAL: Skipping shard {}/{}/s{} - no suitable source available (all candidates recently failed or unhealthy)",
                        database, collection, shard_id);
                    continue;
                };

                tracing::info!(
                    "HEAL: Creating replica for shard {}/{}/s{} on {} (source: {})",
                    database,
                    collection,
                    shard_id,
                    target_node,
                    source_node
                );

                // If target is us, copy data from source
                // If target is another node, tell that node to copy from source
                let physical_coll = format!("{}_s{}", collection, shard_id);

                if target_node == my_node_id {
                    // We delegate the "Do I need to copy?" check to the copy function itself
                    if let Err(e) = copy_shard_from_source(
                        storage,
                        cluster_manager,
                        cluster_secret,
                        database,
                        &physical_coll,
                        &source_node,
                    )
                    .await
                    {
                        tracing::error!("HEAL: Failed to copy shard locally: {}", e);
                        continue;
                    }
                } else {
                    // Tell target node to copy from source
                    if let Some(target_addr) = mgr.get_node_api_address(&target_node) {
                        let url = format!(
                            "http://{}/_api/database/{}/collection/{}/_copy_shard",
                            target_addr, database, physical_coll
                        );

                        let source_addr =
                            mgr.get_node_api_address(&source_node).unwrap_or_default();

                        let client = reqwest::Client::new();
                        let res = client
                            .post(&url)
                            .header("X-Cluster-Secret", cluster_secret)
                            .header("X-Shard-Direct", "true") // Required for auth bypass
                            .json(&serde_json::json!({ "source_address": source_addr }))
                            .timeout(std::time::Duration::from_secs(60))
                            .send()
                            .await;

                        if let Err(e) = res {
                            tracing::error!(
                                "HEAL: Failed to trigger copy on {}: {}",
                                target_node,
                                e
                            );
                            continue;
                        }
                    }
                }

                // Update shard table with new replica
                {
                    let mut tables = shard_tables
                        .write()
                        .map_err(|_| DbError::InternalError("Lock poisoned".to_string()))?;
                    if let Some(table) = tables.get_mut(key) {
                        if let Some(assignment) = table.assignments.get_mut(shard_id) {
                            if !assignment.replica_nodes.contains(&target_node) {
                                assignment.replica_nodes.push(target_node.clone());
                                healed_count += 1;
                                tracing::info!(
                                    "HEAL: Added {} as replica for shard {}",
                                    target_node,
                                    shard_id
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    if healed_count > 0 {
        tracing::info!("HEAL: Successfully healed {} shard replicas", healed_count);
    }

    // Additional check: Resync stale replicas AND primaries
    // If this node is a replica/primary but has significantly less data than the source, resync
    for (key, table) in &tables {
        let parts: Vec<&str> = key.split('.').collect();
        if parts.len() != 2 {
            continue;
        }
        let (database, collection) = (parts[0], parts[1]);

        for (shard_id, assignment) in &table.assignments {
            // Check if this node is involved in this shard (replica OR primary)
            let is_replica = assignment.replica_nodes.contains(&my_node_id.to_string());
            let is_primary = assignment.primary_node == my_node_id;

            if !is_replica && !is_primary {
                continue;
            }

            tracing::debug!(
                "HEAL: Node {} is {} for {}_s{}",
                my_node_id,
                if is_primary { "PRIMARY" } else { "REPLICA" },
                collection,
                shard_id
            );

            // Get local document count
            let physical_coll = format!("{}_s{}", collection, shard_id);
            let local_count = if let Ok(db) = storage.get_database(database) {
                if let Ok(coll) = db.get_collection(&physical_coll) {
                    coll.count()
                } else {
                    0
                }
            } else {
                0
            };

            // Find a healthy source node to sync from
            // If we're a replica, use the primary. If we're the primary, use a healthy replica.
            let source_node = if is_primary {
                // We're primary - find a healthy replica to sync from
                assignment
                    .replica_nodes
                    .iter()
                    .find(|r| healthy_nodes.contains(*r))
                    .cloned()
            } else {
                // We're a replica - use the primary if healthy
                if healthy_nodes.contains(&assignment.primary_node) {
                    Some(assignment.primary_node.clone())
                } else {
                    None
                }
            };

            let source_node = match source_node {
                Some(n) => n,
                None => continue, // No healthy source available
            };

            // Get source document count
            let source_count = if let Some(source_addr) = mgr.get_node_api_address(&source_node) {
                let url = format!(
                    "http://{}/_api/database/{}/collection/{}/count",
                    source_addr, database, &physical_coll
                );
                let client = reqwest::Client::new();

                match client
                    .get(&url)
                    .header("X-Cluster-Secret", cluster_secret)
                    .header("X-Shard-Direct", "true")
                    .timeout(std::time::Duration::from_secs(5))
                    .send()
                    .await
                {
                    Ok(res) if res.status().is_success() => {
                        if let Ok(body) = res.json::<serde_json::Value>().await {
                            body.get("count").and_then(|c| c.as_u64()).unwrap_or(0) as usize
                        } else {
                            0
                        }
                    }
                    Ok(res) => {
                        tracing::warn!(
                            "HEAL: Count request failed for {}_s{} from {}: status {}",
                            collection,
                            shard_id,
                            source_node,
                            res.status()
                        );
                        0
                    }
                    Err(e) => {
                        tracing::warn!(
                            "HEAL: Count request error for {}_s{} from {}: {}",
                            collection,
                            shard_id,
                            source_node,
                            e
                        );
                        0
                    }
                }
            } else {
                0
            };

            // Log the count comparison for debugging
            tracing::debug!(
                "HEAL: {}_s{}: local_count={}, source_count={}, source_node={}",
                collection,
                shard_id,
                local_count,
                source_count,
                source_node
            );

            // If local has significantly fewer docs (>10% difference OR local is 0 with source > 0), resync
            let local_behind = source_count.saturating_sub(local_count);
            let local_ahead = local_count.saturating_sub(source_count);
            let threshold = if local_count == 0 && source_count > 0 {
                1 // If we have 0 docs and source has any, always sync
            } else {
                std::cmp::max(source_count / 10, 100)
            };

            // Case 1: Local is behind source (missing docs) - copy from source
            if local_behind >= threshold && source_count > 0 {
                tracing::warn!(
                    "HEAL: Stale {} detected for {}_s{}: local={}, source={}, resyncing (behind)",
                    if is_primary { "primary" } else { "replica" },
                    collection,
                    shard_id,
                    local_count,
                    source_count
                );

                // Resync by copying all data from source
                if let Err(e) = copy_shard_from_source(
                    storage,
                    cluster_manager,
                    cluster_secret,
                    database,
                    &physical_coll,
                    &source_node,
                )
                .await
                {
                    tracing::error!(
                        "HEAL: Failed to resync stale {}_s{}: {}",
                        collection,
                        shard_id,
                        e
                    );
                } else {
                    healed_count += 1;
                    tracing::info!(
                        "HEAL: Resynced stale {}_s{} from {}",
                        collection,
                        shard_id,
                        source_node
                    );
                }
            }
            // Case 2: Local REPLICA is ahead of PRIMARY (has stale data) - truncate and resync
            else if !is_primary && local_ahead > 0 && source_count > 0 {
                tracing::warn!(
                    "HEAL: Replica {}_s{} has MORE docs than primary: local={}, source={}. Truncating and resyncing.",
                    collection, shard_id, local_count, source_count
                );

                // Truncate local shard
                if let Ok(db) = storage.get_database(database) {
                    if let Ok(coll) = db.get_collection(&physical_coll) {
                        let _ = coll.truncate();
                    }
                }

                // Resync from primary
                if let Err(e) = copy_shard_from_source(
                    storage,
                    cluster_manager,
                    cluster_secret,
                    database,
                    &physical_coll,
                    &source_node,
                )
                .await
                {
                    tracing::error!(
                        "HEAL: Failed to resync replica {}_s{}: {}",
                        collection,
                        shard_id,
                        e
                    );
                } else {
                    healed_count += 1;
                    tracing::info!(
                        "HEAL: Resynced oversized replica {}_s{} from primary {}",
                        collection,
                        shard_id,
                        source_node
                    );
                }
            }
        }
    }

    Ok(healed_count)
}
