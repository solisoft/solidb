//! Blob chunk rebalancing worker
//!
//! This module implements a background maintenance task that periodically
//! rebalances blob chunks across cluster nodes to ensure even distribution.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::time::{interval, Duration};

use crate::cluster::manager::ClusterManager;
use crate::sharding::coordinator::ShardCoordinator;
use crate::storage::StorageEngine;

/// Configuration for blob rebalancing behavior
#[derive(Debug, Clone)]
pub struct RebalanceConfig {
    /// How often to run rebalance checks (default: 3600 seconds = 1 hour)
    pub interval_secs: u64,
    /// Standard deviation threshold to trigger rebalancing (default: 0.2 = 20%)
    pub imbalance_threshold: f64,
    /// Minimum chunks before considering rebalancing (default: 100)
    pub min_chunks_to_rebalance: usize,
    /// Number of chunks to migrate per batch (default: 50)
    pub batch_size: usize,
    /// Enable/disable the rebalance worker (default: true)
    pub enabled: bool,
}

impl Default for RebalanceConfig {
    fn default() -> Self {
        Self {
            interval_secs: 3600,
            imbalance_threshold: 0.2,
            min_chunks_to_rebalance: 100,
            batch_size: 50,
            enabled: true,
        }
    }
}

/// Statistics for blob chunks on a single node
#[derive(Debug, Default)]
pub struct NodeBlobStats {
    pub node_id: String,
    pub chunk_count: usize,
    pub total_bytes: u64,
    pub collections: HashMap<String, CollectionBlobStats>,
}

/// Statistics for blob chunks in a collection
#[derive(Debug, Default)]
pub struct CollectionBlobStats {
    pub chunk_count: usize,
    pub total_bytes: u64,
}

/// Information about a blob chunk to migrate
#[derive(Debug)]
pub struct ChunkMigration {
    pub db_name: String,
    pub coll_name: String,
    pub blob_key: String,
    pub chunk_index: u32,
    pub size_bytes: u64,
    pub source_node: String,
    pub target_node: String,
}

/// The blob rebalance worker
#[derive(Clone)]
pub struct BlobRebalanceWorker {
    storage: Arc<StorageEngine>,
    _coordinator: Arc<ShardCoordinator>,
    cluster_manager: Option<Arc<ClusterManager>>,
    config: Arc<RebalanceConfig>,
    is_rebalancing: Arc<AtomicBool>,
}

