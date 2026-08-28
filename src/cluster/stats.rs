use crate::cluster::manager::ClusterManager;
use crate::cluster::stats_gate::StatsGate;
use crate::sharding::ShardCoordinator;
use crate::storage::engine::StorageEngine;
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeBasicStats {
    pub total_chunk_count: u64,
    pub total_file_count: u64,
    pub storage_bytes: u64,
    pub total_memtable_size: u64,
    pub total_live_size: u64,
    pub cpu_usage_percent: f32,
    pub memory_used_mb: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ShardStat {
    pub id: usize,
    pub primary: String,
    pub replicas: Vec<String>,
    pub status: String, // "Ready", "Syncing", "Migrating"
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CollectionStats {
    pub name: String,
    pub database: String,
    pub shard_count: usize,
    pub replication_factor: usize,
    pub document_count: u64,
    pub chunk_count: u64,
    pub storage_bytes: u64,
    pub shards: Vec<ShardStat>,
    pub status: String,
    pub actions: Vec<String>, // "Rebalancing", etc.
}

/// How long a collection whose counters never move may go without having its
/// `_cluster_informations` document recomputed. Bounds the staleness of the
/// figures the gate cannot see changing (on-disk size after a compaction,
/// in-place document updates).
const FULL_REFRESH: Duration = Duration::from_secs(300);

pub struct ClusterStatsCollector {
    storage: Arc<StorageEngine>,
    coordinator: Arc<ShardCoordinator>,
    // manager: Arc<ClusterManager>, // Unused for now but might need node status
    /// Per-collection gate holding a hash of the last document written, so an
    /// unchanged collection costs neither the stats gathering nor the write.
    gate: StatsGate<u64>,
}

impl ClusterStatsCollector {
    pub fn new(
        storage: Arc<StorageEngine>,
        coordinator: Arc<ShardCoordinator>,
        _manager: Arc<ClusterManager>,
    ) -> Self {
        Self {
            storage,
            coordinator,
            gate: StatsGate::new(FULL_REFRESH),
        }
    }

    pub async fn start(self) {
        let mut tick = interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            if let Err(e) = self.collect_and_store().await {
                error!("Failed to collect cluster stats: {}", e);
            }
        }
    }

    async fn collect_and_store(&self) -> anyhow::Result<()> {
        // One pass over the column families instead of one per database: with
        // 89 databases over 1718 collections, per-database listing walked the
        // full CF list 89 times and cloned every name each time.
        let grouped = self.storage.collections_grouped();

        // Resolve the destination once per cycle rather than once per
        // collection.
        let sys_db = self.storage.get_database("_system")?;
        if sys_db.get_collection("_cluster_informations").is_err() {
            sys_db.create_collection("_cluster_informations".to_string(), None)?;
        }
        let sys_coll = sys_db.get_collection("_cluster_informations")?;

        let mut live: HashSet<String> = HashSet::new();

        // Drive from the database registry, not from the grouped column
        // families: a CF can outlive its database entry (an interrupted drop
        // leaves `<db>:<coll>` behind with no `db:` key), and such a name must
        // not be mistaken for a live database.
        for db_name in self.storage.list_databases() {
            let Ok(db) = self.storage.get_database(&db_name) else {
                continue;
            };
            let Some(coll_names) = grouped.get(&db_name) else {
                continue;
            };

            for coll_name in coll_names {
                // Hide physical shard collections (they end with _sN where N is a number)
                // These are summarized within their logical collection
                if is_physical_shard_collection(coll_name) {
                    continue;
                }

                // Deterministic ID: "db_coll"
                let doc_id = format!("{}_{}", db_name, coll_name);
                live.insert(doc_id.clone());

                let Ok(coll) = db.system_collection(coll_name) else {
                    continue;
                };

                // A sharded collection keeps its documents in the physical
                // shard CFs (skipped above) or on other nodes, so the logical
                // collection's local counters never move and cannot gate it.
                let sharded = coll
                    .get_shard_config()
                    .map(|c| c.num_shards > 0)
                    .unwrap_or(false);
                let counts = (coll.count(), coll.chunk_count());
                if !sharded && !self.gate.needs_refresh(&doc_id, counts) {
                    continue;
                }

                // Get Sharding Info from Coordinator
                let shard_table = self.coordinator.get_shard_table(&db_name, coll_name);

                // Get Cluster-wide stats
                let (document_count, chunk_count, storage_bytes) = self
                    .coordinator
                    .get_total_stats(&db_name, coll_name, None)
                    .await
                    .unwrap_or((0, 0, 0));

                let mut shard_stats = Vec::new();
                let mut shard_count = 0;
                let mut replication_factor = 1;

                if let Some(table) = shard_table {
                    shard_count = table.num_shards as usize;
                    replication_factor = table.replication_factor as usize;

                    // assignments is HashMap<u16, ShardAssignment>
                    // We want to sort by ID
                    let mut assignments: Vec<_> = table.assignments.values().collect();
                    assignments.sort_by_key(|a| a.shard_id);

                    for assignment in assignments {
                        shard_stats.push(ShardStat {
                            id: assignment.shard_id as usize,
                            primary: assignment.primary_node.clone(),
                            replicas: assignment.replica_nodes.clone(),
                            status: "Ready".to_string(),
                        });
                    }
                } else {
                    // Non-sharded or local collection?
                    // Represent as single shard on local node?
                    // For now, empty or default.
                    shard_stats.push(ShardStat {
                        id: 0,
                        primary: "local".to_string(), // Or local node ID?
                        replicas: vec![],
                        status: "Ready".to_string(),
                    });
                }

                let stats = CollectionStats {
                    name: coll_name.clone(),
                    database: db_name.clone(),
                    shard_count,
                    replication_factor,
                    document_count,
                    chunk_count,
                    storage_bytes,
                    shards: shard_stats,
                    status: "Ready".to_string(),
                    actions: vec![],
                };

                let json = serde_json::to_value(&stats)?;

                // Recomputing is cheap; the write is not. A periodic refresh
                // that lands on identical figures must not touch RocksDB.
                let digest = digest_of(&json);
                if self.gate.cached(&doc_id) != Some(digest) {
                    // Store in _system/_cluster_informations. There is no
                    // replace, so delete first when the document exists.
                    if sys_coll.get(&doc_id).is_ok() {
                        sys_coll.delete(&doc_id)?;
                    }

                    // Add _key to json
                    let mut doc = json.as_object().unwrap().clone();
                    doc.insert(
                        "_key".to_string(),
                        serde_json::Value::String(doc_id.clone()),
                    );

                    sys_coll.insert(serde_json::Value::Object(doc))?;
                }

                self.gate.record(&doc_id, counts, digest);
            }
        }

        // Drop gate entries for collections that no longer exist.
        self.gate.retain(&live);

        Ok(())
    }
}

/// Hash of a stats document, used to skip writes that would not change it.
fn digest_of(value: &serde_json::Value) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    hasher.finish()
}

