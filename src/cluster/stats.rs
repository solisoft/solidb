use std::sync::Arc;
use tokio::time::{interval, Duration};
use tracing::error;
use serde::{Serialize, Deserialize};
use crate::storage::engine::StorageEngine;
use crate::sharding::ShardCoordinator;
use crate::cluster::manager::ClusterManager;

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

pub struct ClusterStatsCollector {
    storage: Arc<StorageEngine>,
    coordinator: Arc<ShardCoordinator>,
    // manager: Arc<ClusterManager>, // Unused for now but might need node status
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
        // 1. List all databases
        let databases = self.storage.list_databases();
        
        for db_name in databases {
            // databases is Vec<String> so db_name is String
            let db = self.storage.get_database(&db_name)?;
            // list_collections returns Vec<String> (names) or Vec<Collection>?
            // Checking storage interface. list_collections returns Vec<String> usually or Structs?
            // "the ? operator cannot be applied to type Vec<String>" -> implies it returns Vec<String> directly
            let collections = db.list_collections();
            
            for coll_name in collections {
                // Hide physical shard collections (they end with _sN where N is a number)
                // These are summarized within their logical collection
                if is_physical_shard_collection(&coll_name) {
                    continue;
                }

                // Get Sharding Info from Coordinator
                let shard_table = self.coordinator.get_shard_table(&db_name, &coll_name);
                
                // Get Cluster-wide stats
                let (document_count, chunk_count, storage_bytes) = self.coordinator.get_total_stats(&db_name, &coll_name, None).await.unwrap_or((0, 0, 0));

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
                
                // Store in _system/_cluster_informations
                // We use a generated ID or deterministic ID? 
                // Deterministic ID: "db_coll"
                let doc_id = format!("{}_{}", db_name, coll_name);
                let json = serde_json::to_value(&stats)?;
                
                // We need to write to _system database
                let sys_db = self.storage.get_database("_system")?;
                // Ensure collection exists
                if let Err(_) = sys_db.get_collection("_cluster_informations") {
                    sys_db.create_collection("_cluster_informations".to_string(), None)?;
                }
                let sys_coll = sys_db.get_collection("_cluster_informations")?;
                
                // We need an upsert. 
                // Insert with overwrite? Or Delete then Insert? 
                // StorageEngine usually supports update or we check existence.
                // Assuming `upsert` or `insert` handles it.
                // If we use `insert`, it might fail if exists.
                // Let's rely on `insert` (if it overwrites?) or `replace`?
                // `solidb` storage might not have `replace`.
                // We'll delete and insert for now.
                if sys_coll.get(&doc_id).is_ok() {
                    sys_coll.delete(&doc_id)?;
                }
                
                // Add _key to json
                let mut doc = json.as_object().unwrap().clone();
                doc.insert("_key".to_string(), serde_json::Value::String(doc_id.clone()));
                
                sys_coll.insert(serde_json::Value::Object(doc))?;
            }
        }
        
        Ok(())
    }
}

fn is_physical_shard_collection(name: &str) -> bool {
    if let Some(pos) = name.rfind("_s") {
        let suffix = &name[pos + 2..];
        !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    }
}
