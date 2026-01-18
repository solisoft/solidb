//! Shard rebalancing operations
//!
//! This module handles shard rebalancing across cluster nodes.

#![allow(clippy::too_many_arguments, clippy::type_complexity, clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, RwLock,
};

use crate::cluster::manager::ClusterManager;
use crate::sharding::coordinator::{CollectionShardConfig, ShardTable};
use crate::sharding::migration::BatchSender;
use crate::storage::StorageEngine;
use crate::DbError;

/// Check if we recently completed resharding (to avoid aggressive healing)
pub fn check_recent_resharding(
    is_rebalancing: &AtomicBool,
    last_reshard_time: &RwLock<Option<std::time::Instant>>,
) -> bool {
    // Check if we're currently rebalancing
    if is_rebalancing.load(Ordering::SeqCst) {
        return true;
    }

    // Check if resharding completed within the last 10 seconds
    // This gives the cluster time to stabilize before healing runs
    if let Ok(last_time) = last_reshard_time.read() {
        if let Some(instant) = *last_time {
            let elapsed = instant.elapsed();
            if elapsed.as_secs() < 10 {
                tracing::debug!("check_recent_resharding: resharding completed {}s ago, still in stabilization period", elapsed.as_secs());
                return true;
            }
        }
    }

    false
}

/// Record that resharding has completed - used to prevent aggressive healing
pub fn mark_reshard_completed(last_reshard_time: &RwLock<Option<std::time::Instant>>) {
    if let Ok(mut last_time) = last_reshard_time.write() {
        *last_time = Some(std::time::Instant::now());
        tracing::info!("RESHARD: Marked resharding as completed, delaying healing for 10 seconds");
    }
}

/// Helper struct to implement BatchSender for shard rebalancing
struct RebalanceBatchSender<'a> {
    storage: &'a Arc<StorageEngine>,
    cluster_manager: &'a Option<Arc<ClusterManager>>,
    is_rebalancing: &'a AtomicBool,
}

#[async_trait::async_trait]
impl<'a> BatchSender for RebalanceBatchSender<'a> {
    async fn send_batch(
        &self,
        db_name: &str,
        coll_name: &str,
        config: &CollectionShardConfig,
        batch: Vec<(String, serde_json::Value)>,
    ) -> Result<Vec<String>, String> {
        // Simple implementation that routes to correct shard and upserts
        let table = {
            let tables = self
                .storage
                .get_database(db_name)
                .map_err(|e| e.to_string())?
                .get_collection(coll_name)
                .map_err(|e| e.to_string())?
                .get_stored_shard_table()
                .ok_or("Shard table not found".to_string())?;
            tables
        };

        let local_id = if let Some(mgr) = &self.cluster_manager {
            mgr.local_node_id()
        } else {
            "local".to_string()
        };

        let mut successful_keys: Vec<String> = Vec::new();

        for (key, doc) in batch {
            let shard_key_value = if config.shard_key == "_key" {
                key.clone()
            } else {
                doc.get(&config.shard_key)
                    .and_then(|v| v.as_str())
                    .unwrap_or(&key)
                    .to_string()
            };

            let shard_id =
                crate::sharding::router::ShardRouter::route(&shard_key_value, config.num_shards);

            if let Some(assignment) = table.assignments.get(&shard_id) {
                let primary_node = &assignment.primary_node;

                if primary_node == &local_id || primary_node == "local" {
                    // Local upsert
                    let physical_coll = format!("{}_s{}", coll_name, shard_id);
                    let db = self
                        .storage
                        .get_database(db_name)
                        .map_err(|e| e.to_string())?;
                    let coll = db
                        .get_collection(&physical_coll)
                        .map_err(|e| e.to_string())?;

                    coll.upsert_batch(vec![(key.clone(), doc)])
                        .map_err(|e| e.to_string())?;

                    successful_keys.push(key);
                }
                // Remote operations would require more complex handling
            }
        }

        Ok(successful_keys)
    }

