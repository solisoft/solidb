//! Shard cleanup service for removing documents this node isn't responsible for
//!
//! When a node acts as a "write buffer" (accepting inserts regardless of shard responsibility),
//! this background task periodically cleans up documents that belong to other nodes.

use std::sync::Arc;
use std::time::Duration;
use tracing::{info, debug, error};

use crate::cluster::manager::ClusterManager;
use crate::sharding::router::ShardRouter;
use crate::storage::StorageEngine;

/// Configuration for shard cleanup
pub struct ShardCleanupConfig {
    /// How often to run cleanup (default: 60 seconds)
    pub cleanup_interval: Duration,
    /// Maximum documents to process per cleanup cycle
    pub batch_size: usize,
}

impl Default for ShardCleanupConfig {
    fn default() -> Self {
        Self {
            cleanup_interval: Duration::from_secs(60),
            batch_size: 1000,
        }
    }
}

/// Background service that removes documents this node isn't responsible for
pub struct ShardCleanup {
    config: ShardCleanupConfig,
    storage: Arc<StorageEngine>,
    cluster_manager: Arc<ClusterManager>,
}

impl ShardCleanup {
    pub fn new(
        config: ShardCleanupConfig,
        storage: Arc<StorageEngine>,
        cluster_manager: Arc<ClusterManager>,
    ) -> Self {
        Self {
            config,
            storage,
            cluster_manager,
        }
    }

    /// Start the cleanup background loop
    pub async fn start(self) {
        info!("[SHARD-CLEANUP] Starting shard cleanup service (interval: {:?})", self.config.cleanup_interval);
        
        loop {
            tokio::time::sleep(self.config.cleanup_interval).await;
            
            if let Err(e) = self.run_cleanup_cycle().await {
                error!("[SHARD-CLEANUP] Cleanup cycle failed: {}", e);
            }
        }
    }

    async fn run_cleanup_cycle(&self) -> anyhow::Result<()> {
        // Get cluster topology
        let members = self.cluster_manager.state().get_all_members();
        if members.len() <= 1 {
            // Single node cluster, no cleanup needed
            return Ok(());
        }

        let mut all_nodes: Vec<String> = members.iter()
            .map(|m| m.node.address.clone())
            .collect();
        all_nodes.sort();
        
        let my_id = self.cluster_manager.local_node_id();
        let my_addr = self.cluster_manager.state().get_member(&my_id)
            .map(|m| m.node.address.clone())
            .unwrap_or_else(|| "unknown".to_string());
        
        let my_idx = match all_nodes.iter().position(|n| n == &my_addr) {
            Some(idx) => idx,
            None => {
                debug!("[SHARD-CLEANUP] Can't find self in cluster, skipping cleanup");
                return Ok(());
            }
        };
        
        let num_nodes = all_nodes.len();
        let mut total_deleted = 0usize;

        // Iterate through all databases and collections
        for db_name in self.storage.list_databases() {
            let db = match self.storage.get_database(&db_name) {
                Ok(db) => db,
                Err(_) => continue,
            };

            for coll_name in db.list_collections() {
                let collection = match db.get_collection(&coll_name) {
                    Ok(coll) => coll,
                    Err(_) => continue,
                };

                // Only process sharded collections
                let shard_config = match collection.get_shard_config() {
                    Some(config) if config.num_shards > 0 => config,
                    _ => continue,
                };

                // Scan documents and identify ones we're not responsible for
                let docs_to_delete: Vec<String> = collection
                    .scan(None)
                    .into_iter()
                    .filter_map(|doc| {
                        let key = &doc.key;
                        let shard_id = ShardRouter::route(key, shard_config.num_shards);
                        
                        let is_responsible = ShardRouter::is_shard_replica(
                            shard_id,
                            my_idx,
                            shard_config.replication_factor,
                            num_nodes,
                        );
                        
                        if !is_responsible {
                            Some(key.clone())
                        } else {
                            None
                        }
                    })
                    .take(self.config.batch_size)
                    .collect();

                if !docs_to_delete.is_empty() {
                    let count = docs_to_delete.len();
                    for key in docs_to_delete {
                        if let Err(e) = collection.delete(&key) {
                            debug!("[SHARD-CLEANUP] Failed to delete {}: {}", key, e);
                        } else {
                            total_deleted += 1;
                        }
                    }
                    info!(
                        "[SHARD-CLEANUP] Removed {} foreign documents from {}/{} (my_idx={}, RF={}, num_nodes={})",
                        count, db_name, coll_name, my_idx, shard_config.replication_factor, num_nodes
                    );
                }
            }
        }

        if total_deleted > 0 {
            info!("[SHARD-CLEANUP] Cleanup cycle complete: deleted {} total documents", total_deleted);
        }

        Ok(())
    }
}
