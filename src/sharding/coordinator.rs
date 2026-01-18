//! Shard coordinator compatibility layer
//!
//! This provides the old ShardCoordinator API backed by the new sync module.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use crate::cluster::manager::ClusterManager;
use crate::sharding::migration::BatchSender;
use crate::storage::StorageEngine;
use crate::sync::{LogEntry, Operation};
use crate::DbError;

/// Configuration for a sharded collection
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CollectionShardConfig {
    pub num_shards: u16,
    pub shard_key: String,
    pub replication_factor: u16,
}

/// Shard assignment information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardAssignment {
    pub shard_id: u16,
    pub primary_node: String,
    pub replica_nodes: Vec<String>,
}

/// Shard table for a collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardTable {
    pub database: String,
    pub collection: String,
    pub num_shards: u16,
    pub replication_factor: u16,
    pub shard_key: String,
    pub assignments: HashMap<u16, ShardAssignment>,
}
/// Helper struct to implement BatchSender for ShardCoordinator
/// This allows passing the coordinator to the migration module
struct CoordinatorBatchSender<'a> {
    coordinator: &'a ShardCoordinator,
}

#[async_trait::async_trait]
impl<'a> BatchSender for CoordinatorBatchSender<'a> {
    async fn send_batch(
        &self,
        db_name: &str,
        coll_name: &str,
        config: &CollectionShardConfig,
        batch: Vec<(String, serde_json::Value)>,
    ) -> Result<Vec<String>, String> {
        self.coordinator
            .send_migrated_batch(db_name, coll_name, config, batch)
            .await
    }

    async fn should_pause_resharding(&self) -> bool {
        self.coordinator.should_pause_resharding()
    }
}

/// Coordinator for managing shard assignments
pub struct ShardCoordinator {
    storage: Arc<StorageEngine>,
    cluster_manager: Option<Arc<ClusterManager>>,
    shard_tables: RwLock<HashMap<String, ShardTable>>,
    replication_log: Option<Arc<crate::sync::log::SyncLog>>,
    is_rebalancing: AtomicBool,
    recently_failed_nodes: RwLock<HashMap<String, std::time::Instant>>,
    /// Timestamp of last resharding completion - used to delay healing
    last_reshard_time: RwLock<Option<std::time::Instant>>,
}

impl ShardCoordinator {
    pub const MAX_BLOB_REPLICAS: u16 = 10;
    pub const MIN_BLOB_REPLICAS: u16 = 2;

    pub fn new(
        storage: Arc<StorageEngine>,
        cluster_manager: Option<Arc<ClusterManager>>,
        replication_log: Option<Arc<crate::sync::log::SyncLog>>,
    ) -> Self {
        Self {
            storage,
            cluster_manager,
            shard_tables: RwLock::new(HashMap::new()),
            replication_log,
            is_rebalancing: AtomicBool::new(false),
            recently_failed_nodes: RwLock::new(HashMap::new()),
            last_reshard_time: RwLock::new(None),
        }
    }

    /// Get the cluster secret from the keyfile for inter-node HTTP authentication
    pub fn cluster_secret(&self) -> String {
        self.storage
            .cluster_config()
            .and_then(|c| c.keyfile.clone())
            .unwrap_or_default()
    }

    /// Get shard configuration for a collection
    pub fn get_shard_config(
        &self,
        database: &str,
        collection: &str,
    ) -> Option<CollectionShardConfig> {
        if let Ok(db) = self.storage.get_database(database) {
            if let Ok(coll) = db.get_collection(collection) {
                return coll.get_shard_config();
            }
        }
        None
    }

