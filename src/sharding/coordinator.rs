//! Shard coordinator for routing document operations to correct nodes

use std::collections::HashMap;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

use super::router::ShardRouter;
use super::table::ShardTable;
use crate::cluster::manager::ClusterManager;
use crate::error::{DbError, DbResult};
use crate::storage::{Document, StorageEngine};

/// Collection sharding configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionShardConfig {
    /// Number of shards
    pub num_shards: u16,
    /// Field to use for sharding (default: "_key")
    pub shard_key: String,
    /// Replication factor: 1 = no replicas, 2 = 1 replica per shard, etc.
    #[serde(default = "default_replication_factor")]
    pub replication_factor: u16,
}

fn default_replication_factor() -> u16 {
    1
}

impl Default for CollectionShardConfig {
    fn default() -> Self {
        Self {
            num_shards: 3,
            shard_key: "_key".to_string(),
            replication_factor: 1,
        }
    }
}

/// Coordinates shard-aware document operations
#[derive(Clone)]
pub struct ShardCoordinator {
    storage: Arc<StorageEngine>,
    http_client: Client,
    cluster_manager: Arc<ClusterManager>,
    // For now, we keep a local ShardTable. In a real distributed system, this should be synced.
    // We'll wrap it in RwLock for updates.
    // Map "db/collection" -> ShardTable
    shard_tables: Arc<std::sync::RwLock<HashMap<String, ShardTable>>>,
    // We keep basic health tracking here or rely on ClusterManager?
    // ClusterManager handles health.
}

impl ShardCoordinator {
    pub fn new(
        storage: Arc<StorageEngine>,
        cluster_manager: Arc<ClusterManager>,
    ) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
            