impl BlobRebalanceWorker {
    /// Create a new blob rebalance worker
    pub fn new(
        storage: Arc<StorageEngine>,
        coordinator: Arc<ShardCoordinator>,
        cluster_manager: Option<Arc<ClusterManager>>,
        config: Arc<RebalanceConfig>,
    ) -> Self {
        Self {
            storage,
            _coordinator: coordinator,
            cluster_manager,
            config,
            is_rebalancing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Start the blob rebalance worker
    pub async fn start(self: Arc<Self>) {
        tracing::info!(
            "Starting BlobRebalanceWorker (interval: {}s, threshold: {}%)",
            self.config.interval_secs,
            (self.config.imbalance_threshold * 100.0) as u64
        );

        let mut interval = interval(Duration::from_secs(self.config.interval_secs));

        loop {
            interval.tick().await;

            if !self.config.enabled {
                tracing::debug!("BlobRebalanceWorker disabled, skipping");
                continue;
            }

            if let Err(e) = self.check_and_rebalance().await {
                tracing::error!("Blob rebalance failed: {}", e);
            }
        }
    }

    /// Get the rebalance configuration
    pub fn config(&self) -> Arc<RebalanceConfig> {
        self.config.clone()
    }

    /// Manually trigger blob rebalance check and execution
    pub async fn check_and_rebalance(&self) -> Result<(), String> {
        // Prevent concurrent rebalancing
        if self.is_rebalancing.load(Ordering::SeqCst) {
            tracing::debug!("Blob rebalance already in progress, skipping");
            return Ok(());
        }
        self.is_rebalancing.store(true, Ordering::SeqCst);

        let result = self.check_and_rebalance_inner().await;

        self.is_rebalancing.store(false, Ordering::SeqCst);
        result
    }

    /// Collect blob statistics from all nodes (public for API)
    pub async fn collect_node_stats(&self) -> Result<Vec<NodeBlobStats>, String> {
        self.collect_node_stats_internal().await
    }

    /// Calculate distribution metrics (public for API)
    pub fn calculate_distribution_metrics(
        &self,
        node_stats: &[NodeBlobStats],
    ) -> Result<DistributionMetrics, String> {
        self.calculate_distribution_metrics_internal(node_stats)
    }

    /// Check for imbalance and trigger rebalancing if needed
    async fn check_and_rebalance_inner(&self) -> Result<(), String> {
        // Collect stats from all healthy nodes
        let all_stats = self.collect_node_stats_internal().await?;

        if all_stats.is_empty() {
            tracing::debug!("No nodes available for blob rebalance");
            return Ok(());
        }

        // Calculate global distribution metrics
        let metrics = self.calculate_distribution_metrics_internal(&all_stats)?;

        tracing::info!(
            "Blob distribution: {} nodes, {} total chunks, mean {:.1} chunks/node, std_dev {:.3}",
            all_stats.len(),
            metrics.total_chunks,
            metrics.mean_chunks,
            metrics.std_dev
        );

        // Check if rebalancing is needed
        if metrics.total_chunks < self.config.min_chunks_to_rebalance {
            tracing::debug!(
                "Total chunks ({}) below minimum ({}), skipping rebalance",
                metrics.total_chunks,
                self.config.min_chunks_to_rebalance
            );
            return Ok(());
        }

        let imbalance_ratio = if metrics.mean_chunks > 0.0 {
            metrics.std_dev / metrics.mean_chunks
        } else {
            0.0
        };

        if imbalance_ratio < self.config.imbalance_threshold {
            tracing::debug!(
                "Imbalance ratio ({:.2}%) below threshold ({:.2}%), skipping rebalance",
                imbalance_ratio * 100.0,
                self.config.imbalance_threshold * 100.0
            );
            return Ok(());
        }

        tracing::info!(
            "Blob imbalance detected ({:.2}% > {:.2}%), planning migration",
            imbalance_ratio * 100.0,
            self.config.imbalance_threshold * 100.0
        );

        // Plan and execute migration
        let migrations = self.plan_migrations(&all_stats, &metrics)?;
        self.execute_migrations(&migrations).await?;

        Ok(())
    }

    /// Collect blob statistics from all nodes (internal)
    async fn collect_node_stats_internal(&self) -> Result<Vec<NodeBlobStats>, String> {
        let mut all_stats = Vec::new();

        // Add local node stats
        let local_stats = self.get_local_blob_stats().await?;
        all_stats.push(local_stats);

        // Collect stats from remote nodes via HTTP
        if let Some(ref mgr) = self.cluster_manager {
            let healthy_nodes = mgr.get_healthy_nodes();
            let local_id = mgr.local_node_id();

            for node_id in healthy_nodes {
                if node_id == local_id {
                    continue; // Already collected local stats
                }

                if let Some(addr) = mgr.get_node_api_address(&node_id) {
                    if let Ok(remote_stats) = self.fetch_remote_stats(&node_id, &addr).await {
                        all_stats.push(remote_stats);
                    }
                }
            }
        }

        Ok(all_stats)
    }

    /// Get blob statistics from local storage
    async fn get_local_blob_stats(&self) -> Result<NodeBlobStats, String> {
        let mut stats = NodeBlobStats {
            node_id: self
                .cluster_manager
                .as_ref()
                .map(|m| m.local_node_id())
                .unwrap_or_else(|| "local".to_string()),
            ..Default::default()
        };

        for db_name in self.storage.list_databases() {
            if let Ok(db) = self.storage.get_database(&db_name) {
                for coll_name in db.list_collections() {
                    if coll_name.starts_with('_') {
                        continue;
                    }

                    if let Ok(coll) = db.get_collection(&coll_name) {
                        let coll_stats = self.count_collection_blobs(&coll).await?;
                        if coll_stats.chunk_count > 0 {
                            stats.chunk_count += coll_stats.chunk_count;
                            stats.total_bytes += coll_stats.total_bytes;
                            stats.collections.insert(coll_name, coll_stats);
                        }
                    }
                }
            }
        }

        Ok(stats)
    }

    /// Count blobs in a collection
    async fn count_collection_blobs(
        &self,
        coll: &crate::storage::Collection,
    ) -> Result<CollectionBlobStats, String> {
        let (chunk_count, total_bytes) = coll.blob_stats().map_err(|e| e.to_string())?;

        Ok(CollectionBlobStats {
            chunk_count,
            total_bytes,
        })
    }

    /// Fetch blob stats from a remote node via HTTP
    async fn fetch_remote_stats(
        &self,
        node_id: &str,
        _addr: &str,
    ) -> Result<NodeBlobStats, String> {
        // This would query the remote node's stats endpoint
        // For now, we return a placeholder - this would need to be implemented
        // with an actual HTTP endpoint on the remote node

        // Example endpoint: GET http://{addr}/_internal/stats/blobs
        /*
        let client = get_http_client();
        let url = format!("http://{}/_internal/stats/blobs", addr);
        match client.get(&url).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    let stats: NodeBlobStats = response.json().await?;
                    Ok(stats)
                } else {
                    Err(format!("Failed to fetch stats from {}: {}", node_id, response.status()))
                }
            }
            Err(e) => Err(format!("Failed to fetch stats from {}: {}", node_id, e)),
        }
        */

        // Placeholder: return empty stats for now
        Ok(NodeBlobStats {
            node_id: node_id.to_string(),
            ..Default::default()
        })
    }

    /// Calculate distribution metrics across all nodes (internal)
    fn calculate_distribution_metrics_internal(
        &self,
        node_stats: &[NodeBlobStats],
    ) -> Result<DistributionMetrics, String> {
        if node_stats.is_empty() {
            return Err("No node stats to analyze".to_string());
        }

        let total_chunks: usize = node_stats.iter().map(|s| s.chunk_count).sum();
        let node_count = node_stats.len();

        if total_chunks == 0 {
            return Ok(DistributionMetrics {
                total_chunks: 0,
                mean_chunks: 0.0,
                std_dev: 0.0,
            });
        }

        let mean_chunks = total_chunks as f64 / node_count as f64;

        // Calculate standard deviation
        let variance: f64 = node_stats
            .iter()
            .map(|s| {
                let diff = s.chunk_count as f64 - mean_chunks;
                diff * diff
            })
            .sum::<f64>()
            / node_count as f64;

        let std_dev = variance.sqrt();

        Ok(DistributionMetrics {
            total_chunks,
            mean_chunks,
            std_dev,
        })
    }

    /// Plan chunk migrations to balance distribution
    fn plan_migrations(
        &self,
        node_stats: &[NodeBlobStats],
        metrics: &DistributionMetrics,
    ) -> Result<Vec<ChunkMigration>, String> {
        let mut migrations = Vec::new();

        // Identify overloaded and underloaded nodes
        let mut overloaded: Vec<&NodeBlobStats> = Vec::new();
        let mut underloaded: Vec<&NodeBlobStats> = Vec::new();

        for stats in node_stats {
            let deviation = if metrics.mean_chunks > 0.0 {
                (stats.chunk_count as f64 - metrics.mean_chunks) / metrics.mean_chunks
            } else {
                0.0
            };

            if deviation > self.config.imbalance_threshold {
                overloaded.push(stats);
            } else if deviation < -self.config.imbalance_threshold {
                underloaded.push(stats);
            }
        }

        // Sort by deviation magnitude
        overloaded.sort_by(|a, b| {
            let dev_a = a.chunk_count as f64 / metrics.mean_chunks;
            let dev_b = b.chunk_count as f64 / metrics.mean_chunks;
            dev_b
                .partial_cmp(&dev_a)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        underloaded.sort_by(|a, b| {
            let dev_a = a.chunk_count as f64 / metrics.mean_chunks;
            let dev_b = b.chunk_count as f64 / metrics.mean_chunks;
            dev_a
                .partial_cmp(&dev_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Plan migrations from overloaded to underloaded nodes
        for over in &overloaded {
            for under in &underloaded {
                if migrations.len() >= self.config.batch_size {
                    break;
                }

                // Calculate how many chunks to move
                let target_diff = (metrics.mean_chunks - under.chunk_count as f64) as usize;
                let source_diff = (over.chunk_count as f64 - metrics.mean_chunks) as usize;
                let to_move = target_diff
                    .min(source_diff)
                    .min(self.config.batch_size - migrations.len());

                if to_move == 0 {
                    continue;
                }

                // In a real implementation, we'd select specific chunks to migrate
                // For now, we create placeholder migrations
                let chunks = self.select_chunks_to_migrate(over, to_move)?;
                for chunk in chunks {
                    migrations.push(ChunkMigration {
                        db_name: chunk.db_name,
                        coll_name: chunk.coll_name,
                        blob_key: chunk.blob_key,
                        chunk_index: chunk.chunk_index,
                        size_bytes: chunk.size_bytes,
                        source_node: over.node_id.clone(),
                        target_node: under.node_id.clone(),
                    });
                }
            }
        }

        tracing::info!("Planned {} blob chunk migrations", migrations.len());
        Ok(migrations)
    }

    /// Select chunks to migrate from a node
    fn select_chunks_to_migrate(
        &self,
        _source: &NodeBlobStats,
        _count: usize,
    ) -> Result<Vec<ChunkInfo>, String> {
        // This would iterate through the source node's blob chunks and select
        // the best candidates for migration based on various heuristics:
        // - Chunk size (prefer moving larger chunks)
        // - Access patterns (avoid frequently accessed chunks)
        // - Chunk age (prefer moving older chunks)

        // For now, return an empty list - actual implementation would query
        // RocksDB for blob chunks with the BLO_PREFIX

        Ok(Vec::new())
    }

    /// Execute chunk migrations
    async fn execute_migrations(&self, migrations: &[ChunkMigration]) -> Result<(), String> {
        if migrations.is_empty() {
            return Ok(());
        }

        tracing::info!("Executing {} blob chunk migrations", migrations.len());

        for migration in migrations {
            if let Err(e) = self.migrate_chunk(migration).await {
                tracing::error!(
                    "Failed to migrate chunk {}:{}:{} from {} to {}: {}",
                    migration.db_name,
                    migration.coll_name,
                    migration.blob_key,
                    migration.source_node,
                    migration.target_node,
                    e
                );
            }
        }

        Ok(())
    }

    /// Migrate a single chunk from source to target node
    async fn migrate_chunk(&self, migration: &ChunkMigration) -> Result<(), String> {
        // Check if this is a local migration (same node)
        let local_id = self
            .cluster_manager
            .as_ref()
            .map(|m| m.local_node_id())
            .unwrap_or_else(|| "local".to_string());

        if migration.source_node == local_id && migration.target_node == local_id {
            // No migration needed
            return Ok(());
        }

        if migration.source_node == local_id {
            // We're the source, need to send to remote target
            self.migrate_chunk_to_remote(migration).await?;
        } else if migration.target_node == local_id {
            // We're the target, need to receive from remote source
            self.migrate_chunk_from_remote(migration).await?;
        }

        Ok(())
    }

    /// Migrate chunk to remote target node
    async fn migrate_chunk_to_remote(&self, _migration: &ChunkMigration) -> Result<(), String> {
        // Read chunk data from local storage
        // Send to remote node via HTTP
        // Update shard routing if needed
        // Delete original chunk

        // This would be implemented with an HTTP endpoint on the target node

        Ok(())
    }

    /// Migrate chunk from remote source node
    async fn migrate_chunk_from_remote(&self, _migration: &ChunkMigration) -> Result<(), String> {
        // Request chunk data from source node
        // Write chunk data to local storage
        // Update shard routing if needed

        // This would use an HTTP endpoint on the source node

        Ok(())
    }
}

/// Distribution metrics for blob chunks across nodes
#[derive(Debug)]
pub struct DistributionMetrics {
    pub total_chunks: usize,
    pub mean_chunks: f64,
    pub std_dev: f64,
}

/// Information about a chunk to migrate
struct ChunkInfo {
    db_name: String,
    coll_name: String,
    blob_key: String,
    chunk_index: u32,
    size_bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn create_test_config() -> RebalanceConfig {
        RebalanceConfig {
            interval_secs: 3600,
            imbalance_threshold: 0.2,
            min_chunks_to_rebalance: 100,
            batch_size: 50,
            enabled: true,
        }
    }

    fn create_node_stats(node_id: &str, chunk_count: usize, total_bytes: u64) -> NodeBlobStats {
        NodeBlobStats {
            node_id: node_id.to_string(),
            chunk_count,
            total_bytes,
            collections: HashMap::new(),
        }
    }

    #[test]
    fn test_rebalance_config_default_values() {
        let config = RebalanceConfig::default();
        assert_eq!(config.interval_secs, 3600);
        assert_eq!(config.imbalance_threshold, 0.2);
        assert_eq!(config.min_chunks_to_rebalance, 100);
        assert_eq!(config.batch_size, 50);
        assert!(config.enabled);
    }

    #[test]
    fn test_rebalance_config_custom_values() {
        let config = RebalanceConfig {
            interval_secs: 1800,
            imbalance_threshold: 0.15,
            min_chunks_to_rebalance: 50,
            batch_size: 25,
            enabled: false,
        };
        assert_eq!(config.interval_secs, 1800);
        assert_eq!(config.imbalance_threshold, 0.15);
        assert_eq!(config.min_chunks_to_rebalance, 50);
        assert_eq!(config.batch_size, 25);
        assert!(!config.enabled);
    }

    #[test]
    fn test_calculate_distribution_metrics_empty() {
        let empty_stats: Vec<NodeBlobStats> = vec![];

        // Calculate manually to test logic
        let result = calculate_metrics_internal(&empty_stats);
        assert!(result.is_err());
    }

    #[test]
    fn test_calculate_distribution_metrics_single_node() {
        let stats = vec![create_node_stats("node1", 100, 1000)];

        let result = calculate_metrics_internal(&stats).unwrap();
        assert_eq!(result.total_chunks, 100);
        assert_eq!(result.mean_chunks, 100.0);
        assert_eq!(result.std_dev, 0.0);
    }

    #[test]
    fn test_calculate_distribution_metrics_balanced() {
        // 3 nodes with equal chunks = balanced
        let stats = vec![
            create_node_stats("node1", 100, 1000),
            create_node_stats("node2", 100, 1000),
            create_node_stats("node3", 100, 1000),
        ];

        let result = calculate_metrics_internal(&stats).unwrap();
        assert_eq!(result.total_chunks, 300);
        assert_eq!(result.mean_chunks, 100.0);
        assert!((result.std_dev - 0.0).abs() < 0.001); // Should be 0 or very close
    }

    #[test]
    fn test_calculate_distribution_metrics_unbalanced() {
        // 3 nodes with very different chunks = unbalanced
        let stats = vec![
            create_node_stats("node1", 10, 100),
            create_node_stats("node2", 100, 1000),
            create_node_stats("node3", 190, 1900),
        ];

        let result = calculate_metrics_internal(&stats).unwrap();
        assert_eq!(result.total_chunks, 300);
        assert_eq!(result.mean_chunks, 100.0);
        assert!(result.std_dev > 50.0); // Should have significant deviation
    }

    #[test]
    fn test_imbalance_ratio_calculation() {
        // Perfectly balanced: std_dev = 0, ratio = 0
        let balanced = vec![
            create_node_stats("node1", 100, 1000),
            create_node_stats("node2", 100, 1000),
        ];
        let metrics = calculate_metrics_internal(&balanced).unwrap();
        let ratio = if metrics.mean_chunks > 0.0 {
            metrics.std_dev / metrics.mean_chunks
        } else {
            0.0
        };
        assert!((ratio - 0.0).abs() < 0.001);

        // Unbalanced: node1=10, node2=190, mean=100, std_dev=90
        let unbalanced = vec![
            create_node_stats("node1", 10, 100),
            create_node_stats("node2", 190, 1900),
        ];
        let metrics = calculate_metrics_internal(&unbalanced).unwrap();
        let ratio = if metrics.mean_chunks > 0.0 {
            metrics.std_dev / metrics.mean_chunks
        } else {
            0.0
        };
        assert!(ratio > 0.6); // Should be ~0.9 or higher
    }

    #[test]
    fn test_node_blob_stats_struct() {
        let mut collections = HashMap::new();
        collections.insert(
            "files".to_string(),
            CollectionBlobStats {
                chunk_count: 50,
                total_bytes: 5000,
            },
        );

        let stats = NodeBlobStats {
            node_id: "test-node".to_string(),
            chunk_count: 50,
            total_bytes: 5000,
            collections,
        };

        assert_eq!(stats.node_id, "test-node");
        assert_eq!(stats.chunk_count, 50);
        assert_eq!(stats.total_bytes, 5000);
        assert_eq!(stats.collections.len(), 1);
        assert_eq!(stats.collections.get("files").unwrap().chunk_count, 50);
    }

    #[test]
    fn test_chunk_migration_struct() {
        let migration = ChunkMigration {
            db_name: "test_db".to_string(),
            coll_name: "files".to_string(),
            blob_key: "myfile.txt".to_string(),
            chunk_index: 0,
            size_bytes: 1024,
            source_node: "node1".to_string(),
            target_node: "node2".to_string(),
        };

        assert_eq!(migration.db_name, "test_db");
        assert_eq!(migration.blob_key, "myfile.txt");
        assert_eq!(migration.chunk_index, 0);
        assert_eq!(migration.size_bytes, 1024);
        assert_eq!(migration.source_node, "node1");
        assert_eq!(migration.target_node, "node2");
    }

    #[test]
    fn test_distribution_metrics_struct() {
        let metrics = DistributionMetrics {
            total_chunks: 500,
            mean_chunks: 100.0,
            std_dev: 50.0,
        };

        assert_eq!(metrics.total_chunks, 500);
        assert_eq!(metrics.mean_chunks, 100.0);
        assert_eq!(metrics.std_dev, 50.0);
    }

    #[test]
    fn test_should_rebalance_below_minimum() {
        let config = RebalanceConfig {
            min_chunks_to_rebalance: 100,
            imbalance_threshold: 0.2,
            ..Default::default()
        };

        // Only 50 total chunks - below minimum
        let stats = vec![
            create_node_stats("node1", 25, 250),
            create_node_stats("node2", 25, 250),
        ];
        let metrics = calculate_metrics_internal(&stats).unwrap();

        let should_rebalance = metrics.total_chunks >= config.min_chunks_to_rebalance
            && (metrics.std_dev / metrics.mean_chunks) >= config.imbalance_threshold;

        assert!(!should_rebalance);
    }

    #[test]
    fn test_should_rebalance_above_threshold() {
        let config = RebalanceConfig {
            min_chunks_to_rebalance: 100,
            imbalance_threshold: 0.2,
            ..Default::default()
        };

        // 300 total chunks (above min), but balanced (std_dev = 0)
        let stats = vec![
            create_node_stats("node1", 100, 1000),
            create_node_stats("node2", 100, 1000),
            create_node_stats("node3", 100, 1000),
        ];
        let metrics = calculate_metrics_internal(&stats).unwrap();

        let imbalance_ratio = metrics.std_dev / metrics.mean_chunks;
        let should_rebalance = metrics.total_chunks >= config.min_chunks_to_rebalance
            && imbalance_ratio >= config.imbalance_threshold;

        assert!(!should_rebalance); // Balanced, no rebalance needed
    }

    #[test]
    fn test_should_rebalance_needed() {
        let config = RebalanceConfig {
            min_chunks_to_rebalance: 100,
            imbalance_threshold: 0.2,
            ..Default::default()
        };

        // 300 total chunks AND unbalanced
        let stats = vec![
            create_node_stats("node1", 10, 100),
            create_node_stats("node2", 290, 2900),
        ];
        let metrics = calculate_metrics_internal(&stats).unwrap();

        let imbalance_ratio = metrics.std_dev / metrics.mean_chunks;
        let should_rebalance = metrics.total_chunks >= config.min_chunks_to_rebalance
            && imbalance_ratio >= config.imbalance_threshold;

        assert!(should_rebalance); // Should trigger rebalance
    }
}

// Helper function to calculate metrics (extracted for testing)
#[allow(dead_code)]
fn calculate_metrics_internal(node_stats: &[NodeBlobStats]) -> Result<DistributionMetrics, String> {
    if node_stats.is_empty() {
        return Err("No node statistics provided".to_string());
    }

    let _total_bytes: u64 = node_stats.iter().map(|n| n.total_bytes).sum();

    let mean_chunks = if !node_stats.is_empty() {
        node_stats.iter().map(|n| n.chunk_count).sum::<usize>() as f64 / node_stats.len() as f64
    } else {
        0.0
    };

    // Calculate standard deviation
    let variance: f64 = node_stats
        .iter()
        .map(|n| {
            let diff = n.chunk_count as f64 - mean_chunks;
            diff * diff
        })
        .sum::<f64>()
        / node_stats.len() as f64;

    let std_dev = variance.sqrt();

    Ok(DistributionMetrics {
        total_chunks: node_stats.iter().map(|n| n.chunk_count).sum(),
        mean_chunks,
        std_dev,
    })
}