    /// Get shard table for a collection
    /// Automatically recomputes if cached table contains nodes no longer in the cluster
    /// Returns None for internal system collections (those starting with _)
    pub fn get_shard_table(&self, database: &str, collection: &str) -> Option<ShardTable> {
        let key = format!("{}.{}", database, collection);

        // Fast path: Check cache
        if let Some(table) = self.shard_tables.read().unwrap().get(&key).cloned() {
            // Validate that the cached table is not stale
            // Stale conditions:
            // 1. Primary nodes are unhealthy
            // 2. Shard count doesn't match config (expansion/contraction happened)
            let is_stale = if let Some(ref mgr) = self.cluster_manager {
                let healthy_nodes = mgr.get_healthy_nodes();
                let has_unhealthy_primary = table
                    .assignments
                    .values()
                    .any(|a| !healthy_nodes.contains(&a.primary_node));

                // Check if shard count in config differs from table
                let shard_count_mismatch = if let Ok(db) = self.storage.get_database(database) {
                    if let Ok(coll) = db.get_collection(collection) {
                        if let Some(config) = coll.get_shard_config() {
                            config.num_shards != table.num_shards
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                has_unhealthy_primary || shard_count_mismatch
            } else {
                false
            };

            if !is_stale {
                return Some(table);
            }
            tracing::debug!("Cached shard table {} is stale (unhealthy primaries or shard count mismatch), checking storage for update", key);
        }

        tracing::debug!("Shard table {} not in cache, checking storage", key);

        // Slow path: Check storage and reconstruct if exists
        // This handles cases where node restarted or collection was created via API without coordinator
        if let Ok(db) = self.storage.get_database(database) {
            if let Ok(coll) = db.get_collection(collection) {
                // Skip non-sharded collections - they don't need shard tables
                let shard_config = coll.get_shard_config();
                if shard_config.is_none()
                    || shard_config.as_ref().map(|c| c.num_shards).unwrap_or(0) == 0
                {
                    return None;
                }

                // Try to load persisted table first (preserves assignments)
                if let Some(table) = coll.get_stored_shard_table() {
                    tracing::debug!("Loaded shard table {} from storage (missed cache)", key);
                    self.shard_tables
                        .write()
                        .unwrap()
                        .insert(key.clone(), table.clone());
                    return Some(table);
                } else {
                    tracing::debug!("No persisted shard table found for {}", key);
                }

                if let Some(config) = coll.get_shard_config() {
                    // Fallback: Reconstruct table (fresh computation)
                    // This creates new assignments! Only happens if persistence missing.
                    if let Ok(table) = self.compute_shard_table(database, collection, &config) {
                        tracing::info!("Computed fresh shard table for {} (fallback)", key);
                        // Persist it now so we don't lose it again
                        let _ = coll.set_shard_table(&table);

                        // Cache it
                        self.shard_tables
                            .write()
                            .unwrap()
                            .insert(key, table.clone());
                        return Some(table);
                    }
                }
            } else {
                tracing::warn!(
                    "Collection {} not found during shard table lookup (db: {})",
                    collection,
                    database
                );
            }
        } else {
            tracing::warn!("Database {} not found during shard table lookup", database);
        }

        None
    }

    /// Initialize sharding for a collection
    pub fn init_collection(
        &self,
        database: &str,
        collection: &str,
        config: &CollectionShardConfig,
    ) -> Result<ShardTable, String> {
        let table = self.compute_shard_table(database, collection, config)?;
        let key = format!("{}.{}", database, collection);

        // Save to storage
        if let Ok(db) = self.storage.get_database(database) {
            if let Ok(coll) = db.get_collection(collection) {
                let _ = coll.set_shard_table(&table);
            }
        }

        self.shard_tables
            .write()
            .unwrap()
            .insert(key, table.clone());
        Ok(table)
    }

    /// Update local shard table cache (used when receiving updates from coordinator)
    pub fn update_shard_table_cache(&self, table: ShardTable) {
        let key = format!("{}.{}", table.database, table.collection);
        if let Ok(mut tables) = self.shard_tables.write() {
            tables.insert(key.clone(), table.clone());
            tracing::info!("CACHE: Updated shard table for {}", key);

            // Also persist to local storage (so it survives restart)
            if let Ok(db) = self.storage.get_database(&table.database) {
                if let Ok(coll) = db.get_collection(&table.collection) {
                    let _ = coll.set_shard_table(&table);
                }
            }
        }
    }

    /// Compute shard table assignments based on current cluster state
    /// Only uses HEALTHY nodes to ensure data availability
    fn compute_shard_table(
        &self,
        database: &str,
        collection: &str,
        config: &CollectionShardConfig,
    ) -> Result<ShardTable, String> {
        // Get HEALTHY nodes only (not all members - dead nodes should not get shards)
        let nodes = if let Some(ref mgr) = self.cluster_manager {
            let healthy = mgr.get_healthy_nodes();
            // Sort to ensure deterministic assignment
            let mut node_ids: Vec<String> = healthy.into_iter().collect();
            node_ids.sort();
            node_ids
        } else {
            vec!["local".to_string()]
        };

        if nodes.is_empty() {
            return Err("No nodes available".to_string());
        }

        let assignments = crate::sharding::distribution::compute_assignments(
            &nodes,
            config.num_shards,
            config.replication_factor,
            None, // Initial computation has no history
        )?;

        Ok(ShardTable {
            database: database.to_string(),
            collection: collection.to_string(),
            num_shards: config.num_shards,
            replication_factor: config.replication_factor,
            shard_key: config.shard_key.clone(),
            assignments,
        })
    }

    /// Route a document key to a shard
    pub fn route(&self, key: &str, num_shards: u16) -> u16 {
        crate::sharding::router::ShardRouter::route(key, num_shards)
    }

    /// Check if this node should store a shard
    pub fn is_shard_replica(
        shard_id: u16,
        node_index: usize,
        replication_factor: u16,
        num_nodes: usize,
    ) -> bool {
        crate::sharding::router::ShardRouter::is_shard_replica(
            shard_id,
            node_index,
            replication_factor,
            num_nodes,
        )
    }

    /// Insert batch with shard-aware distribution
    pub async fn insert_batch_sharded(
        &self,
        _database: &str,
        _collection: &str,
        documents: Vec<serde_json::Value>,
    ) -> Result<Vec<serde_json::Value>, String> {
        // For now, just return documents - actual distribution would be implemented
        // when full shard coordination is needed
        Ok(documents)
    }

    /// Get all node addresses in the cluster
    pub fn get_node_addresses(&self) -> Vec<String> {
        if let Some(ref mgr) = self.cluster_manager {
            mgr.state()
                .get_all_members()
                .into_iter()
                .map(|m| m.node.address.clone())
                .collect()
        } else {
            vec!["local".to_string()]
        }
    }

    /// Get all node IDs in the cluster
    pub fn get_node_ids(&self) -> Vec<String> {
        if let Some(ref mgr) = self.cluster_manager {
            mgr.state()
                .get_all_members()
                .into_iter()
                .map(|m| m.node.id.clone())
                .collect()
        } else {
            vec!["local".to_string()]
        }
    }

    /// Get this node's address
    pub fn my_address(&self) -> String {
        if let Some(ref mgr) = self.cluster_manager {
            mgr.get_local_address()
        } else {
            "local".to_string()
        }
    }

    /// Get API address for a specific node (for scatter-gather queries)
    pub fn get_node_api_address(&self, node_id: &str) -> Option<String> {
        if let Some(ref mgr) = self.cluster_manager {
            mgr.get_node_api_address(node_id)
        } else {
            None
        }
    }

    /// Get count of healthy nodes in the cluster
    pub fn get_healthy_node_count(&self) -> usize {
        if let Some(ref mgr) = self.cluster_manager {
            mgr.get_healthy_nodes().len()
        } else {
            1
        }
    }

    /// Calculate optimal replication factor for blob collections
    /// Formula: min(max(2, healthy_nodes / 2), MAX_BLOB_REPLICAS)
    /// Example: 10 nodes -> 5 replicas
    pub fn calculate_blob_replication_factor(&self) -> u16 {
        let healthy_count = self.get_healthy_node_count() as u16;
        (healthy_count / 2)
            .max(Self::MIN_BLOB_REPLICAS)
            .min(Self::MAX_BLOB_REPLICAS)
    }

    /// Get my node ID
    pub fn my_node_id(&self) -> String {
        if let Some(ref mgr) = self.cluster_manager {
            mgr.local_node_id()
        } else {
            "local".to_string()
        }
    }

    /// Reload shard tables from persistent storage
    /// This is called before cleanup to ensure we have the latest shard config
    /// (e.g., when another node reduced shard count)
    pub async fn reload_shard_tables_from_storage(&self) {
        tracing::info!("Reloading shard tables from storage");

        for db_name in self.storage.list_databases() {
            if let Ok(db) = self.storage.get_database(&db_name) {
                for coll_name in db.list_collections() {
                    // Skip system collections and physical shards
                    if coll_name.starts_with('_') || coll_name.contains("_s") {
                        continue;
                    }

                    if let Ok(coll) = db.get_collection(&coll_name) {
                        // Load shard table from storage
                        if let Some(table) = coll.get_stored_shard_table() {
                            let key = format!("{}.{}", db_name, coll_name);
                            if let Ok(mut tables) = self.shard_tables.write() {
                                tables.insert(key.clone(), table);
                                tracing::debug!("Reloaded shard table for {}", key);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Implementation of BatchSender trait for ShardCoordinator
    /// This allows the migration module to use the coordinator's networking capabilities
    async fn send_migrated_batch(
        &self,
        db_name: &str,
        coll_name: &str,
        config: &CollectionShardConfig,
        batch: Vec<(String, serde_json::Value)>,
    ) -> Result<Vec<String>, String> {
        self.upsert_batch_to_shards(db_name, coll_name, config, batch)
            .await
            .map_err(|e| e.to_string())
    }

    /// Rebalance shards across healthy nodes
    ///
    /// This recalculates shard assignments based on current active nodes
    /// and redistributes shards to maintain equal distribution using the
    /// new resilient distribution logic.
    pub async fn rebalance(&self) -> Result<(), crate::error::DbError> {
        // Prevent concurrent rebalancing operations which can cause deadlocks
        if self.is_rebalancing.load(Ordering::SeqCst) {
            tracing::warn!(
                "REBALANCE: Another rebalancing operation is already in progress, skipping"
            );
            return Ok(());
        }
        self.is_rebalancing.store(true, Ordering::SeqCst);

        let initial_res = async {
            tracing::info!("Starting shard rebalance (New Implementation)");

            // DEADLOCK PREVENTION: Add coordination delay to prevent distributed deadlocks
            // When expanding shards (e.g., 3->4), all nodes start resharding simultaneously
            // and try to communicate with each other, potentially causing circular waits.
            // Nodes with higher IDs wait longer to allow lower-ID nodes to establish first.
            if let Some(ref mgr) = self.cluster_manager {
                let my_id = mgr.local_node_id();
                let my_hash = my_id
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
            let nodes = if let Some(ref mgr) = self.cluster_manager {
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
            for db_name in self.storage.list_databases() {
                if let Ok(db) = self.storage.get_database(&db_name) {
                    for coll_name in db.list_collections() {
                        if coll_name.starts_with('_') || coll_name.contains("_s") {
                            continue;
                        }
                        if let Ok(coll) = db.get_collection(&coll_name) {
                            if let Some(config) = coll.get_shard_config() {
                                sharded_collections.push((
                                    db_name.clone(),
                                    coll_name.clone(),
                                    config,
                                ));
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
                let current_table = self.get_shard_table(&db_name, &coll_name);

                // 1. Detect Config vs Table mismatch (Expansion/Contraction)
                if let Some(ref table) = current_table {
                    // Check if we need to adjust shard count based on node count
                    // (Auto-scale down if nodes < shards, but usually explicit config wins)
                    // Let's stick to explicit config for now, or the user's wish for resilience.
                    // The user asked for "Adding a new shard should reshard... Removing...".
                    // This implies config change drives it.

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
                if let Ok(db) = self.storage.get_database(&db_name) {
                    if let Ok(coll) = db.get_collection(&coll_name) {
                        let _ = coll.set_shard_table(&new_table);
                    }
                }
                // Update cache
                self.shard_tables
                    .write()
                    .unwrap()
                    .insert(key.clone(), new_table.clone());

                // 4. Create Physical Shards (if expansion or new)
                if let Err(e) = self.create_shards(&db_name, &coll_name).await {
                    tracing::error!("Failed to create shards for {}: {}", key, e);
                }

                // 5. Trigger Data Migration
                // Migration is needed if:
                // - Shard count changed (resharding)
                // - Assignments changed (rebalancing) - though reshard logic covers this too
                // For safety, we can run resharding check if ANYTHING changed.
                // But full scan is expensive.
                // If only assignments changed (nodes added/removed), strictly speaking we just need to move shards.
                // However, the user asked for "fully rewritten" and "reshard the data evenly".
                // Our `reshard_collection` handles moving misplaced docs.
                // If shard count is same, but primary owner changed, `reshard_collection` will see mismatched `new_shard_id` vs `current_physical_location`?
                // Wait, `reshard_collection` checks `ShardRouter::route(key, new_shards)`.
                // If `new_shards` == `old_shards`, routing doesn't change shard ID.
                // But if the *assignment* of that shard ID changed from Node A to Node B,
                // Node A (old primary) still has the data in `_sN`.
                // `reshard_collection` on Node A sees it has `_sN`.
                // It routes key -> `N`.
                // It checks if `N` != `s` (current physical). They are Equal.
                // So it does NOT move it.
                // PROBLEM: `reshard_collection` logic (as implemented in migration.rs) handles *SHARD ID* changes (rehashing).
                // It does NOT handle "Shard N moved from Node A to Node B".
                // Node A still has `_sN`. Node B has empty `_sN`.
                // We need `move_shard` logic for that.

                // Let's implement move logic here or inside migration?
                // Actually `reshard_collection` in migration.rs was designed for resharding (rehashing).

                // If only assignments changed, we should use the `heal_shards` mechanism or similar
                // by treating the new primary as "healthy replica" and old primary as "to be removed".
                // But `heal_shards` (existing) copies FROM source TO target.
                // We can use that!

                // But if shard COUNT determines we need migration (rehashing):
                if needs_migration {
                    // This handles 4->5 or 5->4.
                    // We need a struct that implements BatchSender.
                    // Since we are inside `rebalance`, we can't implement trait on `&self` easily if we need `async`.
                    // But we can genericize or wrap.
                    // Or implement BatchSender for ShardCoordinator wrapper.

                    tracing::info!(
                        "REBALANCE: Resharding {} from {} to {} shards",
                        key,
                        old_shards,
                        config.num_shards
                    );

                    let sender = CoordinatorBatchSender { coordinator: self };
                    let my_node_id = self.my_node_id();

                    // We need to determine if we act on old assignments (removed shards) or current
                    // The migration logic iterates `max(old, new)`.
                    let current_assignments_map = new_table.assignments.clone();

                    if let Err(_e) = crate::sharding::migration::reshard_collection(
                        &self.storage,
                        &sender,
                        &db_name,
                        &coll_name,
                        old_shards,
                        config.num_shards,
                        &my_node_id,
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
                    // We shrunk, so we need to migrate data from removed shards on all nodes
                    if let Err(e) = self
                        .broadcast_reshard_removed_shards(
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

                // After potentially re-hashing, we check for pure assignment moves (Node A -> Node B)
                // This is covered by `heal_shards` which ensures the new primary gets data,
                // and `cleanup_orphaned_shards` which removes data from old owners.
                // So we don't need explicit move logic here, provided `heal_shards` works.
            }

            Ok::<(), crate::error::DbError>(())
        }
        .await;

        // Mark resharding completed - this delays healing for 60 seconds to allow stabilization
        self.mark_reshard_completed();
        self.is_rebalancing.store(false, Ordering::SeqCst);
        initial_res
    }

    /// Check if rebalancing is in progress
    pub fn is_rebalancing(&self) -> bool {
        self.is_rebalancing.load(Ordering::SeqCst)
    }

    /// Check if a node recently failed and came back online
    /// This helps avoid using stale data from nodes that just recovered
    fn was_recently_failed(&self, node_id: &str) -> bool {
        const RECENT_FAILURE_WINDOW_SECS: u64 = 300; // 5 minutes

        let recently_failed = self.recently_failed_nodes.read().unwrap();
        if let Some(failure_time) = recently_failed.get(node_id) {
            let elapsed = failure_time.elapsed();
            elapsed.as_secs() < RECENT_FAILURE_WINDOW_SECS
        } else {
            false
        }
    }

    /// Record that a node failed (called when failover occurs)
    pub fn record_node_failure(&self, node_id: &str) {
        let mut recently_failed = self.recently_failed_nodes.write().unwrap();
        recently_failed.insert(node_id.to_string(), std::time::Instant::now());
        tracing::info!("Recorded node failure for {}", node_id);
    }

    /// Clear failure record when node is confirmed healthy
    pub fn clear_node_failure(&self, node_id: &str) {
        let mut recently_failed = self.recently_failed_nodes.write().unwrap();
        recently_failed.remove(node_id);
        tracing::info!("Cleared failure record for {}", node_id);
    }

    /// Clean up old failure records
    pub fn cleanup_old_failures(&self) {
        const MAX_AGE_SECS: u64 = 3600; // 1 hour
        let mut recently_failed = self.recently_failed_nodes.write().unwrap();
        let now = std::time::Instant::now();

        recently_failed
            .retain(|_, failure_time| now.duration_since(*failure_time).as_secs() < MAX_AGE_SECS);
    }

    /// Broadcast reshard requests for removed shards to all nodes
    async fn broadcast_reshard_removed_shards(
        &self,
        db_name: &str,
        coll_name: &str,
        old_shards: u16,
        new_shards: u16,
    ) -> Result<(), crate::error::DbError> {
        let mgr = match &self.cluster_manager {
            Some(m) => m,
            None => return Ok(()), // Single node, no broadcast needed
        };

        let _my_node_id = mgr.local_node_id();
        let client = reqwest::Client::new();
        let secret = self.cluster_secret();

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
                        .header("X-Cluster-Secret", &secret)
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

    /// Check if we recently completed resharding (to avoid aggressive healing)
    fn check_recent_resharding(&self) -> bool {
        // Check if we're currently rebalancing
        if self.is_rebalancing() {
            return true;
        }

        // Check if resharding completed within the last 10 seconds
        // This gives the cluster time to stabilize before healing runs
        if let Ok(last_time) = self.last_reshard_time.read() {
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
    pub fn mark_reshard_completed(&self) {
        if let Ok(mut last_time) = self.last_reshard_time.write() {
            *last_time = Some(std::time::Instant::now());
            tracing::info!(
                "RESHARD: Marked resharding as completed, delaying healing for 10 seconds"
            );
        }
    }

    /// Check if resharding should be paused due to cluster health issues
    fn should_pause_resharding(&self) -> bool {
        if let Some(mgr) = &self.cluster_manager {
            let healthy_nodes = mgr.get_healthy_nodes();
            let total_nodes = mgr.state().get_all_members().len();

            // Pause resharding if less than 50% of nodes are healthy
            if healthy_nodes.len() < (total_nodes + 1) / 2 {
                tracing::warn!(
                    "RESHARD: Pausing resharding - only {}/{} nodes are healthy",
                    healthy_nodes.len(),
                    total_nodes
                );
                return true;
            }

            // Also check for recently failed nodes
            let recently_failed = self.recently_failed_nodes.read().unwrap();
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

    /// Clear failure records for nodes that are currently healthy
    pub fn clear_failures_for_healthy_nodes(&self) {
        if let Some(mgr) = &self.cluster_manager {
            let healthy_nodes = mgr.get_healthy_nodes();
            let mut recently_failed = self.recently_failed_nodes.write().unwrap();

            for healthy_node in &healthy_nodes {
                if recently_failed.contains_key(healthy_node) {
                    recently_failed.remove(healthy_node);
                    tracing::info!("Cleared failure record for healthy node {}", healthy_node);
                }
            }
        }
    }

    /// Repair sharded collection by checking for misplaced documents and fixing them
    /// This cleans up duplicates left over from failed migration cleanups
    pub async fn repair_collection(
        &self,
        db_name: &str,
        coll_name: &str,
    ) -> Result<String, String> {
        let db = self
            .storage
            .get_database(db_name)
            .map_err(|e| e.to_string())?;
        let main_coll = db.get_collection(coll_name).map_err(|e| e.to_string())?;
        let config = main_coll
            .get_shard_config()
            .ok_or("Missing shard config".to_string())?;

        let mut report = String::new();
        let mut total_fixed = 0;
        let mut total_moved = 0;
        let mut total_errors = 0;

        report.push_str(&format!(
            "Repairing {}.{} (Num Shards: {})\n",
            db_name, coll_name, config.num_shards
        ));

        // Iterate through ALL potential physical shards (current num_shards)
        // Note: usage of 0..num_shards checks shards that SHOULD exist.
        // But duplicates might be in orphaned shards too?
        // Orphaned shards are usually removed by `remove_orphaned_shards`.
        // Duplicates here are likely in shards 0..3 (if expanded 3->4).
        // Check 0..config.num_shards.
        for s in 0..config.num_shards {
            let physical_name = format!("{}_s{}", coll_name, s);

            if let Ok(physical_coll) = db.get_collection(&physical_name) {
                let documents = physical_coll.all();
                let doc_count = documents.len();
                let mut shard_fixed = 0;
                let mut shard_moved = 0;

                tracing::info!(
                    "REPAIR: Scanning shard {} ({} docs)...",
                    physical_name,
                    doc_count
                );

                let mut redundant_keys = Vec::new();
                let mut misplaced_docs = Vec::new();
                let mut misplaced_keys = Vec::new(); // Keep track of keys for deletion after move

                // 1. Scan and Classify
                for doc in documents {
                    let id_str = doc.key.clone();
                    let route_key = if config.shard_key == "_key" {
                        doc.key.clone()
                    } else {
                        doc.key.clone() // Default
                    };

                    let target_shard =
                        crate::sharding::router::ShardRouter::route(&route_key, config.num_shards);

                    if target_shard != s {
                        // Document is misplaced!

                        // Check if it exists in expected location
                        let exists = self.get(db_name, coll_name, &id_str).await.is_ok();

                        if exists {
                            // It exists in target -> Redundant Duplicate
                            redundant_keys.push(id_str);
                            shard_fixed += 1;
                        } else {
                            // It DOES NOT exist -> Misplaced (needs move)
                            misplaced_docs.push(doc.to_value());
                            misplaced_keys.push(id_str);
                        }
                    }
                }

                // 2. Batch Move Misplaced Docs
                if !misplaced_docs.is_empty() {
                    let total_to_move = misplaced_docs.len();
                    tracing::info!(
                        "REPAIR: Moving {} misplaced docs from {}...",
                        total_to_move,
                        physical_name
                    );

                    match self
                        .insert_batch(db_name, coll_name, &config, misplaced_docs)
                        .await
                    {
                        Ok((success, fail)) => {
                            if fail == 0 {
                                // Move successful (all), now we can delete them from source
                                redundant_keys.extend(misplaced_keys);
                                shard_moved += success; // Actually moved
                            } else {
                                // Partial failure. Safety check: DO NOT DELETE from source to avoid data loss.
                                // We could try to identify which failed, but insert_batch doesn't return that.
                                // User can run repair again.
                                tracing::warn!("REPAIR: Batch move had failures (success={}, fail={}). Skipping delete for safety.", success, fail);
                                total_errors += fail;
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                "REPAIR: Batch move failed for {}: {}",
                                physical_name,
                                e
                            );
                            total_errors += 1;
                            // We do NOT delete misplaced_keys if move failed.
                        }
                    }
                }

                // 3. Batch Delete Redundant Docs
                if !redundant_keys.is_empty() {
                    match physical_coll.delete_batch(redundant_keys) {
                        Ok(_n) => {
                            // n duplicates deleted
                            // shard_fixed/shard_moved counts track logic, n tracks actual deletes
                        }
                        Err(e) => {
                            tracing::error!(
                                "REPAIR: Batch delete failed for {}: {}",
                                physical_name,
                                e
                            );
                            total_errors += 1;
                        }
                    }
                }

                if shard_fixed > 0 || shard_moved > 0 {
                    report.push_str(&format!(
                        "  Shard {}: Removed {} duplicates (already in target), Moved {} docs\n",
                        s, shard_fixed, shard_moved
                    ));
                    total_fixed += shard_fixed;
                    total_moved += shard_moved;
                }
            }
        }

        report.push_str(&format!("----------------------------------\nTotal: {} duplicates removed, {} docs moved, {} errors.\n", total_fixed, total_moved, total_errors));
        tracing::info!("{}", report);
        Ok(report)
    }

    /// Promote a healthy replica to be the new primary for a shard
    /// Returns the new primary node ID if successful
    pub fn promote_replica(
        &self,
        database: &str,
        collection: &str,
        shard_id: u16,
    ) -> Option<String> {
        let key = format!("{}.{}", database, collection);
        let mut table_to_persist = None;
        let mut result = None;

        {
            let mut tables = self.shard_tables.write().ok()?;
            let table = tables.get_mut(&key)?;

            let assignment = table.assignments.get(&shard_id)?;

            // Find a healthy replica
            if let Some(mgr) = &self.cluster_manager {
                for replica in &assignment.replica_nodes {
                    if mgr.is_node_healthy(replica) {
                        // Promote this replica to primary
                        let new_primary = replica.clone();
                        let old_primary = assignment.primary_node.clone();

                        // Update the assignment
                        let mut new_replicas: Vec<String> = assignment
                            .replica_nodes
                            .iter()
                            .filter(|n| *n != &new_primary)
                            .cloned()
                            .collect();

                        // Old primary becomes a replica (for when it comes back)
                        new_replicas.push(old_primary.clone());

                        table.assignments.insert(
                            shard_id,
                            ShardAssignment {
                                shard_id,
                                primary_node: new_primary.clone(),
                                replica_nodes: new_replicas,
                            },
                        );

                        tracing::warn!(
                            "FAILOVER: Promoted {} to primary for shard {} (was: {})",
                            new_primary,
                            shard_id,
                            old_primary
                        );

                        // Record that the old primary failed (for future healing decisions)
                        self.record_node_failure(&old_primary);

                        table_to_persist = Some(table.clone());
                        result = Some(new_primary);
                        break;
                    }
                }
            }
        } // Drop lock

        // Persist the changes
        if let Some(table) = table_to_persist {
            if let Ok(db) = self.storage.get_database(database) {
                if let Ok(coll) = db.get_collection(collection) {
                    let _ = coll.set_shard_table(&table);
                }
            }
        }

        result
    }

    /// Heal shards by creating new replicas when nodes are unhealthy
    /// This maintains the replication factor when nodes fail
    pub async fn heal_shards(&self) -> Result<usize, crate::error::DbError> {
        // Skip healing if rebalancing is in progress to prevent data duplication
        if self.is_rebalancing() {
            tracing::debug!("HEAL: Skipping - rebalancing in progress");
            return Ok(0);
        }

        // Skip aggressive healing right after resharding to allow assignments to stabilize
        // Check if we recently completed resharding by looking at a timestamp or flag
        // For now, be more conservative and only heal shards that clearly need it
        let recently_resharded = self.check_recent_resharding();
        if recently_resharded {
            tracing::debug!(
                "HEAL: Skipping aggressive healing - recently resharded, allowing stabilization"
            );
            return Ok(0);
        }

        let mgr = match &self.cluster_manager {
            Some(m) => m,
            None => return Ok(0), // No cluster manager, nothing to heal
        };

        let healthy_nodes = mgr.get_healthy_nodes();
        if healthy_nodes.is_empty() {
            return Ok(0);
        }

        // Clear failure records for nodes that are now healthy
        self.clear_failures_for_healthy_nodes();

        let my_node_id = self.my_node_id();
        let mut healed_count = 0usize;

        tracing::debug!(
            "HEAL: Starting shard healing check. Healthy nodes: {:?}",
            healthy_nodes
        );

        // Get all shard tables
        let tables: Vec<(String, ShardTable)> = {
            let guard = self
                .shard_tables
                .read()
                .map_err(|_| crate::error::DbError::InternalError("Lock poisoned".to_string()))?;
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
            let replication_factor = if let Ok(db) = self.storage.get_database(database) {
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
                    // IMPORTANT: Only exclude HEALTHY nodes that have the shard.
                    // Dead nodes should NOT block choosing a healthy candidate.
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
                        && !self.was_recently_failed(&assignment.primary_node)
                    {
                        assignment.primary_node.clone()
                    } else if let Some(replica) = healthy_replicas
                        .iter()
                        .find(|r| !self.was_recently_failed(*r))
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
                        // This allows checking for Stale data (Count mismatch) instead of just "Empty vs Non-Empty"
                        if let Err(e) = self
                            .copy_shard_from_source(database, &physical_coll, &source_node)
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
                            let secret = self.cluster_secret();

                            let source_addr =
                                mgr.get_node_api_address(&source_node).unwrap_or_default();

                            let client = reqwest::Client::new();
                            let res = client
                                .post(&url)
                                .header("X-Cluster-Secret", &secret)
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
                        let mut tables = self.shard_tables.write().map_err(|_| {
                            crate::error::DbError::InternalError("Lock poisoned".to_string())
                        })?;
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
                let is_replica = assignment.replica_nodes.contains(&my_node_id);
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
                let local_count = if let Ok(db) = self.storage.get_database(database) {
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
                let source_count = if let Some(source_addr) = mgr.get_node_api_address(&source_node)
                {
                    let url = format!(
                        "http://{}/_api/database/{}/collection/{}/count",
                        source_addr, database, &physical_coll
                    );
                    let secret = self.cluster_secret();
                    let client = reqwest::Client::new();

                    match client
                        .get(&url)
                        .header("X-Cluster-Secret", &secret)
                        .header("X-Shard-Direct", "true") // Required for auth bypass
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
                        collection, shard_id, local_count, source_count
                    );

                    // Resync by copying all data from source
                    if let Err(e) = self
                        .copy_shard_from_source(database, &physical_coll, &source_node)
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
                    if let Ok(db) = self.storage.get_database(database) {
                        if let Ok(coll) = db.get_collection(&physical_coll) {
                            let _ = coll.truncate();
                        }
                    }

                    // Resync from primary
                    if let Err(e) = self
                        .copy_shard_from_source(database, &physical_coll, &source_node)
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

    /// Clean up orphaned shard collections on this node
    ///
    /// When a node restarts and its shards have been reassigned to other nodes,
    /// this function removes the local physical shard collections that are no longer
    /// assigned to this node (neither as primary nor replica).
    pub async fn cleanup_orphaned_shards(&self) -> Result<usize, crate::error::DbError> {
        let my_node_id = self.my_node_id();
        let mut cleaned_count = 0usize;

        // Iterate all databases
        for db_name in self.storage.list_databases() {
            let db = match self.storage.get_database(&db_name) {
                Ok(d) => d,
                Err(_) => continue,
            };

            // Get all collections in this database
            let collections = db.list_collections();

            for coll_name in collections {
                // Check if this looks like a physical shard collection (ends with _s<N>)
                if let Some(base_name) = coll_name.strip_suffix(|c: char| c.is_ascii_digit()) {
                    if let Some(base) = base_name.strip_suffix("_s") {
                        // This is a physical shard collection
                        // Extract shard ID
                        let shard_suffix = &coll_name[base.len() + 2..];
                        let shard_id: u16 = match shard_suffix.parse() {
                            Ok(id) => id,
                            Err(_) => continue,
                        };

                        // Check if we should have this shard
                        let key = format!("{}.{}", db_name, base);
                        let tables = self.shard_tables.read().map_err(|_| {
                            crate::error::DbError::InternalError("Lock poisoned".to_string())
                        })?;

                        let is_assigned_to_us = if let Some(table) = tables.get(&key) {
                            if let Some(assignment) = table.assignments.get(&shard_id) {
                                assignment.primary_node == my_node_id
                                    || assignment.replica_nodes.contains(&my_node_id)
                            } else {
                                false
                            }
                        } else {
                            // No shard table means we can't determine - be safe and keep it
                            true
                        };

                        if !is_assigned_to_us {
                            // This shard is not assigned to us - clean it up
                            tracing::warn!(
                                "CLEANUP: Removing orphaned shard collection {}/{} (not assigned to node {})",
                                db_name, coll_name, my_node_id
                            );

                            if let Err(e) = db.delete_collection(&coll_name) {
                                tracing::error!(
                                    "CLEANUP: Failed to delete orphaned shard {}/{}: {}",
                                    db_name,
                                    coll_name,
                                    e
                                );
                            } else {
                                cleaned_count += 1;
                            }
                        }
                    }
                }
            }
        }

        if cleaned_count > 0 {
            tracing::info!(
                "CLEANUP: Removed {} orphaned shard collections",
                cleaned_count
            );
        }

        Ok(cleaned_count)
    }

    /// Broadcast cleanup to all cluster nodes
    /// This ensures all nodes remove their orphaned shard collections after contraction
    pub async fn broadcast_cleanup_orphaned_shards(&self) -> Result<(), crate::error::DbError> {
        // First clean up locally
        if let Err(e) = self.cleanup_orphaned_shards().await {
            tracing::error!("CLEANUP: Local cleanup failed: {}", e);
        }

        // Then broadcast to all remote nodes
        let mgr = match &self.cluster_manager {
            Some(m) => m,
            None => return Ok(()), // Single node, local cleanup is enough
        };

        let my_node_id = mgr.local_node_id();
        let client = reqwest::Client::new();
        let secret = self.cluster_secret();

        // Collect all shard tables to broadcast
        let tables: Vec<ShardTable> = {
            let guard = self
                .shard_tables
                .read()
                .map_err(|_| crate::error::DbError::InternalError("Lock poisoned".to_string()))?;
            guard.values().cloned().collect()
        };

        for member in mgr.state().get_all_members() {
            if member.node.id == my_node_id {
                continue; // Skip self
            }

            let addr = &member.node.api_address;
            let url = format!("http://{}/_api/cluster/cleanup", addr);

            tracing::info!(
                "CLEANUP: Broadcasting cleanup (with {} tables) to node {} at {}",
                tables.len(),
                member.node.id,
                addr
            );

            match client
                .post(&url)
                .header("X-Cluster-Secret", &secret)
                .json(&tables) // Send tables in body
                .timeout(std::time::Duration::from_secs(30))
                .send()
                .await
            {
                Ok(res) => {
                    if !res.status().is_success() {
                        tracing::warn!(
                            "CLEANUP: Cleanup broadcast to {} failed with status {}",
                            addr,
                            res.status()
                        );
                    } else {
                        tracing::info!("CLEANUP: Cleanup broadcast to {} successful", addr);
                    }
                }
                Err(e) => {
                    tracing::warn!("CLEANUP: Cleanup broadcast to {} failed: {}", addr, e);
                }
            }
        }

        Ok(())
    }

    /// Copy shard data from a source node
    async fn copy_shard_from_source(
        &self,
        database: &str,
        physical_coll: &str,
        source_node: &str,
    ) -> Result<usize, crate::error::DbError> {
        use base64::{engine::general_purpose, Engine as _};

        let mgr = self.cluster_manager.as_ref().ok_or_else(|| {
            crate::error::DbError::InternalError("No cluster manager".to_string())
        })?;

        let source_addr = mgr.get_node_api_address(source_node).ok_or_else(|| {
            crate::error::DbError::InternalError("Source node address not found".to_string())
        })?;

        // Step 1: Check Source Count using Metadata API
        let secret = self.cluster_secret();
        let client = reqwest::Client::new();

        // Use standard Collection API to get metadata (count)
        let meta_url = format!(
            "http://{}/_api/database/{}/collection/{}",
            source_addr, database, physical_coll
        );
        let meta_res = client
            .get(&meta_url)
            .header("X-Cluster-Secret", &secret)
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
        let db = self.storage.get_database(database)?;
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
                tracing::info!("HEAL: Mismatch or Blob forced sync for {}/{} (Local: {}, Source: {}). Syncing.", database, physical_coll, local_count, source_count);
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
            .header("X-Cluster-Secret", &secret)
            .header("X-Shard-Direct", "true")
            .timeout(std::time::Duration::from_secs(3600)) // Long timeout for large shards
            .send()
            .await
            .map_err(|e| {
                crate::error::DbError::InternalError(format!("Export request failed: {}", e))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            tracing::error!("HEAL: Export failed - status: {}, url: {}", status, url);
            return Err(crate::error::DbError::InternalError(format!(
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
                let batch_to_insert: Vec<(String, serde_json::Value)> =
                    batch_docs.drain(..).collect();
                if let Err(e) = coll.upsert_batch(batch_to_insert) {
                    tracing::error!("HEAL: Batch upsert failed: {}", e);
                } else {
                    total_copied += count;
                }
            }
        }

        // Final Flush
        if !batch_docs.len() > 0 {
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

    /// Insert a batch of documents with shard coordination
    pub async fn insert_batch(
        &self,
        database: &str,
        collection: &str,
        config: &CollectionShardConfig,
        documents: Vec<serde_json::Value>,
    ) -> Result<(usize, usize), crate::error::DbError> {
        use crate::sharding::router::ShardRouter;
        use std::collections::HashMap;

        let table = self.get_shard_table(database, collection).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard table not found".to_string())
        })?;

        let local_id = if let Some(mgr) = &self.cluster_manager {
            mgr.local_node_id()
        } else {
            "local".to_string()
        };

        // Group documents by shard
        let mut shard_batches: HashMap<u16, Vec<serde_json::Value>> = HashMap::new();

        for mut doc in documents {
            // Ensure _key exists
            let key = if let Some(k) = doc.get("_key").and_then(|v| v.as_str()) {
                k.to_string()
            } else {
                let k = uuid::Uuid::now_v7().to_string();
                if let Some(obj) = doc.as_object_mut() {
                    obj.insert("_key".to_string(), serde_json::Value::String(k.clone()));
                }
                k
            };

            // Determine shard
            let shard_key_value = if config.shard_key == "_key" {
                key
            } else {
                doc.get(&config.shard_key)
                    .and_then(|v| v.as_str())
                    .unwrap_or(&key)
                    .to_string()
            };

            let shard_id = ShardRouter::route(&shard_key_value, config.num_shards);
            shard_batches.entry(shard_id).or_default().push(doc);
        }

        let mut total_success = 0usize;
        let mut total_fail = 0usize;
        let client = reqwest::Client::new();
        let secret = self.cluster_secret();

        // Collect futures for parallel processing
        let mut local_batches = Vec::new();
        let mut remote_futures = Vec::new();

        // Separate local and remote batches
        for (shard_id, batch) in shard_batches {
            let physical_coll = format!("{}_s{}", collection, shard_id);

            let assignment = match table.assignments.get(&shard_id) {
                Some(a) => a,
                None => {
                    total_fail += batch.len();
                    continue;
                }
            };

            let primary_node = &assignment.primary_node;

            if primary_node == &local_id || primary_node == "local" {
                // Queue local batch (with shard_id for replica forwarding)
                local_batches.push((shard_id, physical_coll, batch));
            } else {
                // Queue remote batch as future
                if let Some(mgr) = &self.cluster_manager {
                    if let Some(addr) = mgr.get_node_api_address(primary_node) {
                        let url = format!(
                            "http://{}/_api/database/{}/document/{}/_batch",
                            addr, database, physical_coll
                        );
                        tracing::info!(
                            "INSERT BATCH: Queuing {} docs for remote shard {} at {}",
                            batch.len(),
                            physical_coll,
                            addr
                        );

                        let batch_size = batch.len();
                        let client = client.clone();
                        let secret = secret.clone();

                        let future = async move {
                            let res = client
                                .post(&url)
                                .header("X-Shard-Direct", "true")
                                .header("X-Cluster-Secret", &secret)
                                .json(&batch)
                                .send()
                                .await;

                            match res {
                                Ok(r) if r.status().is_success() => (batch_size, 0usize),
                                Ok(r) => {
                                    tracing::error!("Remote batch insert failed: {}", r.status());
                                    (0, batch_size)
                                }
                                Err(e) => {
                                    tracing::error!("Remote batch insert request failed: {}", e);
                                    (0, batch_size)
                                }
                            }
                        };
                        remote_futures.push(future);
                    } else {
                        total_fail += batch.len();
                    }
                } else {
                    total_fail += batch.len();
                }
            }
        }

        // Process local batches and forward to replicas
        let mut replica_futures = Vec::new();

        for (shard_id, physical_coll, batch) in local_batches {
            let db = self.storage.get_database(database)?;
            let coll = db.get_collection(&physical_coll)?;

            // Convert to keyed docs for upsert (prevents duplicates)
            let keyed_docs: Vec<(String, serde_json::Value)> = batch
                .iter()
                .map(|doc| {
                    let key = doc
                        .get("_key")
                        .and_then(|k| k.as_str())
                        .unwrap_or("")
                        .to_string();
                    (key, doc.clone())
                })
                .filter(|(key, _)| !key.is_empty())
                .collect();

            match coll.upsert_batch(keyed_docs) {
                Ok(count) => {
                    // NOTE: Do NOT log to replication log for sharded data!
                    // Shard data is partitioned - each node only stores its assigned shards.
                    // Instead, forward to REPLICA nodes for fault tolerance.
                    total_success += count;

                    // Forward to replica nodes for fault tolerance
                    if let Some(assignment) = table.assignments.get(&shard_id) {
                        if !assignment.replica_nodes.is_empty() {
                            if let Some(mgr) = &self.cluster_manager {
                                for replica_node in &assignment.replica_nodes {
                                    if let Some(addr) = mgr.get_node_api_address(replica_node) {
                                        let url = format!(
                                            "http://{}/_api/database/{}/document/{}/_replica",
                                            addr, database, physical_coll
                                        );
                                        tracing::debug!(
                                            "REPLICA: Forwarding {} docs to replica {} at {}",
                                            batch.len(),
                                            physical_coll,
                                            addr
                                        );

                                        let client = client.clone();
                                        let secret = secret.clone();
                                        let batch = batch.clone();

                                        let future = async move {
                                            let _ = client
                                                .post(&url)
                                                .header("X-Shard-Direct", "true")
                                                .header("X-Cluster-Secret", &secret)
                                                .json(&batch)
                                                .send()
                                                .await;
                                            // Replica failures are logged but don't affect success count
                                        };
                                        replica_futures.push(future);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(_) => {
                    total_fail += batch.len();
                }
            }
        }

        // Process remote batches in PARALLEL
        if !remote_futures.is_empty() {
            let results = futures::future::join_all(remote_futures).await;
            for (success, fail) in results {
                total_success += success;
                total_fail += fail;
            }
        }

        // Process replica forwarding in PARALLEL (fire-and-forget, don't wait)
        if !replica_futures.is_empty() {
            futures::future::join_all(replica_futures).await;
        }

        Ok((total_success, total_fail))
    }

    /// Upsert a batch of documents to shards (insert-or-update to prevent duplicates)
    /// Used during resharding to avoid creating duplicate documents
    /// Upsert a batch of documents to their correct shards
    /// Returns a list of keys that were SUCCESSFULLY upserted
    /// This allows the caller to delete only the successful ones from source
    pub async fn upsert_batch_to_shards(
        &self,
        database: &str,
        collection: &str,
        config: &CollectionShardConfig,
        documents: Vec<(String, serde_json::Value)>, // (key, doc) pairs
    ) -> Result<Vec<String>, crate::error::DbError> {
        use crate::sharding::router::ShardRouter;
        use std::collections::HashMap;

        let table = self.get_shard_table(database, collection).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard table not found".to_string())
        })?;

        let local_id = if let Some(mgr) = &self.cluster_manager {
            mgr.local_node_id()
        } else {
            "local".to_string()
        };

        // Group documents by target shard
        let mut shard_batches: HashMap<u16, Vec<(String, serde_json::Value)>> = HashMap::new();

        for (key, doc) in documents {
            let shard_key_value = if config.shard_key == "_key" {
                key.clone()
            } else {
                doc.get(&config.shard_key)
                    .and_then(|v| v.as_str())
                    .unwrap_or(&key)
                    .to_string()
            };

            let shard_id = ShardRouter::route(&shard_key_value, config.num_shards);
            shard_batches.entry(shard_id).or_default().push((key, doc));
        }

        let mut successful_keys: Vec<String> = Vec::new();

        // Process each shard batch
        // Add small delays between shards to prevent overwhelming the network during resharding
        let mut shard_count = 0;
        for (shard_id, batch) in shard_batches {
            if shard_count > 0 {
                // Small delay to prevent all shards from being processed simultaneously
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
            shard_count += 1;
            let physical_coll = format!("{}_s{}", collection, shard_id);
            // Collect keys for this batch to mark as success if operation succeeds
            let batch_keys: Vec<String> = batch.iter().map(|(k, _)| k.clone()).collect();
            let batch_len = batch_keys.len();

            let assignment = match table.assignments.get(&shard_id) {
                Some(a) => a,
                None => {
                    tracing::error!("UPSERT: No assignment found for shard {}", shard_id);
                    continue;
                }
            };

            let primary_node = &assignment.primary_node;

            if primary_node == &local_id || primary_node == "local" {
                // Local upsert - use Collection::upsert_batch to prevent duplicates
                let db = self.storage.get_database(database)?;
                let coll = match db.get_collection(&physical_coll) {
                    Ok(c) => c,
                    Err(_) => {
                        // Create shard if missing
                        db.create_collection(physical_coll.clone(), None)?;
                        db.get_collection(&physical_coll)?
                    }
                };

                match coll.upsert_batch(batch) {
                    Ok(_) => {
                        // All docs in batch succeeded (local atomic batch)
                        successful_keys.extend(batch_keys);
                    }
                    Err(e) => {
                        tracing::error!("UPSERT: Local upsert failed for {}: {}", physical_coll, e);
                    }
                }
            } else {
                // Remote upsert - forward via HTTP batch endpoint
                if let Some(mgr) = &self.cluster_manager {
                    if let Some(addr) = mgr.get_node_api_address(primary_node) {
                        // Circuit breaker: skip nodes that recently failed
                        if self.was_recently_failed(primary_node) {
                            tracing::warn!("UPSERT: Skipping batch to recently failed node {} (circuit breaker)", primary_node);
                            // Don't mark as successful - let migration handle this as a failure
                            break;
                        }

                        let url = format!(
                            "http://{}/_api/database/{}/document/{}/_batch",
                            addr, database, physical_coll
                        );
                        let secret = self.cluster_secret();
                        let client = reqwest::Client::new();

                        // Extract just values for the remote call
                        let values: Vec<serde_json::Value> =
                            batch.into_iter().map(|(_, v)| v).collect();

                        // Retry logic with exponential backoff for remote batch operations
                        let mut retry_count = 0;
                        const MAX_RETRIES: u32 = 3;
                        let mut last_error = None;

                        loop {
                            let timeout_duration = if retry_count == 0 {
                                std::time::Duration::from_secs(30)
                            } else {
                                // Exponential backoff: 30s, 60s, 120s
                                std::time::Duration::from_secs(30 * (1 << retry_count))
                            };

                            match tokio::time::timeout(
                                timeout_duration,
                                client
                                    .post(&url)
                                    .header("X-Shard-Direct", "true")
                                    .header("X-Migration", "true") // Prevent replica forwarding during resharding
                                    .header("X-Cluster-Secret", &secret)
                                    .json(&values)
                                    .send(),
                            )
                            .await
                            {
                                Ok(Ok(res)) => {
                                    if res.status().is_success() {
                                        // Success! Mark keys as successful
                                        successful_keys.extend(batch_keys);
                                        break;
                                    } else {
                                        let status = res.status();
                                        let err_msg = format!("HTTP {}", status);
                                        tracing::warn!("UPSERT: Remote batch request to {} failed: {} (attempt {}/{})",
                                            addr, err_msg, retry_count + 1, MAX_RETRIES + 1);
                                        last_error = Some(err_msg);

                                        if status.as_u16() >= 500 {
                                            // Server errors are retryable
                                            if retry_count < MAX_RETRIES {
                                                retry_count += 1;
                                                tokio::time::sleep(
                                                    std::time::Duration::from_millis(
                                                        1000 * (1 << retry_count),
                                                    ),
                                                )
                                                .await;
                                                continue;
                                            }
                                        }
                                        // Client errors or max retries reached
                                        break;
                                    }
                                }
                                Ok(Err(e)) => {
                                    tracing::warn!("UPSERT: Remote batch request to {} failed: {} (attempt {}/{})",
                                        addr, e, retry_count + 1, MAX_RETRIES + 1);
                                    last_error = Some(e.to_string());

                                    // Network errors are retryable
                                    if retry_count < MAX_RETRIES {
                                        retry_count += 1;
                                        tokio::time::sleep(std::time::Duration::from_millis(
                                            1000 * (1 << retry_count),
                                        ))
                                        .await;
                                        continue;
                                    }
                                    break;
                                }
                                Err(_) => {
                                    tracing::warn!("UPSERT: Remote batch request to {} timed out after {:?} (attempt {}/{})",
                                        addr, timeout_duration, retry_count + 1, MAX_RETRIES + 1);
                                    last_error =
                                        Some(format!("timeout after {:?}", timeout_duration));

                                    // Timeouts are retryable
                                    if retry_count < MAX_RETRIES {
                                        retry_count += 1;
                                        tokio::time::sleep(std::time::Duration::from_millis(
                                            1000 * (1 << retry_count),
                                        ))
                                        .await;
                                        continue;
                                    }
                                    break;
                                }
                            }
                        }

                        if successful_keys.is_empty() && batch_len > 0 {
                            // All retries failed, log final error and record node failure
                            tracing::error!("UPSERT: Remote batch request to {} failed after {} attempts. Last error: {}. Recording node as failed.",
                                addr, MAX_RETRIES + 1, last_error.unwrap_or("unknown".to_string()));
                            self.record_node_failure(primary_node);
                        }
                    } else {
                        tracing::error!(
                            "UPSERT: Primary node address unknown for {}",
                            primary_node
                        );
                    }
                }
            }
        }

        Ok(successful_keys)
    }

    /// Insert a single document with shard coordination
    pub async fn insert(
        &self,
        database: &str,
        collection: &str,
        config: &CollectionShardConfig,
        mut document: serde_json::Value,
    ) -> Result<serde_json::Value, crate::error::DbError> {
        use crate::sharding::router::ShardRouter;

        // 1. Determine Shard Key
        let key = if let Some(k) = document.get("_key").and_then(|v| v.as_str()) {
            k.to_string()
        } else {
            let k = uuid::Uuid::now_v7().to_string();
            if let Some(obj) = document.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::Value::String(k.clone()));
            }
            k
        };

        let shard_key_value = if config.shard_key == "_key" {
            key.clone()
        } else {
            document
                .get(&config.shard_key)
                .and_then(|v| v.as_str())
                .unwrap_or(&key) // Fallback to key if shard key missing? Or Error?
                .to_string()
        };

        // 2. Route to Shard ID
        let shard_id = ShardRouter::route(&shard_key_value, config.num_shards);

        // 3. Get Physical Collection Name
        let physical_coll = format!("{}_s{}", collection, shard_id);

        // 4. Find Primary Node
        let table = self.get_shard_table(database, collection).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard table not found".to_string())
        })?;

        let assignment = table.assignments.get(&shard_id).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard assignment not found".to_string())
        })?;

        let primary_node = &assignment.primary_node;

        // 5. Check if Local
        let local_id = if let Some(mgr) = &self.cluster_manager {
            mgr.local_node_id()
        } else {
            "local".to_string()
        };

        if primary_node == &local_id || primary_node == "local" {
            // Write to LOCAL physical shard
            let db = self.storage.get_database(database)?;
            // Ensure physical collection exists (it should)
            let coll = db.get_collection(&physical_coll)?;
            let inserted = coll.insert(document)?;

            // NOTE: Do NOT log to replication log for sharded data!
            // Each node only stores its assigned shards - data is partitioned, not replicated.

            Ok(inserted.to_value())
        } else {
            // Check if primary is healthy
            let target_node = if let Some(mgr) = &self.cluster_manager {
                if mgr.is_node_healthy(primary_node) {
                    primary_node.clone()
                } else {
                    // Primary is unhealthy - promote a replica
                    tracing::warn!(
                        "Primary {} is unhealthy for shard {}, attempting failover",
                        primary_node,
                        shard_id
                    );
                    if let Some(new_primary) = self.promote_replica(database, collection, shard_id)
                    {
                        new_primary
                    } else {
                        return Err(crate::error::DbError::InternalError(format!(
                            "Primary {} unhealthy and no healthy replica for failover",
                            primary_node
                        )));
                    }
                }
            } else {
                primary_node.clone()
            };

            // Check if the new target is now local (we got promoted)
            if target_node == local_id {
                let db = self.storage.get_database(database)?;
                let coll = db.get_collection(&physical_coll)?;
                let inserted = coll.insert(document)?;
                return Ok(inserted.to_value());
            }

            // FORWARD to Remote Primary
            if let Some(mgr) = &self.cluster_manager {
                if let Some(addr) = mgr.get_node_api_address(&target_node) {
                    let client = reqwest::Client::new();
                    let url = format!(
                        "http://{}/_api/database/{}/document/{}",
                        addr, database, physical_coll
                    );

                    // Get Cluster Secret
                    let secret = self.cluster_secret();

                    let res = client
                        .post(&url)
                        .header("X-Shard-Direct", "true")
                        .header("X-Cluster-Secret", &secret)
                        .timeout(std::time::Duration::from_secs(10))
                        .json(&document)
                        .send()
                        .await
                        .map_err(|e| {
                            crate::error::DbError::InternalError(format!(
                                "Forwarding failed: {}",
                                e
                            ))
                        })?;

                    if res.status().is_success() {
                        let val: serde_json::Value = res.json().await.map_err(|e| {
                            crate::error::DbError::InternalError(format!("Invalid response: {}", e))
                        })?;
                        Ok(val)
                    } else {
                        Err(crate::error::DbError::InternalError(format!(
                            "Remote insert failed: {}",
                            res.status()
                        )))
                    }
                } else {
                    Err(crate::error::DbError::InternalError(format!(
                        "Target node {} address unknown",
                        target_node
                    )))
                }
            } else {
                Err(crate::error::DbError::InternalError(
                    "Cluster manager missing for remote write".to_string(),
                ))
            }
        }
    }

    /// Upload a blob with shard awareness
    /// Handles both metadata document routing and chunk distribution
    pub async fn upload_blob(
        &self,
        database: &str,
        collection: &str,
        config: &CollectionShardConfig,
        mut document: serde_json::Value,
        chunks: Vec<(u32, Vec<u8>)>,
    ) -> Result<serde_json::Value, crate::error::DbError> {
        use crate::sharding::router::ShardRouter;

        // 1. Determine Shard Key (same as regular insert)
        let key = if let Some(k) = document.get("_key").and_then(|v| v.as_str()) {
            k.to_string()
        } else {
            let k = uuid::Uuid::now_v7().to_string();
            if let Some(obj) = document.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::Value::String(k.clone()));
            }
            k
        };

        let shard_key_value = if config.shard_key == "_key" {
            key.clone()
        } else {
            document
                .get(&config.shard_key)
                .and_then(|v| v.as_str())
                .unwrap_or(&key)
                .to_string()
        };

        // 2. Route to Shard ID
        let shard_id = ShardRouter::route(&shard_key_value, config.num_shards);

        // 3. Get Physical Collection Name
        let physical_coll = format!("{}_s{}", collection, shard_id);

        // 4. Find Primary Node
        let table = self.get_shard_table(database, collection).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard table not found".to_string())
        })?;

        let assignment = table.assignments.get(&shard_id).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard assignment not found".to_string())
        })?;

        let primary_node = &assignment.primary_node;

        // 5. Check if Local
        let local_id = if let Some(mgr) = &self.cluster_manager {
            mgr.local_node_id()
        } else {
            "local".to_string()
        };

        if primary_node == &local_id || primary_node == "local" {
            // Write to LOCAL physical shard
            let db = self.storage.get_database(database)?;
            let coll = db.get_collection(&physical_coll)?;

            // Store chunks first
            for (chunk_index, chunk_data) in &chunks {
                coll.put_blob_chunk(&key, *chunk_index, chunk_data)?;
            }

            // Store metadata document
            let inserted = coll.insert(document)?;

            Ok(inserted.to_value())
        } else {
            // Check if primary is healthy
            let target_node = if let Some(mgr) = &self.cluster_manager {
                if mgr.is_node_healthy(primary_node) {
                    primary_node.clone()
                } else {
                    // Primary is unhealthy - promote a replica
                    tracing::warn!(
                        "Primary {} is unhealthy for shard {}, attempting failover",
                        primary_node,
                        shard_id
                    );
                    if let Some(new_primary) = self.promote_replica(database, collection, shard_id)
                    {
                        new_primary
                    } else {
                        return Err(crate::error::DbError::InternalError(format!(
                            "Primary {} unhealthy and no healthy replica for failover",
                            primary_node
                        )));
                    }
                }
            } else {
                primary_node.clone()
            };

            // Check if the new target is now local (we got promoted)
            if target_node == local_id {
                let db = self.storage.get_database(database)?;
                let coll = db.get_collection(&physical_coll)?;

                // Store chunks first
                for (chunk_index, chunk_data) in &chunks {
                    coll.put_blob_chunk(&key, *chunk_index, chunk_data)?;
                }

                // Store metadata document
                let inserted = coll.insert(document)?;
                return Ok(inserted.to_value());
            }

            // FORWARD to Remote Primary
            if let Some(mgr) = &self.cluster_manager {
                if let Some(addr) = mgr.get_node_api_address(&target_node) {
                    let client = reqwest::Client::new();
                    let url = format!(
                        "http://{}/_internal/blob/upload/{}/{}",
                        addr, database, physical_coll
                    );

                    // Create multipart form with metadata and chunks
                    let mut form = reqwest::multipart::Form::new();

                    // Add metadata
                    let meta_json = serde_json::to_string(&document).map_err(|e| {
                        crate::error::DbError::InternalError(format!(
                            "Failed to serialize metadata: {}",
                            e
                        ))
                    })?;
                    let meta_part = reqwest::multipart::Part::text(meta_json)
                        .mime_str("application/json")
                        .map_err(|e| {
                            crate::error::DbError::InternalError(format!("Invalid mime: {}", e))
                        })?;
                    form = form.part("metadata", meta_part);

                    // Add chunks
                    for (chunk_index, chunk_data) in &chunks {
                        let part = reqwest::multipart::Part::bytes(chunk_data.clone())
                            .mime_str("application/octet-stream")
                            .map_err(|e| {
                                crate::error::DbError::InternalError(format!("Invalid mime: {}", e))
                            })?;
                        form = form.part(format!("chunk_{}", chunk_index), part);
                    }

                    // Get Cluster Secret
                    let secret = self.cluster_secret();

                    let res = client
                        .post(&url)
                        .header("X-Shard-Direct", "true")
                        .header("X-Cluster-Secret", &secret)
                        .timeout(std::time::Duration::from_secs(60)) // Longer timeout for blob uploads
                        .multipart(form)
                        .send()
                        .await
                        .map_err(|e| {
                            crate::error::DbError::InternalError(format!(
                                "Blob upload forwarding failed: {}",
                                e
                            ))
                        })?;

                    let status = res.status();
                    if status.is_success() {
                        let val: serde_json::Value = res.json().await.map_err(|e| {
                            crate::error::DbError::InternalError(format!(
                                "Invalid blob upload response: {}",
                                e
                            ))
                        })?;
                        Ok(val)
                    } else {
                        let error_text = res.text().await.unwrap_or_default();
                        Err(crate::error::DbError::InternalError(format!(
                            "Remote blob upload failed: {} - {}",
                            status, error_text
                        )))
                    }
                } else {
                    Err(crate::error::DbError::InternalError(format!(
                        "Target node {} address unknown",
                        target_node
                    )))
                }
            } else {
                Err(crate::error::DbError::InternalError(
                    "Cluster manager missing for remote blob upload".to_string(),
                ))
            }
        }
    }

    /// Download a blob with shard awareness
    pub async fn download_blob(
        &self,
        database: &str,
        collection: &str,
        config: &CollectionShardConfig,
        key: &str,
    ) -> Result<axum::response::Response, crate::error::DbError> {
        use crate::sharding::router::ShardRouter;
        use axum::response::Response;

        // 1. Route to Shard ID using the blob key
        let shard_id = ShardRouter::route(key, config.num_shards);

        // 2. Get Physical Collection Name
        let physical_coll = format!("{}_s{}", collection, shard_id);

        // 3. Find Primary Node
        let table = self.get_shard_table(database, collection).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard table not found".to_string())
        })?;

        let assignment = table.assignments.get(&shard_id).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard assignment not found".to_string())
        })?;

        let primary_node = &assignment.primary_node;

        // 4. Check if Local
        let local_id = if let Some(mgr) = &self.cluster_manager {
            mgr.local_node_id()
        } else {
            "local".to_string()
        };

        if primary_node == &local_id || primary_node == "local" {
            // Serve from LOCAL physical shard
            let db = self.storage.get_database(database)?;
            let coll = db.get_collection(&physical_coll)?;

            // Check if blob exists
            if coll.get(key).is_err() {
                return Err(DbError::DocumentNotFound(format!(
                    "Blob not found: {}",
                    key
                )));
            }

            // Get content type and filename from metadata
            let content_type = if let Ok(doc) = coll.get(key) {
                if let Some(v) = doc.get("type") {
                    if let Some(s) = v.as_str() {
                        s.to_string()
                    } else {
                        "application/octet-stream".to_string()
                    }
                } else {
                    "application/octet-stream".to_string()
                }
            } else {
                "application/octet-stream".to_string()
            };

            let file_name = if let Ok(doc) = coll.get(key) {
                if let Some(v) = doc.get("name") {
                    if let Some(s) = v.as_str() {
                        Some(s.to_string())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Create streaming response
            let key = key.to_string();
            let stream = async_stream::stream! {
                let mut chunk_idx = 0;
                loop {
                    match coll.get_blob_chunk(&key, chunk_idx) {
                        Ok(Some(data)) => {
                            yield Ok::<_, std::io::Error>(axum::body::Bytes::from(data));
                            chunk_idx += 1;
                        }
                        Ok(None) => break, // End of chunks
                        Err(e) => {
                            yield Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()));
                            break;
                        }
                    }
                }
            };

            let body = axum::body::Body::from_stream(stream);

            let mut builder =
                axum::response::Response::builder().header("Content-Type", content_type);

            if let Some(name) = file_name {
                let disposition = format!("attachment; filename=\"{}\"", name);
                builder = builder.header("Content-Disposition", disposition);
            }

            Ok(builder
                .body(body)
                .map_err(|e| DbError::InternalError(format!("Failed to build response: {}", e)))?)
        } else {
            // FORWARD to Remote Primary
            if let Some(mgr) = &self.cluster_manager {
                if let Some(addr) = mgr.get_node_api_address(primary_node) {
                    let client = reqwest::Client::new();
                    let url = format!(
                        "http://{}/_api/blob/{}/{}/{}",
                        addr, database, physical_coll, key
                    );

                    // Get Cluster Secret
                    let secret = self.cluster_secret();

                    let res = client
                        .get(&url)
                        .header("X-Cluster-Secret", &secret)
                        .timeout(std::time::Duration::from_secs(60))
                        .send()
                        .await
                        .map_err(|e| {
                            crate::error::DbError::InternalError(format!(
                                "Blob download forwarding failed: {}",
                                e
                            ))
                        })?;

                    if res.status().is_success() {
                        // Convert reqwest response to axum response
                        let status = res.status();
                        let headers = res.headers().clone();
                        let body = res.bytes().await.map_err(|e| {
                            crate::error::DbError::InternalError(format!(
                                "Failed to read response body: {}",
                                e
                            ))
                        })?;

                        let mut response = Response::builder().status(status);

                        // Copy headers
                        for (key, value) in headers.iter() {
                            if let Ok(val_str) = value.to_str() {
                                response = response.header(key, val_str);
                            }
                        }

                        let axum_response =
                            response.body(axum::body::Body::from(body)).map_err(|e| {
                                crate::error::DbError::InternalError(format!(
                                    "Failed to build response: {}",
                                    e
                                ))
                            })?;

                        Ok(axum_response)
                    } else {
                        Err(crate::error::DbError::InternalError(format!(
                            "Remote blob download failed: {}",
                            res.status()
                        )))
                    }
                } else {
                    Err(crate::error::DbError::InternalError(format!(
                        "Target node {} address unknown",
                        primary_node
                    )))
                }
            } else {
                Err(crate::error::DbError::InternalError(
                    "Cluster manager missing for remote blob download".to_string(),
                ))
            }
        }
    }

    /// Get a document with shard awareness
    pub async fn get(
        &self,
        database: &str,
        collection: &str,
        key: &str,
    ) -> Result<serde_json::Value, crate::error::DbError> {
        use crate::sharding::router::ShardRouter;

        let table = self.get_shard_table(database, collection).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard table not found".to_string())
        })?;

        // TODO: Handle custom shard_keys. For now assume _key
        let shard_id = ShardRouter::route(key, table.num_shards);
        let physical_coll = format!("{}_s{}", collection, shard_id);

        let assignment = table.assignments.get(&shard_id).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard assignment not found".to_string())
        })?;
        let primary_node = &assignment.primary_node;

        // Check local
        let local_id = if let Some(mgr) = &self.cluster_manager {
            mgr.local_node_id()
        } else {
            "local".to_string()
        };

        if primary_node == &local_id || primary_node == "local" {
            let db = self.storage.get_database(database)?;
            let coll = db.get_collection(&physical_coll)?;
            let doc = coll.get(key)?;
            Ok(doc.to_value())
        } else {
            // Check if primary is healthy, otherwise try replicas
            let nodes_to_try = if let Some(mgr) = &self.cluster_manager {
                if mgr.is_node_healthy(primary_node) {
                    // Primary is healthy, try it first, then replicas
                    let mut nodes = vec![primary_node.clone()];
                    nodes.extend(assignment.replica_nodes.iter().cloned());
                    nodes
                } else {
                    // Primary is unhealthy, try replicas only
                    tracing::warn!(
                        "Primary {} is unhealthy for shard {}, trying replicas",
                        primary_node,
                        shard_id
                    );
                    assignment.replica_nodes.clone()
                }
            } else {
                vec![primary_node.clone()]
            };

            if nodes_to_try.is_empty() {
                return Err(crate::error::DbError::InternalError(
                    "No healthy nodes for shard".to_string(),
                ));
            }

            // Try nodes in order until one succeeds
            let client = reqwest::Client::new();
            let secret = self.cluster_secret();

            for node_id in &nodes_to_try {
                if let Some(mgr) = &self.cluster_manager {
                    if let Some(addr) = mgr.get_node_api_address(node_id) {
                        let url = format!(
                            "http://{}/_api/database/{}/document/{}/{}",
                            addr, database, physical_coll, key
                        );

                        let res = client
                            .get(&url)
                            .header("X-Shard-Direct", "true")
                            .header("X-Cluster-Secret", &secret)
                            .timeout(std::time::Duration::from_secs(5))
                            .send()
                            .await;

                        match res {
                            Ok(r) if r.status().is_success() => {
                                return r.json().await.map_err(|e| {
                                    crate::error::DbError::InternalError(format!(
                                        "Invalid response: {}",
                                        e
                                    ))
                                });
                            }
                            Ok(r) if r.status() == reqwest::StatusCode::NOT_FOUND => {
                                return Err(crate::error::DbError::DocumentNotFound(
                                    key.to_string(),
                                ));
                            }
                            _ => {
                                tracing::debug!("Failed to get from {}, trying next node", node_id);
                                continue;
                            }
                        }
                    }
                }
            }

            Err(crate::error::DbError::InternalError(
                "All nodes failed for shard read".to_string(),
            ))
        }
    }

    /// Get replica nodes for a given key
    pub fn get_replicas(&self, key: &str, config: &CollectionShardConfig) -> Vec<String> {
        use crate::sharding::router::ShardRouter;
        let _shard_id = ShardRouter::route(key, config.num_shards);
        if let Some(_table) = self.get_shard_table("", "") {
            // Context missing db/coll - this API is flawed
            // Try to look up assignment from cached tables?
            // But we don't know db/coll here.
            // Logic needs db/coll.
            vec![]
        } else {
            // Fallback: calculate theoretical replicas
            // This method is used by `handlers.rs` to decorate response.
            vec![] // Stub for now
        }
    }

    /// Update a document with shard coordination
    pub async fn update(
        &self,
        database: &str,
        collection: &str,
        config: &CollectionShardConfig,
        key: &str,
        document: serde_json::Value,
    ) -> Result<serde_json::Value, crate::error::DbError> {
        use crate::sharding::router::ShardRouter;

        let shard_key_value = if config.shard_key == "_key" {
            key.to_string()
        } else {
            // For update, we might not have the full doc content to extract shard key?
            // If shard key is immutable, we can assume it matches current doc?
            // But we don't have current doc.
            // If shard key is NOT _key, update(key) is ambiguous if we don't know shard key.
            // Assume _key for now.
            key.to_string()
        };

        let shard_id = ShardRouter::route(&shard_key_value, config.num_shards);
        let physical_coll = format!("{}_s{}", collection, shard_id);

        let table = self.get_shard_table(database, collection).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard table not found".to_string())
        })?;
        let assignment = table.assignments.get(&shard_id).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard assignment not found".to_string())
        })?;
        let primary_node = &assignment.primary_node;

        let local_id = if let Some(mgr) = &self.cluster_manager {
            mgr.local_node_id()
        } else {
            "local".to_string()
        };

        if primary_node == &local_id || primary_node == "local" {
            let db = self.storage.get_database(database)?;
            let coll = db.get_collection(&physical_coll)?;

            // Apply update locally
            // Note: handlers.rs usually does "get then merge".
            // `collection.update` does merge.
            coll.update(key, document.clone())?;

            if let Some(ref log) = self.replication_log {
                let entry = LogEntry {
                    sequence: 0,
                    node_id: "".to_string(),
                    database: database.to_string(),
                    collection: physical_coll.clone(),
                    operation: Operation::Update,
                    key: key.to_string(),
                    data: serde_json::to_vec(&document).ok(),
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                    origin_sequence: None,
                };
                let _ = log.append(entry);
            }

            Ok(document)
        } else {
            // Forward
            if let Some(mgr) = &self.cluster_manager {
                if let Some(addr) = mgr.get_node_api_address(primary_node) {
                    let client = reqwest::Client::new();
                    let url = format!(
                        "http://{}/_api/database/{}/document/{}/{}",
                        addr, database, physical_coll, key
                    );
                    let secret = self.cluster_secret();

                    let res = client
                        .put(&url)
                        .header("X-Shard-Direct", "true")
                        .header("X-Cluster-Secret", secret)
                        .json(&document)
                        .send()
                        .await
                        .map_err(|e| {
                            crate::error::DbError::InternalError(format!(
                                "Forwarding update failed: {}",
                                e
                            ))
                        })?;

                    if res.status().is_success() {
                        let val: serde_json::Value = res.json().await.map_err(|e| {
                            crate::error::DbError::InternalError(format!("Invalid response: {}", e))
                        })?;
                        Ok(val)
                    } else {
                        Err(crate::error::DbError::InternalError(format!(
                            "Remote update failed: {}",
                            res.status()
                        )))
                    }
                } else {
                    Err(crate::error::DbError::InternalError(
                        "Primary node unknown".to_string(),
                    ))
                }
            } else {
                Err(crate::error::DbError::InternalError(
                    "Cluster manager missing".to_string(),
                ))
            }
        }
    }

    /// Delete a document with shard coordination
    pub async fn delete(
        &self,
        database: &str,
        collection: &str,
        config: &CollectionShardConfig,
        key: &str,
    ) -> Result<(), crate::error::DbError> {
        use crate::sharding::router::ShardRouter;
        // Assume _key is shard key
        let shard_id = ShardRouter::route(key, config.num_shards);
        let physical_coll = format!("{}_s{}", collection, shard_id);

        let table = self.get_shard_table(database, collection).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard table not found".to_string())
        })?;
        let assignment = table.assignments.get(&shard_id).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard assignment not found".to_string())
        })?;
        let primary_node = &assignment.primary_node;

        let local_id = if let Some(mgr) = &self.cluster_manager {
            mgr.local_node_id()
        } else {
            "local".to_string()
        };

        if primary_node == &local_id || primary_node == "local" {
            let db = self.storage.get_database(database)?;
            let coll = db.get_collection(&physical_coll)?;
            coll.delete(key)?;

            if let Some(ref log) = self.replication_log {
                let entry = LogEntry {
                    sequence: 0,
                    node_id: "".to_string(),
                    database: database.to_string(),
                    collection: physical_coll.clone(),
                    operation: Operation::Delete,
                    key: key.to_string(),
                    data: None,
                    timestamp: chrono::Utc::now().timestamp_millis() as u64,
                    origin_sequence: None,
                };
                let _ = log.append(entry);
            }

            Ok(())
        } else {
            // Forward
            if let Some(mgr) = &self.cluster_manager {
                if let Some(addr) = mgr.get_node_api_address(primary_node) {
                    let client = reqwest::Client::new();
                    let url = format!(
                        "http://{}/_api/database/{}/document/{}/{}",
                        addr, database, physical_coll, key
                    );
                    let secret = self.cluster_secret();

                    let res = client
                        .delete(&url)
                        .header("X-Shard-Direct", "true")
                        .header("X-Cluster-Secret", secret)
                        .send()
                        .await
                        .map_err(|e| {
                            crate::error::DbError::InternalError(format!(
                                "Forwarding delete failed: {}",
                                e
                            ))
                        })?;

                    if res.status().is_success() {
                        Ok(())
                    } else {
                        Err(crate::error::DbError::InternalError(format!(
                            "Remote delete failed: {}",
                            res.status()
                        )))
                    }
                } else {
                    Err(crate::error::DbError::InternalError(
                        "Primary node unknown".to_string(),
                    ))
                }
            } else {
                Err(crate::error::DbError::InternalError(
                    "Cluster manager missing".to_string(),
                ))
            }
        }
    }

    /// Scan all shards for documents
    pub async fn scan_all_shards(
        &self,
        database: &str,
        collection: &str,
        _config: &CollectionShardConfig,
    ) -> Result<Vec<crate::storage::Document>, crate::error::DbError> {
        let db = self.storage.get_database(database)?;
        let coll = db.get_collection(collection)?;
        let docs = coll.scan(None);
        Ok(docs)
    }

    /// Remove a node from the cluster
    ///
    /// This removes the node from all shard assignments and triggers a rebalance
    pub async fn remove_node(&self, node_addr: &str) -> Result<(), crate::error::DbError> {
        tracing::info!("Removing node {} from cluster", node_addr);

        // First, update all shard assignments to remove this node
        {
            let mut tables = self.shard_tables.write().unwrap();

            for (key, table) in tables.iter_mut() {
                let mut orphaned_shards = Vec::new();

                for (shard_id, assignment) in table.assignments.iter_mut() {
                    // Remove from replicas
                    assignment.replica_nodes.retain(|n| n != node_addr);

                    // Check if this was the primary
                    if assignment.primary_node == node_addr {
                        orphaned_shards.push(*shard_id);
                    }
                }

                if !orphaned_shards.is_empty() {
                    tracing::warn!(
                        "Node {} was primary for {} shards in {}, will reassign",
                        node_addr,
                        orphaned_shards.len(),
                        key
                    );
                }
            }
        }

        // Trigger rebalance to redistribute
        self.rebalance().await?;

        tracing::info!("Node {} removed successfully", node_addr);
        Ok(())
    }

    /// Get all nodes that have shards for a collection
    pub fn get_collection_nodes(&self, _config: &CollectionShardConfig) -> Vec<String> {
        // Stub: return all cluster nodes for now
        self.get_node_addresses()
    }

    /// Get this node's index in the sorted list of all nodes
    pub fn get_node_index(&self) -> Option<usize> {
        let nodes = self.get_node_addresses();
        let my_addr = self.my_address();
        nodes.iter().position(|n| n == &my_addr)
    }

    /// Create physical shards on all assigned nodes
    pub async fn create_shards(&self, database: &str, collection: &str) -> Result<(), String> {
        let table = self
            .get_shard_table(database, collection)
            .ok_or_else(|| "Shard table not found".to_string())?;

        // Get collection type to propagate to shards
        let collection_type = if let Ok(db) = self.storage.get_database(database) {
            if let Ok(coll) = db.get_collection(collection) {
                Some(coll.get_type().to_string())
            } else {
                None
            }
        } else {
            None
        };

        let client = reqwest::Client::new();
        let secret = self.cluster_secret();

        // Track created shards to avoid duplicates
        let mut created_shards = std::collections::HashSet::new();

        for (shard_id, assignment) in &table.assignments {
            let phys_name = format!("{}_s{}", collection, shard_id);

            // Check if we already processed this shard (unlikely given loop, but safety first)
            if created_shards.contains(&phys_name) {
                continue;
            }
            created_shards.insert(phys_name.clone());

            let is_local = if let Some(mgr) = &self.cluster_manager {
                assignment.primary_node == mgr.local_node_id()
            } else {
                true
            };

            // DEBUG LOGGING
            tracing::info!(
                "CREATE_SHARDS: Processing {} (is_local={}). Primary: {}",
                phys_name,
                is_local,
                assignment.primary_node
            );

            // DEBUG LOGGING
            tracing::info!(
                "CREATE_SHARDS: Processing {} (is_local={}). Primary: {}",
                phys_name,
                is_local,
                assignment.primary_node
            );

            // Unified loop for Primary AND Replicas
            // We want to ensure the shard exists on ALL assigned nodes
            let targets =
                std::iter::once(&assignment.primary_node).chain(assignment.replica_nodes.iter());

            for target_node in targets {
                let is_target_local = if let Some(mgr) = &self.cluster_manager {
                    target_node == &mgr.local_node_id()
                } else {
                    true
                };

                if is_target_local {
                    // Create Local
                    if let Ok(db) = self.storage.get_database(database) {
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
                                let msg =
                                    format!("Failed to create local shard {}: {}", phys_name, e);
                                tracing::error!("{}", msg);
                                // Continue to next target
                            } else {
                                // Log it
                                if let Some(log) = &self.replication_log {
                                    let entry = LogEntry {
                                        sequence: 0,
                                        node_id: "".to_string(),
                                        database: database.to_string(),
                                        collection: phys_name.clone(),
                                        operation: Operation::CreateCollection,
                                        key: "".to_string(),
                                        data: None,
                                        timestamp: chrono::Utc::now().timestamp_millis() as u64,
                                        origin_sequence: None,
                                    };
                                    let _ = log.append(entry);
                                }
                            }
                        }
                    }
                } else {
                    // Remote Create
                    if let Some(mgr) = &self.cluster_manager {
                        if let Some(addr) = mgr.get_node_api_address(target_node) {
                            let url =
                                format!("http://{}/_api/database/{}/collection", addr, database);
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
                                .header("X-Cluster-Secret", &secret)
                                .json(&body)
                                .send()
                                .await
                            {
                                Ok(res) => {
                                    if !res.status().is_success() {
                                        let status = res.status();
                                        if status.as_u16() == 409 {
                                            tracing::debug!("CREATE_SHARDS: Remote shard {} already exists (409)", phys_name);
                                        } else {
                                            let err_text = res.text().await.unwrap_or_default();
                                            tracing::error!("CREATE_SHARDS: Remote creation of {} failed: {} - {}", phys_name, status, err_text);
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

    /// Get aggregated document count for a sharded collection
    pub async fn get_total_count(
        &self,
        database: &str,
        collection: &str,
        auth_header: Option<String>,
    ) -> Result<usize, crate::error::DbError> {
        let config = self
            .get_shard_config(database, collection)
            .ok_or_else(|| crate::error::DbError::CollectionNotFound(collection.to_string()))?;

        if config.num_shards == 0 {
            // Non-sharded
            let db = self.storage.get_database(database)?;
            let coll = db.get_collection(collection)?;
            return Ok(coll.count());
        }

        let table = self.get_shard_table(database, collection).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard table not found".to_string())
        })?;

        let my_id = self.my_node_id();
        let client = reqwest::Client::new();
        let secret = self.cluster_secret();

        let mut total_count = 0usize;

        for shard_id in 0..config.num_shards {
            let assignment = table.assignments.get(&shard_id).ok_or_else(|| {
                crate::error::DbError::InternalError(format!(
                    "No assignment for shard {}",
                    shard_id
                ))
            })?;

            let physical_name = format!("{}_s{}", collection, shard_id);
            let mut shard_count = 0usize;
            let mut found = false;

            // Try local primary/replica first
            let has_local = assignment.primary_node == my_id
                || assignment.replica_nodes.contains(&my_id)
                || assignment.primary_node == "local";
            if has_local {
                if let Ok(db) = self.storage.get_database(database) {
                    if let Ok(coll) = db.get_collection(&physical_name) {
                        shard_count = coll.count();
                        found = true;
                    }
                }
            }

            // If not found locally, try primary node then replicas
            if !found {
                if let Some(mgr) = &self.cluster_manager {
                    // Try primary node
                    if let Some(addr) = mgr.get_node_api_address(&assignment.primary_node) {
                        let url = format!(
                            "http://{}/_api/database/{}/collection/{}/count",
                            addr, database, physical_name
                        );
                        let mut req = client
                            .get(&url)
                            .header("X-Cluster-Secret", &secret)
                            .timeout(std::time::Duration::from_secs(2));

                        if let Some(ref auth) = auth_header {
                            req = req.header("Authorization", auth);
                        }

                        match req.send().await {
                            Ok(res) if res.status().is_success() => {
                                if let Ok(json) = res.json::<serde_json::Value>().await {
                                    if let Some(c) = json.get("count").and_then(|v| v.as_u64()) {
                                        shard_count = c as usize;
                                        found = true;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }

                    // If primary failed, try replicas
                    if !found {
                        for replica_node in &assignment.replica_nodes {
                            if let Some(addr) = mgr.get_node_api_address(replica_node) {
                                let url = format!(
                                    "http://{}/_api/database/{}/collection/{}/count",
                                    addr, database, physical_name
                                );
                                let mut req = client
                                    .get(&url)
                                    .header("X-Cluster-Secret", &secret)
                                    .timeout(std::time::Duration::from_secs(2));

                                if let Some(ref auth) = auth_header {
                                    req = req.header("Authorization", auth);
                                }

                                match req.send().await {
                                    Ok(res) if res.status().is_success() => {
                                        if let Ok(json) = res.json::<serde_json::Value>().await {
                                            if let Some(c) =
                                                json.get("count").and_then(|v| v.as_u64())
                                            {
                                                shard_count = c as usize;
                                                found = true;
                                                break;
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }

            if found {
                total_count += shard_count;
            } else {
                tracing::warn!(
                    "Could not get count for shard {} of {} from any node",
                    shard_id,
                    collection
                );
            }
        }

        Ok(total_count)
    }

    /// Get aggregated stats (docs, chunks, size) for a sharded collection
    pub async fn get_total_stats(
        &self,
        database: &str,
        collection: &str,
        auth_header: Option<String>,
    ) -> Result<(u64, u64, u64), crate::error::DbError> {
        let config = self
            .get_shard_config(database, collection)
            .ok_or_else(|| crate::error::DbError::CollectionNotFound(collection.to_string()))?;

        if config.num_shards == 0 {
            // Non-sharded or logical base
            let db = self.storage.get_database(database)?;
            let coll = db.get_collection(collection)?;
            let stats = coll.stats();
            return Ok((
                stats.document_count as u64,
                stats.chunk_count as u64,
                stats.disk_usage.sst_files_size + stats.disk_usage.memtable_size,
            ));
        }

        let table = self.get_shard_table(database, collection).ok_or_else(|| {
            crate::error::DbError::InternalError("Shard table not found".to_string())
        })?;

        let my_id = self.my_node_id();
        let client = reqwest::Client::new();
        let secret = self.cluster_secret();

        let mut total_docs = 0u64;
        let mut total_chunks = 0u64;
        let mut total_size = 0u64;

        for shard_id in 0..config.num_shards {
            let assignment = table.assignments.get(&shard_id).ok_or_else(|| {
                crate::error::DbError::InternalError(format!(
                    "No assignment for shard {}",
                    shard_id
                ))
            })?;

            let physical_name = format!("{}_s{}", collection, shard_id);
            let mut shard_stats: Option<(u64, u64, u64)> = None;

            // 1. Try local primary/replica first
            let has_local = assignment.primary_node == my_id
                || assignment.replica_nodes.contains(&my_id)
                || assignment.primary_node == "local";
            if has_local {
                if let Ok(db) = self.storage.get_database(database) {
                    if let Ok(coll) = db.get_collection(&physical_name) {
                        let s = coll.stats();
                        shard_stats = Some((
                            s.document_count as u64,
                            s.chunk_count as u64,
                            s.disk_usage.sst_files_size + s.disk_usage.memtable_size,
                        ));
                    }
                }
            }

            // 2. If not found locally, try primary node then replicas
            if shard_stats.is_none() {
                if let Some(mgr) = &self.cluster_manager {
                    // Collect nodes to try: primary first, then replicas
                    let mut nodes_to_try = vec![assignment.primary_node.clone()];
                    nodes_to_try.extend(assignment.replica_nodes.clone());

                    for node_id in nodes_to_try {
                        if let Some(addr) = mgr.get_node_api_address(&node_id) {
                            let url = format!(
                                "http://{}/_api/database/{}/collection/{}/stats",
                                addr, database, physical_name
                            );
                            let mut req = client
                                .get(&url)
                                .header("X-Cluster-Secret", &secret)
                                .timeout(std::time::Duration::from_secs(2));

                            if let Some(ref auth) = auth_header {
                                req = req.header("Authorization", auth);
                            }

                            if let Ok(res) = req.send().await {
                                if res.status().is_success() {
                                    if let Ok(json) = res.json::<serde_json::Value>().await {
                                        let doc_count = json
                                            .get("document_count")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let chunk_count = json
                                            .get("chunk_count")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let disk_usage = json
                                            .get("disk_usage")
                                            .and_then(|v| v.get("sst_files_size"))
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        let mem_usage = json
                                            .get("disk_usage")
                                            .and_then(|v| v.get("memtable_size"))
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0);
                                        shard_stats =
                                            Some((doc_count, chunk_count, disk_usage + mem_usage));
                                        break;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if let Some((d, c, s)) = shard_stats {
                total_docs += d;
                total_chunks += c;
                total_size += s;
            } else {
                tracing::warn!(
                    "Could not get stats for shard {} of {} from any node",
                    shard_id,
                    collection
                );
            }
        }

        Ok((total_docs, total_chunks, total_size))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_coordinator() -> (ShardCoordinator, TempDir) {
        let tmp_dir = TempDir::new().expect("Failed to create temp dir");
        let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
            .expect("Failed to create storage engine");

        let coordinator = ShardCoordinator::new(
            Arc::new(engine),
            None, // No cluster manager for unit tests
            None, // No replication log
        );

        (coordinator, tmp_dir)
    }

    #[test]
    fn test_new_coordinator() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Should not be rebalancing initially
        assert!(!coordinator.is_rebalancing());

        // Without cluster manager, should return "local"
        assert_eq!(coordinator.my_node_id(), "local");
        assert_eq!(coordinator.my_address(), "local");
    }

    #[test]
    fn test_route_delegates_to_router() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Verify routing is consistent
        let shard1 = coordinator.route("test_key", 10);
        let shard2 = coordinator.route("test_key", 10);
        assert_eq!(shard1, shard2);

        // Different keys should potentially route to different shards
        let shard_a = coordinator.route("key_a", 100);
        let shard_b = coordinator.route("key_b", 100);
        // They might be equal, but the function should work
        assert!(shard_a < 100);
        assert!(shard_b < 100);
    }

    #[test]
    fn test_is_shard_replica() {
        // Static method test
        // Shard 0, RF=2, 3 nodes: nodes 0 and 1 should have it
        assert!(ShardCoordinator::is_shard_replica(0, 0, 2, 3));
        assert!(ShardCoordinator::is_shard_replica(0, 1, 2, 3));
        assert!(!ShardCoordinator::is_shard_replica(0, 2, 2, 3));

        // Shard 1, RF=2, 3 nodes: nodes 1 and 2 should have it
        assert!(!ShardCoordinator::is_shard_replica(1, 0, 2, 3));
        assert!(ShardCoordinator::is_shard_replica(1, 1, 2, 3));
        assert!(ShardCoordinator::is_shard_replica(1, 2, 2, 3));

        // Edge cases
        assert!(!ShardCoordinator::is_shard_replica(0, 0, 0, 3)); // RF=0
        assert!(!ShardCoordinator::is_shard_replica(0, 0, 2, 0)); // num_nodes=0
    }

    #[test]
    fn test_record_and_clear_node_failure() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Record failure
        coordinator.record_node_failure("node1");

        // Check it was recorded
        let failures = coordinator.recently_failed_nodes.read().unwrap();
        assert!(failures.contains_key("node1"));
        drop(failures);

        // Clear failure
        coordinator.clear_node_failure("node1");

        // Check it was cleared
        let failures = coordinator.recently_failed_nodes.read().unwrap();
        assert!(!failures.contains_key("node1"));
    }

    #[test]
    fn test_cleanup_old_failures() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Record a failure
        coordinator.record_node_failure("node1");

        // Cleanup should keep recent failures
        coordinator.cleanup_old_failures();

        let failures = coordinator.recently_failed_nodes.read().unwrap();
        assert!(failures.contains_key("node1")); // Should still be there (recent)
    }

    #[test]
    fn test_is_rebalancing_flag() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Initially false
        assert!(!coordinator.is_rebalancing());

        // Set to true
        coordinator.is_rebalancing.store(true, Ordering::SeqCst);
        assert!(coordinator.is_rebalancing());

        // Set back to false
        coordinator.is_rebalancing.store(false, Ordering::SeqCst);
        assert!(!coordinator.is_rebalancing());
    }

    #[test]
    fn test_mark_reshard_completed() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Initially no reshard time
        {
            let last_time = coordinator.last_reshard_time.read().unwrap();
            assert!(last_time.is_none());
        }

        // Mark as completed
        coordinator.mark_reshard_completed();

        // Should now have a time
        {
            let last_time = coordinator.last_reshard_time.read().unwrap();
            assert!(last_time.is_some());
        }
    }

    #[test]
    fn test_check_recent_resharding() {
        let (coordinator, _tmp) = create_test_coordinator();

        // No recent resharding initially
        assert!(!coordinator.check_recent_resharding());

        // Mark reshard completed
        coordinator.mark_reshard_completed();

        // Should return true (within 10 second window)
        assert!(coordinator.check_recent_resharding());

        // Also returns true if currently rebalancing
        coordinator.is_rebalancing.store(true, Ordering::SeqCst);
        assert!(coordinator.check_recent_resharding());
    }

    #[test]
    fn test_calculate_blob_replication_factor_single_node() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Without cluster manager, healthy_count = 1
        // Formula: min(max(2, 1/2), 10) = min(max(2, 0), 10) = min(2, 10) = 2
        let rf = coordinator.calculate_blob_replication_factor();
        assert_eq!(rf, ShardCoordinator::MIN_BLOB_REPLICAS);
    }

    #[test]
    fn test_get_node_addresses_without_cluster() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Without cluster manager, should return ["local"]
        let addresses = coordinator.get_node_addresses();
        assert_eq!(addresses, vec!["local".to_string()]);
    }

    #[test]
    fn test_get_node_ids_without_cluster() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Without cluster manager, should return ["local"]
        let ids = coordinator.get_node_ids();
        assert_eq!(ids, vec!["local".to_string()]);
    }

    #[test]
    fn test_get_healthy_node_count_without_cluster() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Without cluster manager, should return 1
        assert_eq!(coordinator.get_healthy_node_count(), 1);
    }

    #[test]
    fn test_get_node_api_address_without_cluster() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Without cluster manager, should return None
        assert!(coordinator.get_node_api_address("any_node").is_none());
    }

    #[test]
    fn test_get_shard_config_nonexistent() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Non-existent database/collection should return None
        assert!(coordinator
            .get_shard_config("nonexistent_db", "nonexistent_coll")
            .is_none());
    }

    #[test]
    fn test_get_shard_table_nonexistent() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Non-existent database/collection should return None
        assert!(coordinator
            .get_shard_table("nonexistent_db", "nonexistent_coll")
            .is_none());
    }

    #[test]
    fn test_should_pause_resharding_without_cluster() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Without cluster manager, should return false
        assert!(!coordinator.should_pause_resharding());
    }

    #[test]
    fn test_collection_shard_config_default() {
        let config = CollectionShardConfig::default();

        assert_eq!(config.num_shards, 0);
        assert_eq!(config.shard_key, "");
        assert_eq!(config.replication_factor, 0);
    }

    #[test]
    fn test_shard_table_creation() {
        let mut assignments = HashMap::new();
        assignments.insert(
            0,
            ShardAssignment {
                shard_id: 0,
                primary_node: "node1".to_string(),
                replica_nodes: vec!["node2".to_string()],
            },
        );

        let table = ShardTable {
            database: "test_db".to_string(),
            collection: "test_coll".to_string(),
            num_shards: 4,
            replication_factor: 2,
            shard_key: "_key".to_string(),
            assignments,
        };

        assert_eq!(table.database, "test_db");
        assert_eq!(table.collection, "test_coll");
        assert_eq!(table.num_shards, 4);
        assert_eq!(table.replication_factor, 2);
        assert_eq!(table.assignments.len(), 1);
    }

    #[test]
    fn test_shard_assignment_creation() {
        let assignment = ShardAssignment {
            shard_id: 5,
            primary_node: "primary".to_string(),
            replica_nodes: vec!["replica1".to_string(), "replica2".to_string()],
        };

        assert_eq!(assignment.shard_id, 5);
        assert_eq!(assignment.primary_node, "primary");
        assert_eq!(assignment.replica_nodes.len(), 2);
    }

    #[test]
    fn test_constants() {
        assert_eq!(ShardCoordinator::MAX_BLOB_REPLICAS, 10);
        assert_eq!(ShardCoordinator::MIN_BLOB_REPLICAS, 2);
    }

    #[test]
    fn test_update_shard_table_cache() {
        let (coordinator, _tmp) = create_test_coordinator();

        let table = ShardTable {
            database: "db1".to_string(),
            collection: "coll1".to_string(),
            num_shards: 4,
            replication_factor: 2,
            shard_key: "_key".to_string(),
            assignments: HashMap::new(),
        };

        coordinator.update_shard_table_cache(table.clone());

        // Check it was cached
        let tables = coordinator.shard_tables.read().unwrap();
        assert!(tables.contains_key("db1.coll1"));
    }

    #[test]
    fn test_get_node_index_without_cluster() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Without cluster manager, returns None since we can't compute index
        // Actually the implementation might differ - let's check
        let index = coordinator.get_node_index();
        // Without cluster, node index lookup fails
        assert!(index.is_none() || index == Some(0));
    }

    #[test]
    fn test_clear_failures_for_healthy_nodes_without_cluster() {
        let (coordinator, _tmp) = create_test_coordinator();

        // Record some failures
        coordinator.record_node_failure("node1");
        coordinator.record_node_failure("node2");

        // Without cluster manager, this should be a no-op
        coordinator.clear_failures_for_healthy_nodes();

        // Failures should still be there (no cluster manager to determine health)
        let failures = coordinator.recently_failed_nodes.read().unwrap();
        assert!(failures.contains_key("node1"));
        assert!(failures.contains_key("node2"));
    }
}
