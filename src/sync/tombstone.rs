//! Tombstone Retention for Deleted Documents
//!
//! Manages cleanup of deleted document records (tombstones) after a TTL.
//! This prevents unlimited growth of the sync log and local storage.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Configuration for tombstone retention
#[derive(Clone, Debug)]
pub struct TombstoneConfig {
    /// How long to keep tombstones before deletion (default: 30 days)
    pub retention_period: Duration,
    /// How often to run cleanup (default: 24 hours)
    pub cleanup_interval: Duration,
    /// Maximum number of tombstones to delete per cleanup run (default: 10000)
    pub batch_size: usize,
    /// Enable automatic cleanup
    pub enabled: bool,
}

impl Default for TombstoneConfig {
    fn default() -> Self {
        Self {
            retention_period: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
            cleanup_interval: Duration::from_secs(24 * 60 * 60),      // 24 hours
            batch_size: 10000,
            enabled: true,
        }
    }
}

impl TombstoneConfig {
    /// Create config with custom retention period in days
    pub fn with_retention_days(days: u64) -> Self {
        Self {
            retention_period: Duration::from_secs(days * 24 * 60 * 60),
            ..Default::default()
        }
    }

    /// Disable automatic cleanup
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }
}

/// Tracks tombstone metadata for a deleted document
#[derive(Clone, Debug)]
pub struct Tombstone {
    /// Document key
    pub key: String,
    /// Collection name
    pub collection: String,
    /// Database name
    pub database: String,
    /// When the document was deleted
    pub deleted_at: u64,
    /// Version vector at time of deletion
    pub version_vector: String,
    /// Sync sequence number
    pub sequence: u64,
}

/// Manages tombstone retention and cleanup
pub struct TombstoneManager {
    config: TombstoneConfig,
    /// In-memory index of recent tombstones (collection -> tombstones)
    tombstones: Arc<RwLock<HashMap<String, Vec<Tombstone>>>>,
    /// Last cleanup timestamp
    last_cleanup: Arc<RwLock<u64>>,
}

impl TombstoneManager {
    /// Create a new tombstone manager
    pub fn new(config: TombstoneConfig) -> Self {
        Self {
            config,
            tombstones: Arc::new(RwLock::new(HashMap::new())),
            last_cleanup: Arc::new(RwLock::new(0)),
        }
    }

    /// Start background cleanup task
    pub async fn start_cleanup_task(&self) {
        if !self.config.enabled {
            info!("Tombstone cleanup is disabled");
            return;
        }

        let config = self.config.clone();
        let tombstones = self.tombstones.clone();
        let last_cleanup = self.last_cleanup.clone();

        tokio::spawn(async move {
            let mut cleanup_interval = interval(config.cleanup_interval);

            loop {
                cleanup_interval.tick().await;

                let now = current_timestamp();
                let last = *last_cleanup.read().await;

                // Only run cleanup if enough time has passed
                if now - last >= config.cleanup_interval.as_millis() as u64 {
                    debug!("Running tombstone cleanup...");

                    let manager = TombstoneManager {
                        config: config.clone(),
                        tombstones: tombstones.clone(),
                        last_cleanup: last_cleanup.clone(),
                    };

                    match manager.cleanup().await {
                        Ok(count) => {
                            if count > 0 {
                                info!("Cleaned up {} tombstones", count);
                            }
                            *last_cleanup.write().await = now;
                        }
                        Err(e) => {
                            error!("Tombstone cleanup failed: {}", e);
                        }
                    }
                }
            }
        });
    }

    /// Perform cleanup of old tombstones
    async fn cleanup(&self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let cutoff = current_timestamp() - self.config.retention_period.as_millis() as u64;
        let mut total_removed = 0;

        let mut tombstones = self.tombstones.write().await;

        for (collection, docs) in tombstones.iter_mut() {
            let before_count = docs.len();

            // Remove tombstones older than retention period
            docs.retain(|tombstone| {
                let keep = tombstone.deleted_at > cutoff;
                if !keep {
                    debug!(
                        "Removing tombstone: {} in {} (deleted at {})",
                        tombstone.key, collection, tombstone.deleted_at
                    );
                }
                keep
            });

            let removed = before_count - docs.len();
            total_removed += removed;

            // If we hit batch size limit, stop
            if total_removed >= self.config.batch_size {
                warn!("Tombstone cleanup batch size limit reached");
                break;
            }
        }

        // Remove empty collections from index
        tombstones.retain(|_, docs| !docs.is_empty());

        Ok(total_removed)
    }

    /// Register a new tombstone when a document is deleted
    pub async fn add_tombstone(&self, tombstone: Tombstone) {
        let mut tombstones = self.tombstones.write().await;
        let key = format!("{}.{}", tombstone.database, tombstone.collection);

        tombstones
            .entry(key)
            .or_insert_with(Vec::new)
            .push(tombstone);
    }