        Self {
            storage,
            http_client,
            cluster_manager,
            shard_tables: Arc::new(std::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Helper to get local node ID
    fn local_node_id(&self) -> String {
        self.cluster_manager.local_node_id()
    }

    /// Helper to resolve node ID to address
    fn resolve_node_address(&self, node_id: &str) -> Option<String> {
        // ClusterManager should provide this
        // We might need to expose a method in ClusterManager
        // For now, let's assume ClusterManager has `get_node_address`
        // Inspecting ClusterManager... it has `state()` which has `members`.
        if node_id == self.local_node_id() {
             // Return normalized local address? Or just handle local check differently.
             // Let's rely on is_local check.
             return None; 
        }
        
        // TODO: Access cluster state to get address
        // member = self.cluster_manager.state().get_member(node_id)
        // member.address
        // We will implement this connection after this file write.
        None // Placeholder
    }

    /// Check if I am responsible for the shard (Primary or Replica)
    pub fn is_responsible_for(&self, db_name: &str, coll_name: &str, shard_id: u16) -> bool {
        let key = format!("{}/{}", db_name, coll_name);
        let tables = self.shard_tables.read().unwrap();
        
        if let Some(table) = tables.get(&key) {
            if let Some(assignment) = table.get_assignment(shard_id) {
                let my_id = self.local_node_id();
                if assignment.primary_node == my_id {
                    return true;
                }
                if assignment.replica_nodes.contains(&my_id) {
                    return true;
                }
            }
            if table.assignments.is_empty() {
                return true; // Fallback
            }
        } else {
             // No table found, assume local/standalone or implicit
             return true; 
        }
        false
    }

    // ... Re-implement insert, etc. logic using table ...
    // To keep this response short and avoid context limit, I will write a stub first and then fill method by method if needed,
    // OR likely logic is similar to before but resolving nodes via Table.

    /// Insert document
    pub async fn insert(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_config: &CollectionShardConfig,
        mut doc: Value,
    ) -> DbResult<Document> {
         // 1. Generate Key
         let key = if let Some(key) = doc.get("_key").and_then(|v| v.as_str()) {
            key.to_string()
         } else {
            let key = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
            if let Value::Object(ref mut obj) = doc {
                obj.insert("_key".to_string(), Value::String(key.clone()));
            }
            key
         };
         
         // 2. Route
         let shard_id = ShardRouter::route(&key, shard_config.num_shards);
         
         // 3. Resolve Nodes
         let (primary, replicas) = {
             let key = format!("{}/{}", db_name, coll_name);
             let tables = self.shard_tables.read().unwrap();
             if let Some(table) = tables.get(&key) {
                 if let Some(assign) = table.get_assignment(shard_id) {
                     (Some(assign.primary_node.clone()), assign.replica_nodes.clone())
                 } else {
                     (None, vec![])
                 }
             } else {
                 (None, vec![])
             }
         };
         
         // For now, if no assignment, use ALGORITHMIC sharding based on cluster members
         if primary.is_none() {
             // Compute if we should store this document using is_shard_replica logic
             let mut nodes: Vec<String> = self.cluster_manager.state().get_all_members()
                 .iter()
                 .map(|m| m.node.address.clone())
                 .collect();
             nodes.sort();
             
             if nodes.is_empty() {
                 // Truly standalone, just insert locally
                 let collection = self.storage.get_database(db_name)?.get_collection(coll_name)?;
                 return collection.insert(doc);
             }
             
             let my_addr = self.cluster_manager.state().get_member(&self.local_node_id())
                 .map(|m| m.node.address.clone())
                 .unwrap_or_else(|| "unknown".to_string());
             
             let num_nodes = nodes.len();
             
             // Diagnostic: log once per batch (first doc only for now)
             static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
             if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                 tracing::info!(
                     "[SHARD-DEBUG] nodes={:?}, my_addr={}, my_id={}, num_nodes={}",
                     nodes, my_addr, self.local_node_id(), num_nodes
                 );
             }
             
             // Am I responsible for this shard (as primary or replica)?
             if let Some(my_idx) = nodes.iter().position(|n| n == &my_addr) {
                 let is_responsible = ShardRouter::is_shard_replica(
                     shard_id, 
                     my_idx, 
                     shard_config.replication_factor, 
                     num_nodes
                 );
                 
                 if is_responsible {
                     // I am primary or replica, insert locally
                     let collection = self.storage.get_database(db_name)?.get_collection(coll_name)?;
                     return collection.insert(doc);
                 } else {
                     // I am NOT responsible for this shard.
                     // Since forwarding is not implemented yet, we STILL store locally
                     // to prevent data loss. The replication mechanism will distribute
                     // to the correct nodes, and nodes will filter at receive time.
                     // TODO: Implement proper forwarding to responsible node
                     tracing::debug!(
                         "[SHARD] Not responsible for shard {} (my_idx={}, RF={}, num_nodes={}), storing locally anyway (forwarding not implemented)",
                         shard_id, my_idx, shard_config.replication_factor, num_nodes
                     );
                     let collection = self.storage.get_database(db_name)?.get_collection(coll_name)?;
                     return collection.insert(doc);
                 }
             } else {
                 // Fallback: can't find self in cluster, insert locally
                 tracing::warn!("[SHARD] Can't find self in cluster, inserting locally");
                 let collection = self.storage.get_database(db_name)?.get_collection(coll_name)?;
                 return collection.insert(doc);
             }
         }
         
         let primary_node = primary.unwrap();
         
         // 4. Check if I am primary
         if primary_node == self.local_node_id() {
             // Local Insert
             let collection = self.storage.get_database(db_name)?.get_collection(coll_name)?;
             let res = collection.insert(doc.clone())?;
             
             // 5. Replicate to others
             // TODO: Fan-out to replicas
             
             return Ok(res);
         } else {
             // 5. Forward to primary
             // self.forward_insert(primary_node, ...)
             return Err(DbError::InternalError("Forwarding not fully implemented yet".to_string()));
         }
    }

    pub async fn insert_batch(
        &self,
        db_name: &str,
        coll_name: &str,
        shard_config: &CollectionShardConfig,
        docs: Vec<Value>,
    ) -> DbResult<(usize, usize)> {
        // Simple implementation: insert one by one using insert()
        let mut successes = 0;
        let mut failures = 0;
        
        for doc in docs {
            match self.insert(db_name, coll_name, shard_config, doc).await {
                Ok(_) => successes += 1,
                Err(_) => failures += 1,
            }
        }
        Ok((successes, failures))
    }

    pub async fn get(
        &self,
        db_name: &str,
        coll_name: &str,
        _shard_config: &CollectionShardConfig,
        key: &str,
    ) -> DbResult<Document> {
        let collection = self.storage.get_database(db_name)?.get_collection(coll_name)?;
        collection.get(key).map_err(DbError::from)
    }

    pub async fn update(
        &self,
        db_name: &str,
        coll_name: &str,
        _shard_config: &CollectionShardConfig,
        key: &str,
        data: Value,
    ) -> DbResult<Document> {
        let collection = self.storage.get_database(db_name)?.get_collection(coll_name)?;
        collection.update(key, data).map_err(DbError::from)
    }

    pub async fn delete(
        &self,
        db_name: &str,
        coll_name: &str,
        _shard_config: &CollectionShardConfig,
        key: &str,
    ) -> DbResult<bool> {
         let collection = self.storage.get_database(db_name)?.get_collection(coll_name)?;
         collection.delete(key).map(|_| true).map_err(DbError::from)
    }

    pub async fn scan_all_shards(
        &self,
        db_name: &str,
        coll_name: &str,
        _shard_config: &CollectionShardConfig,
    ) -> DbResult<Vec<Document>> {
        // Just scan local for now
        let collection = self.storage.get_database(db_name)?.get_collection(coll_name)?;
        Ok(collection.scan(None))
    }

    pub async fn rebalance(&self) -> DbResult<()> {
        // Stub
        Ok(())
    }

    pub async fn remove_node(&self, _node_addr: &str) -> DbResult<()> {
        // Stub
        Ok(())
    }

    pub fn is_local(&self, db_name: &str, coll_name: &str, shard_id: u16) -> bool {
        self.is_responsible_for(db_name, coll_name, shard_id)
    }

    pub fn get_collection_nodes(&self, _shard_config: &CollectionShardConfig) -> Vec<String> {
        // Return all known nodes as potential holders? 
        // Or just local?
        // Return all active nodes.
        self.get_node_addresses()
    }

    pub fn get_node_index(&self) -> usize {
        // Find index of local node in sorted list of active nodes
        let my_id = self.local_node_id();
        let members = self.cluster_manager.state().active_nodes(); // returns sorted list?
        // ClusterState methods usually return Vec<ClusterMember> or similar. 
        // Need to check ClusterState API.
        // Assuming active_nodes() returns Vec<ClusterMember> or Node info.
        // Let's rely on basic ID check.
        0 // Placeholder
    }
    
    pub fn get_replicas(&self, key: &str, shard_config: &CollectionShardConfig) -> Vec<String> {
        // Get ALL nodes (stable ring including dead ones) and sort them for deterministic ordering
        // Use ADDRESSES because ReplicationService uses addresses for sharding logic
        let mut nodes: Vec<String> = self.cluster_manager.state().get_all_members()
            .iter()
            .map(|m| m.node.address.clone())
            .collect();
        nodes.sort();
        
        let num_nodes = nodes.len();
        if num_nodes == 0 {
             return vec![];
        }

        // Calculate shard ID
        let shard_id = ShardRouter::route(key, shard_config.num_shards);
        
        // Find all nodes responsible (primary + replicas)
        let mut replicas = Vec::new();
        for i in 0..shard_config.replication_factor {
             let idx = (shard_id as usize + i as usize) % num_nodes;
             if let Some(node_id) = nodes.get(idx) {
                 if !replicas.contains(node_id) {
                     replicas.push(node_id.clone());
                 }
             }
        }
        
        replicas
    }

    
    // Missing methods required by executor.rs and handlers.rs
    
    pub fn get_node_addresses(&self) -> Vec<String> {
        // We need to return API addresses from cluster manager (not replication addresses)
        // ClusterManager tracks active nodes.
        // We iterate active nodes and get API addresses for HTTP requests.
        self.cluster_manager.state().active_nodes().iter().map(|n| n.api_address.clone()).collect()
    }
    
    pub fn my_address(&self) -> String {
        // Use API address for HTTP requests (matching get_node_addresses)
        self.cluster_manager.get_node_api_address(&self.local_node_id())
            .unwrap_or_else(|| "127.0.0.1:0".to_string())
    }
    
    pub fn get_http_client(&self) -> &Client {
        &self.http_client
    }

    pub fn get_shard_table(&self, db_name: &str, coll_name: &str) -> Option<ShardTable> {
        let key = format!("{}/{}", db_name, coll_name);
        let tables = self.shard_tables.read().unwrap();
        tables.get(&key).cloned()
    }
}