fn is_physical_shard_collection(name: &str) -> bool {
    if let Some(pos) = name.rfind("_s") {
        let suffix = &name[pos + 2..];
        !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_physical_shard_collection() {
        // Should match physical shard collections
        assert!(is_physical_shard_collection("users_s0"));
        assert!(is_physical_shard_collection("users_s1"));
        assert!(is_physical_shard_collection("users_s10"));
        assert!(is_physical_shard_collection("orders_s99"));

        // Should not match regular collections
        assert!(!is_physical_shard_collection("users"));
        assert!(!is_physical_shard_collection("_system"));
        assert!(!is_physical_shard_collection("user_settings"));

        // Should not match non-numeric suffixes
        assert!(!is_physical_shard_collection("users_shard"));
        assert!(!is_physical_shard_collection("users_s"));
        assert!(!is_physical_shard_collection("users_ss1"));
    }

    #[test]
    fn test_node_basic_stats_default() {
        let stats = NodeBasicStats {
            total_chunk_count: 100,
            total_file_count: 50,
            storage_bytes: 1024 * 1024,
            total_memtable_size: 4096,
            total_live_size: 512,
            cpu_usage_percent: 25.5,
            memory_used_mb: 256,
        };

        assert_eq!(stats.total_chunk_count, 100);
        assert_eq!(stats.total_file_count, 50);
        assert_eq!(stats.memory_used_mb, 256);
    }

    #[test]
    fn test_shard_stat() {
        let stat = ShardStat {
            id: 5,
            primary: "node1".to_string(),
            replicas: vec!["node2".to_string(), "node3".to_string()],
            status: "Ready".to_string(),
        };

        assert_eq!(stat.id, 5);
        assert_eq!(stat.primary, "node1");
        assert_eq!(stat.replicas.len(), 2);
        assert_eq!(stat.status, "Ready");
    }

    #[test]
    fn test_collection_stats() {
        let stats = CollectionStats {
            name: "users".to_string(),
            database: "mydb".to_string(),
            shard_count: 4,
            replication_factor: 2,
            document_count: 1000,
            chunk_count: 50,
            storage_bytes: 1024 * 1024 * 10,
            shards: vec![],
            status: "Ready".to_string(),
            actions: vec![],
        };

        assert_eq!(stats.name, "users");
        assert_eq!(stats.shard_count, 4);
        assert_eq!(stats.document_count, 1000);
    }

    #[test]
    fn test_node_basic_stats_serialization() {
        let stats = NodeBasicStats {
            total_chunk_count: 10,
            total_file_count: 5,
            storage_bytes: 1000,
            total_memtable_size: 500,
            total_live_size: 200,
            cpu_usage_percent: 15.0,
            memory_used_mb: 128,
        };

        let json = serde_json::to_string(&stats).unwrap();
        assert!(json.contains("total_chunk_count"));
        assert!(json.contains("cpu_usage_percent"));

        let deserialized: NodeBasicStats = serde_json::from_str(&json).unwrap();
        assert_eq!(stats.total_chunk_count, deserialized.total_chunk_count);
    }

    #[test]
    fn test_collection_stats_with_shards() {
        let shards = vec![
            ShardStat {
                id: 0,
                primary: "node1".to_string(),
                replicas: vec!["node2".to_string()],
                status: "Ready".to_string(),
            },
            ShardStat {
                id: 1,
                primary: "node2".to_string(),
                replicas: vec!["node3".to_string()],
                status: "Syncing".to_string(),
            },
        ];

        let stats = CollectionStats {
            name: "orders".to_string(),
            database: "shop".to_string(),
            shard_count: 2,
            replication_factor: 2,
            document_count: 5000,
            chunk_count: 100,
            storage_bytes: 50 * 1024 * 1024,
            shards,
            status: "Ready".to_string(),
            actions: vec!["Rebalancing".to_string()],
        };

        assert_eq!(stats.shards.len(), 2);
        assert_eq!(stats.shards[0].status, "Ready");
        assert_eq!(stats.shards[1].status, "Syncing");
        assert_eq!(stats.actions.len(), 1);
    }
}