    /// Check if a document has a tombstone (was recently deleted)
    pub async fn has_tombstone(
        &self,
        database: &str,
        collection: &str,
        key: &str,
    ) -> Option<Tombstone> {
        let tombstones = self.tombstones.read().await;
        let coll_key = format!("{}.{}", database, collection);

        if let Some(docs) = tombstones.get(&coll_key) {
            return docs.iter().find(|t| t.key == key).cloned();
        }

        None
    }

    /// Get all tombstones for a collection
    pub async fn get_tombstones(&self, database: &str, collection: &str) -> Vec<Tombstone> {
        let tombstones = self.tombstones.read().await;
        let key = format!("{}.{}", database, collection);

        tombstones.get(&key).cloned().unwrap_or_default()
    }

    /// Get tombstone statistics
    pub async fn get_stats(&self) -> TombstoneStats {
        let tombstones = self.tombstones.read().await;

        let total_collections = tombstones.len();
        let total_tombstones: usize = tombstones.values().map(|v| v.len()).sum();

        let oldest_tombstone = tombstones
            .values()
            .flat_map(|v| v.iter())
            .map(|t| t.deleted_at)
            .min();

        let newest_tombstone = tombstones
            .values()
            .flat_map(|v| v.iter())
            .map(|t| t.deleted_at)
            .max();

        TombstoneStats {
            total_collections,
            total_tombstones,
            oldest_tombstone,
            newest_tombstone,
            retention_period_days: self.config.retention_period.as_secs() / (24 * 60 * 60),
        }
    }

    /// Force immediate cleanup (for testing or manual triggers)
    pub async fn force_cleanup(&self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        info!("Forcing tombstone cleanup...");
        let count = self.cleanup().await?;
        *self.last_cleanup.write().await = current_timestamp();
        Ok(count)
    }

    /// Update configuration
    pub fn update_config(&mut self, config: TombstoneConfig) {
        self.config = config;
    }
}

/// Statistics about tombstones
#[derive(Debug, Clone)]
pub struct TombstoneStats {
    pub total_collections: usize,
    pub total_tombstones: usize,
    pub oldest_tombstone: Option<u64>,
    pub newest_tombstone: Option<u64>,
    pub retention_period_days: u64,
}

impl std::fmt::Display for TombstoneStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Tombstones: {} in {} collections (retention: {} days)",
            self.total_tombstones, self.total_collections, self.retention_period_days
        )
    }
}

/// Get current timestamp in milliseconds
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Extension trait for collections to handle tombstones
pub trait TombstoneAware {
    /// Delete with tombstone tracking
    fn delete_with_tombstone(
        &self,
        key: &str,
        tombstone: &Tombstone,
    ) -> Result<(), Box<dyn std::error::Error>>;

    /// Check if document was recently deleted
    fn was_recently_deleted(&self, key: &str) -> bool;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tombstone(key: &str, deleted_at: u64) -> Tombstone {
        Tombstone {
            key: key.to_string(),
            collection: "test".to_string(),
            database: "mydb".to_string(),
            deleted_at,
            version_vector: "{}".to_string(),
            sequence: 1,
        }
    }

    #[tokio::test]
    async fn test_add_and_retrieve_tombstone() {
        let manager = TombstoneManager::new(TombstoneConfig::disabled());

        let tombstone = create_test_tombstone("doc-1", current_timestamp());
        manager.add_tombstone(tombstone.clone()).await;

        let found = manager.has_tombstone("mydb", "test", "doc-1").await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().key, "doc-1");
    }

    #[tokio::test]
    async fn test_cleanup_old_tombstones() {
        let config = TombstoneConfig::with_retention_days(1);
        let manager = TombstoneManager::new(config);

        let old_time = current_timestamp() - (2 * 24 * 60 * 60 * 1000); // 2 days ago
        let new_time = current_timestamp();

        // Add old tombstone
        manager
            .add_tombstone(create_test_tombstone("old-doc", old_time))
            .await;

        // Add new tombstone
        manager
            .add_tombstone(create_test_tombstone("new-doc", new_time))
            .await;

        // Cleanup
        let removed = manager.force_cleanup().await.unwrap();
        assert_eq!(removed, 1);

        // Old tombstone should be gone
        assert!(manager
            .has_tombstone("mydb", "test", "old-doc")
            .await
            .is_none());

        // New tombstone should remain
        assert!(manager
            .has_tombstone("mydb", "test", "new-doc")
            .await
            .is_some());
    }

    #[tokio::test]
    async fn test_stats() {
        let manager = TombstoneManager::new(TombstoneConfig::disabled());

        manager
            .add_tombstone(create_test_tombstone("doc-1", current_timestamp()))
            .await;
        manager
            .add_tombstone(create_test_tombstone("doc-2", current_timestamp()))
            .await;

        let stats = manager.get_stats().await;
        assert_eq!(stats.total_tombstones, 2);
        assert_eq!(stats.total_collections, 1);
    }
}