    async fn should_pause_resharding(&self) -> bool {
        // Check if we're still rebalancing (prevents recursive resharding)
        self.is_rebalancing.load(Ordering::SeqCst)
    }
}

/// Broadcast reshard requests for removed shards to all nodes
pub async fn broadcast_reshard_removed_shards(
    cluster_manager: &Option<Arc<ClusterManager>>,
    cluster_secret: &str,
    db_name: &str,
    coll_name: &str,
    old_shards: u16,
    new_shards: u16,
) -> Result<(), DbError> {
    let mgr = match cluster_manager {
        Some(m) => m,
        None => return Ok(()), // Single node, no broadcast needed
    };

    let client = reqwest::Client::new();

    // For each removed shard, broadcast reshard request to all nodes
    for removed_shard_id in new_shards..old_shards {
        tracing::info!(
            "BROADCAST: Sending reshard requests for removed shard {}_s{} to all nodes",
            coll_name,
            removed_shard_id
        );

        for member in mgr.state().get_all_members() {
            let node_id = &member.node.id;
            let addr = &member.node.api_address;

            let request = serde_json::json!({
                "database": db_name,
                "collection": coll_name,
                "old_shards": old_shards,
                "new_shards": new_shards,
                "removed_shard_id": removed_shard_id
            });

            let url = format!("http://{}/_api/cluster/reshard", addr);

            match tokio::time::timeout(
                std::time::Duration::from_secs(30),
                client
                    .post(&url)
                    .header("X-Cluster-Secret", cluster_secret)
                    .json(&request)
                    .send(),
            )
            .await
            {
                Ok(Ok(response)) => {
                    if response.status().is_success() {
                        tracing::debug!(
                            "BROADCAST: Successfully sent reshard request to {} for shard {}",
                            node_id,
                            removed_shard_id
                        );
                    } else {
                        tracing::warn!(
                            "BROADCAST: Failed to send reshard request to {}: status {}",
                            node_id,
                            response.status()
                        );
                    }
                }
                Ok(Err(e)) => {
                    tracing::error!(
                        "BROADCAST: Failed to send reshard request to {}: {}",
                        node_id,
                        e
                    );
                }
                Err(_) => {
                    tracing::error!(
                        "BROADCAST: Timeout sending reshard request to {} for shard {}",
                        node_id,
                        removed_shard_id
                    );
                }
            }
        }
    }

    Ok(())
}

/// Rebalance shards across healthy nodes
///
/// This recalculates shard assignments based on current active nodes
/// and redistributes shards to maintain equal distribution.
pub async fn rebalance(
    storage: &Arc<StorageEngine>,
    cluster_manager: &Option<Arc<ClusterManager>>,
    shard_tables: &RwLock<HashMap<String, ShardTable>>,
    is_rebalancing: &AtomicBool,
    last_reshard_time: &RwLock<Option<std::time::Instant>>,
    my_node_id: &str,
    cluster_secret: &str,
) -> Result<(), DbError> {
    // Prevent concurrent rebalancing operations which can cause deadlocks
    if is_rebalancing.load(Ordering::SeqCst) {
        tracing::warn!("REBALANCE: Another rebalancing operation is already in progress, skipping");
        return Ok(());
    }
    is_rebalancing.store(true, Ordering::SeqCst);

    let initial_res = async {
        tracing::info!("Starting shard rebalance (New Implementation)");

        // DEADLOCK PREVENTION: Add coordination delay to prevent distributed deadlocks
        // When expanding shards (e.g., 3->4), all nodes start resharding simultaneously
        // and try to communicate with each other, potentially causing circular waits.
        // Nodes with higher IDs wait longer to allow lower-ID nodes to establish first.
        if cluster_manager.is_some() {
            let my_hash = my_node_id
                .bytes()
                .fold(0u32, |acc, b| acc.wrapping_add(b as u32));
            let coordination_delay_ms = (my_hash % 3000) as u64; // 0-3 second staggered delay

            tracing::info!(
                "REBALANCE: Waiting {}ms for coordination to prevent distributed deadlocks",
                coordination_delay_ms
            );
            tokio::time::sleep(tokio::time::Duration::from_millis(coordination_delay_ms)).await;
        }

        // Get current active node IDs
        let nodes = if let Some(ref mgr) = cluster_manager {
            let healthy = mgr.get_healthy_nodes();
            let mut node_ids: Vec<String> = healthy.into_iter().collect();
            node_ids.sort();
            node_ids
        } else {
            vec!["local".to_string()]
        };

        if nodes.is_empty() {
            return Ok(());
        }

        // Iterate over all sharded collections
        let mut sharded_collections = Vec::new();
        for db_name in storage.list_databases() {
            if let Ok(db) = storage.get_database(&db_name) {
                for coll_name in db.list_collections() {
                    if coll_name.starts_with('_') || coll_name.contains("_s") {
                        continue;
                    }
                    if let Ok(coll) = db.get_collection(&coll_name) {
                        if let Some(config) = coll.get_shard_config() {
                            sharded_collections.push((db_name.clone(), coll_name.clone(), config));
                        }
                    }
                }
            }
        }

        for (db_name, coll_name, config) in sharded_collections {
            let key = format!("{}.{}", db_name, coll_name);
            let mut needs_migration = false;
            let mut old_shards = config.num_shards;
            let mut old_assignments = HashMap::new();

            // Get current table to check for changes
            let current_table = {
                let tables = shard_tables.read().unwrap();
                tables.get(&key).cloned()
            };

            // 1. Detect Config vs Table mismatch (Expansion/Contraction)
            if let Some(ref table) = current_table {
                if table.num_shards != config.num_shards {
                    tracing::info!(
                        "REBALANCE: Config change detected for {}: {} -> {} shards",
                        key,
                        table.num_shards,
                        config.num_shards
                    );
                    old_shards = table.num_shards;
                    old_assignments = table.assignments.clone();
                    needs_migration = true;
                }
            }

            // 2. Compute NEW Assignments
            let previous_assignments = current_table.as_ref().map(|t| &t.assignments);

            let new_assignments = match crate::sharding::distribution::compute_assignments(
                &nodes,
                config.num_shards,
                config.replication_factor,
                previous_assignments,
            ) {
                Ok(a) => a,
                Err(e) => {
                    tracing::error!("Failed to compute assignments for {}: {}", key, e);
                    continue;
                }
            };

            // 3. Persist New Table
            let new_table = ShardTable {
                database: db_name.clone(),
                collection: coll_name.clone(),
                num_shards: config.num_shards,
                replication_factor: config.replication_factor,
                shard_key: config.shard_key.clone(),
                assignments: new_assignments.clone(),
            };

            // Save to storage
            if let Ok(db) = storage.get_database(&db_name) {
                if let Ok(coll) = db.get_collection(&coll_name) {
                    let _ = coll.set_shard_table(&new_table);
                }
            }
            // Update cache
            shard_tables
                .write()
                .unwrap()
                .insert(key.clone(), new_table.clone());

            // 4. Create Physical Shards (if expansion or new)
            if let Err(e) = create_shards(
                storage,
                cluster_manager,
                &db_name,
                &coll_name,
                &new_table,
                cluster_secret,
                my_node_id,
            )
            .await
            {
                tracing::error!("Failed to create shards for {}: {}", key, e);
            }

            // 5. Trigger Data Migration
            if needs_migration {
                tracing::info!(
                    "REBALANCE: Resharding {} from {} to {} shards",
                    key,
                    old_shards,
                    config.num_shards
                );

                // Handle migration - using simplified approach
                let current_assignments_map = new_table.assignments.clone();

                if let Err(_e) = crate::sharding::migration::reshard_collection(
                    storage,
                    &RebalanceBatchSender {
                        storage,
                        cluster_manager,
                        is_rebalancing,
                    },
                    &db_name,
                    &coll_name,
                    old_shards,
                    config.num_shards,
                    my_node_id,
                    &old_assignments,
                    &current_assignments_map,
                )
                .await
                {
                    tracing::error!("Resharding failed for {}: {}", key, _e);
                }
            }

            // Handle removed shards during contraction
            if old_shards > config.num_shards {
                if let Err(e) = broadcast_reshard_removed_shards(
                    cluster_manager,
                    cluster_secret,
                    &db_name,
                    &coll_name,
                    old_shards,
                    config.num_shards,
                )
                .await
                {
                    tracing::error!("Failed to broadcast reshard for removed shards: {}", e);
                }
            }
        }

        Ok::<(), DbError>(())
    }
    .await;

    // Mark resharding completed - this delays healing for 60 seconds to allow stabilization
    mark_reshard_completed(last_reshard_time);
    is_rebalancing.store(false, Ordering::SeqCst);
    initial_res
}

/// Create physical shards on all assigned nodes
async fn create_shards(
    storage: &Arc<StorageEngine>,
    cluster_manager: &Option<Arc<ClusterManager>>,
    database: &str,
    collection: &str,
    table: &ShardTable,
    cluster_secret: &str,
    my_node_id: &str,
) -> Result<(), String> {
    // Get collection type to propagate to shards
    let collection_type = if let Ok(db) = storage.get_database(database) {
        if let Ok(coll) = db.get_collection(collection) {
            Some(coll.get_type().to_string())
        } else {
            None
        }
    } else {
        None
    };

    let client = reqwest::Client::new();

    // Track created shards to avoid duplicates
    let mut created_shards = std::collections::HashSet::new();

    for (shard_id, assignment) in &table.assignments {
        let phys_name = format!("{}_s{}", collection, shard_id);

        // Check if we already processed this shard
        if created_shards.contains(&phys_name) {
            continue;
        }
        created_shards.insert(phys_name.clone());

        tracing::info!(
            "CREATE_SHARDS: Processing {} (is_local={}). Primary: {}",
            phys_name,
            assignment.primary_node == my_node_id,
            assignment.primary_node
        );

        // Unified loop for Primary AND Replicas
        let targets =
            std::iter::once(&assignment.primary_node).chain(assignment.replica_nodes.iter());

        for target_node in targets {
            let is_target_local = if let Some(mgr) = &cluster_manager {
                target_node == &mgr.local_node_id()
            } else {
                true
            };

            if is_target_local {
                // Create Local
                if let Ok(db) = storage.get_database(database) {
                    if db.get_collection(&phys_name).is_err() {
                        tracing::info!(
                            "CREATE_SHARDS: Creating local physical shard {} on {} type={:?}",
                            phys_name,
                            target_node,
                            collection_type
                        );
                        if let Err(e) =
                            db.create_collection(phys_name.clone(), collection_type.clone())
                        {
                            let msg = format!("Failed to create local shard {}: {}", phys_name, e);
                            tracing::error!("{}", msg);
                        }
                    }
                }
            } else {
                // Remote Create
                if let Some(mgr) = &cluster_manager {
                    if let Some(addr) = mgr.get_node_api_address(target_node) {
                        let url = format!("http://{}/_api/database/{}/collection", addr, database);
                        tracing::info!(
                            "CREATE_SHARDS: Remote creating {} at {} (url={}) type={:?}",
                            phys_name,
                            addr,
                            url,
                            collection_type
                        );
                        let body = serde_json::json!({
                            "name": phys_name,
                            "type": collection_type
                        });

                        match client
                            .post(&url)
                            .header("X-Shard-Direct", "true")
                            .header("X-Cluster-Secret", cluster_secret)
                            .json(&body)
                            .send()
                            .await
                        {
                            Ok(res) => {
                                if !res.status().is_success() {
                                    let status = res.status();
                                    if status.as_u16() == 409 {
                                        tracing::debug!(
                                            "CREATE_SHARDS: Remote shard {} already exists (409)",
                                            phys_name
                                        );
                                    } else {
                                        let err_text = res.text().await.unwrap_or_default();
                                        tracing::error!(
                                            "CREATE_SHARDS: Remote creation of {} failed: {} - {}",
                                            phys_name,
                                            status,
                                            err_text
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::error!("Request failed to {}: {}", addr, e);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}
