use std::sync::Arc;
use tracing::{info, error};

use super::node::Node;
use super::state::{ClusterState, NodeStatus};
use super::health::HealthMonitor;
use super::transport::{Transport, ClusterMessage};

pub struct ClusterManager {
    local_node: Node,
    state: ClusterState,
    transport: Arc<dyn Transport>,
    health_monitor: Option<HealthMonitor>,
    replication_log: Option<Arc<crate::replication::log::ReplicationLog>>,
    storage: Option<Arc<crate::storage::engine::StorageEngine>>,
}

impl ClusterManager {
    pub fn new(
        local_node: Node,
        state: ClusterState,
        transport: Arc<dyn Transport>,
        replication_log: Option<Arc<crate::replication::log::ReplicationLog>>,
        storage: Option<Arc<crate::storage::engine::StorageEngine>>,
    ) -> Self {
        // Add local node to state
        state.add_member(local_node.clone(), NodeStatus::Active);
        
        Self {
            local_node,
            state,
            transport,
            health_monitor: None,
            replication_log,
            storage,
        }
    }

    pub fn set_replication_log(&mut self, log: Arc<crate::replication::log::ReplicationLog>) {
        self.replication_log = Some(log);
    }

    pub fn local_node_id(&self) -> String {
        self.local_node.id.clone()
    }

    pub fn get_node_address(&self, node_id: &str) -> Option<String> {
        self.state.get_member(node_id).map(|m| m.node.address)
    }

    /// Get the HTTP API address for a node (used for scatter-gather queries)
    pub fn get_node_api_address(&self, node_id: &str) -> Option<String> {
        self.state.get_member(node_id).map(|m| m.node.api_address)
    }

    pub fn state(&self) -> &ClusterState {
        &self.state
    }


    pub fn set_health_monitor(&mut self, monitor: HealthMonitor) {
        self.health_monitor = Some(monitor);
    }

    pub async fn start(&self) {
        info!("Starting ClusterManager for node {}", self.local_node.id);
        
        // Start health monitor if configured
        if let Some(monitor) = &self.health_monitor {
            // In a real impl, we'd spawn this. For now, since monitor.start consumes self, 
            // we need to handle ownership or cloning.
             // tokio::spawn(async move { monitor.start().await });
             // For this stubs we just leave it for now
        }
    }

    pub async fn join_cluster(&self, seed_node: &str) -> anyhow::Result<()> {
        info!("Attempting to join cluster via seed {}", seed_node);
        let msg = ClusterMessage::JoinRequest(self.local_node.clone());
        self.transport.send(seed_node, msg).await?;
        Ok(())
    }

    pub async fn handle_message(&self, msg: ClusterMessage) {
        match msg {
            ClusterMessage::JoinRequest(node) => {
                info!("Node {} wants to join", node.id);
                self.state.add_member(node.clone(), NodeStatus::Active);
                
                // Respond with current peers
                let peers = self.state.active_nodes();
                let response = ClusterMessage::JoinResponse { 
                    success: true, 
                    peers 
                };
                
                // We would need to send back response here. 
                // Currently Transport specific send requires address.
                let _ = self.transport.send(&node.address, response).await;
            }
            ClusterMessage::JoinResponse { success, peers } => {
                if success {
                    info!("Successfully joined cluster. Received {} peers.", peers.len());
                        for peer in peers {
                        if peer.id != self.local_node.id {
                            let is_new = self.state.add_member(peer.clone(), NodeStatus::Active);
                            if is_new {
                                info!("Discovered new peer via JoinResponse: {}. Initiating handshake.", peer.id);
                                // Send JoinRequest to this new peer to ensure they know us
                                let transport = self.transport.clone();
                                let local_node = self.local_node.clone();
                                let peer_addr = peer.address.clone();
                                tokio::spawn(async move {
                                    let join_msg = ClusterMessage::JoinRequest(local_node);
                                    if let Err(e) = transport.send(&peer_addr, join_msg).await {
                                        error!("Failed to handshake with new peer {}: {}", peer_addr, e);
                                    }
                                });
                            }
                        }
                    }
                }
            }
            ClusterMessage::Heartbeat { from, sequence } => {
                // trace!("Received heartbeat from {} seq {}", from, sequence);
                // self.state.update_heartbeat(&from, sequence);
                // Using info! for debugging now, catch them all
                tracing::trace!("Received heartbeat from {} seq {}", from, sequence);
                self.state.update_heartbeat(&from, sequence);
            }
            ClusterMessage::Leave { from } => {
                info!("Node {} is leaving", from);
                self.state.remove_member(&from);
            }
            ClusterMessage::Replication(repl_msg) => {
                 self.handle_replication_message(repl_msg).await;
            }
        }
    }
    
    async fn handle_replication_message(&self, msg: crate::replication::protocol::ReplicationMessage) {
        use crate::replication::protocol::{ReplicationMessage, Operation};
        use std::collections::HashMap;
        
        match msg {
            ReplicationMessage::SyncResponse { entries, .. } => {
                let total_entries = entries.len();
                if total_entries == 0 {
                    return;
                }
                
                tracing::debug!("Received {} replication entries", total_entries);
                
                // Filter entries first (loop detection + cycle detection)
                let mut valid_entries = Vec::with_capacity(total_entries);
                for entry in entries {
                    // Loop Detection: If we originated this entry, ignore it.
                    if entry.node_id == self.local_node.id {
                        continue;
                    }
                    
                    // Cycle Detection via Vector Clock (Max Origin Sequence)
                    let origin_seq = entry.origin_sequence.unwrap_or(0);
                    if origin_seq > 0 {
                        if !self.state.check_and_update_origin_sequence(entry.node_id.clone(), origin_seq) {
                            continue;
                        }
                    }
                    valid_entries.push(entry);
                }
                
                if valid_entries.is_empty() {
                    return;
                }
                
                // Append to replication log in batch
                if let Some(log) = &self.replication_log {
                    if let Err(e) = log.append_batch(valid_entries.clone()) {
                        error!("Failed to append replication batch: {}", e);
                        return;
                    }
                }
                
                // Group entries by (database, collection) for batched application
                let mut insert_update_groups: HashMap<(String, String), Vec<(String, serde_json::Value)>> = HashMap::new();
                let mut other_entries = Vec::new();
                
                for entry in &valid_entries {
                    match entry.operation {
                        Operation::Insert | Operation::Update => {
                            if let Some(data) = &entry.data {
                                if let Ok(doc_value) = serde_json::from_slice::<serde_json::Value>(data) {
                                    let key = (entry.database.clone(), entry.collection.clone());
                                    insert_update_groups
                                        .entry(key)
                                        .or_insert_with(Vec::new)
                                        .push((entry.key.clone(), doc_value));
                                }
                            }
                        }
                        _ => {
                            other_entries.push(entry.clone());
                        }
                    }
                }
                
                // Apply batched inserts/updates in blocking task
                if let Some(storage) = &self.storage {
                    let storage = storage.clone();
                    let insert_update_groups = insert_update_groups;
                    
                    // Compute node topology BEFORE spawning blocking task
                    let mut all_nodes: Vec<String> = self.state.get_all_members()
                        .iter()
                        .map(|m| m.node.address.clone())
                        .collect();
                    all_nodes.sort();
                    let my_addr = self.local_node.address.clone();
                    let my_index = all_nodes.iter().position(|n| n == &my_addr);
                    let num_nodes = all_nodes.len();
                    
                    // Spawn blocking task for RocksDB writes
                    let result = tokio::task::spawn_blocking(move || {
                        for ((db_name, coll_name), docs) in insert_update_groups {
                            if let Ok(db) = storage.get_database(&db_name) {
                                if let Ok(collection) = db.get_collection(&coll_name) {
                                    // SHARD-AWARE FILTERING
                                    let docs_to_apply = if let Some(shard_config) = collection.get_shard_config() {
                                        if shard_config.num_shards > 0 && num_nodes > 0 {
                                            if let Some(my_idx) = my_index {
                                                let original_count = docs.len();
                                                let filtered: Vec<(String, serde_json::Value)> = docs
                                                    .into_iter()
                                                    .filter(|(doc_key, _)| {
                                                        let shard_id = crate::sharding::router::ShardRouter::route(
                                                            doc_key,
                                                            shard_config.num_shards,
                                                        );
                                                        crate::sharding::router::ShardRouter::is_shard_replica(
                                                            shard_id,
                                                            my_idx,
                                                            shard_config.replication_factor,
                                                            num_nodes,
                                                        )
                                                    })
                                                    .collect();
                                                
                                                let skipped = original_count - filtered.len();
                                                if skipped > 0 {
                                                    tracing::info!(
                                                        "[SHARD-FILTER] Filtered {}/{} docs for {}/{} (RF={}, my_idx={}, num_nodes={})",
                                                        skipped, original_count, db_name, coll_name,
                                                        shard_config.replication_factor, my_idx, num_nodes
                                                    );
                                                }
                                                filtered
                                            } else {
                                                tracing::warn!("[SHARD-FILTER] Could not determine node index, applying all docs");
                                                docs
                                            }
                                        } else {
                                            docs // No shards configured
                                        }
                                    } else {
                                        docs // Not sharded
                                    };
                                    
                                    if docs_to_apply.is_empty() {
                                        continue;
                                    }
                                    
                                    let doc_count = docs_to_apply.len();
                                    match collection.upsert_batch(docs_to_apply) {
                                        Ok(_) => {
                                            tracing::debug!("Batch upserted {} docs to {}/{}", doc_count, db_name, coll_name);
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to batch upsert to {}/{}: {}", db_name, coll_name, e);
                                        }
                                    }
                                }
                            }
                        }
                    }).await;
                    
                    if let Err(e) = result {
                        error!("Blocking task panicked: {}", e);
                    }
                    
                    // Apply non-insert/update operations sequentially (less common)
                    for entry in other_entries {
                        if let Err(e) = self.apply_replication_entry(&entry) {
                            error!("Failed to apply replication entry: {}", e);
                        }
                    }
                }
                
                info!("Applied {} replication entries", valid_entries.len());
            }
            _ => {}
        }
    }

    fn apply_replication_entry(&self, entry: &crate::replication::protocol::LogEntry) -> anyhow::Result<()> {
        use crate::replication::protocol::Operation;
        if let Some(storage) = &self.storage {
             match entry.operation {
                Operation::CreateCollection => {
                    let db = storage.get_database(&entry.database)?;
                    
                    // Extract collection type from metadata
                    let collection_type = if let Some(data) = &entry.data {
                        let metadata: serde_json::Value = serde_json::from_slice(data).unwrap_or_default();
                        metadata.get("type").and_then(|v| v.as_str()).map(|s| s.to_string())
                    } else {
                        None
                    };
                    
                    // Create the collection
                    if let Err(e) = db.create_collection(entry.collection.clone(), collection_type.clone()) {
                        // Ignore if already exists (idempotency)
                        if !e.to_string().contains("already exists") {
                            return Err(anyhow::anyhow!("Create collection failed: {}", e));
                        }
                    }
                    
                    // Apply shard config if present
                    if let Some(data) = &entry.data {
                        let metadata: serde_json::Value = serde_json::from_slice(data).unwrap_or_default();
                        
                        // Set collection type
                        if let Some(ctype) = collection_type {
                            if let Ok(coll) = db.get_collection(&entry.collection) {
                                let _ = coll.set_type(&ctype);
                            }
                        }
                        
                        // Set shard config
                        if let Some(shard_config_val) = metadata.get("shardConfig") {
                            if !shard_config_val.is_null() {
                                if let Ok(coll) = db.get_collection(&entry.collection) {
                                    let config = crate::sharding::coordinator::CollectionShardConfig {
                                        num_shards: shard_config_val.get("num_shards").and_then(|v| v.as_u64()).unwrap_or(1) as u16,
                                        shard_key: shard_config_val.get("shard_key").and_then(|v| v.as_str()).unwrap_or("_key").to_string(),
                                        replication_factor: shard_config_val.get("replication_factor").and_then(|v| v.as_u64()).unwrap_or(1) as u16,
                                    };
                                    let _ = coll.set_shard_config(&config);
                                }
                            }
                        }
                    }
                }
                Operation::Insert | Operation::Update => {
                    let db = storage.get_database(&entry.database)?;
                    let collection = db.get_collection(&entry.collection)?;
                    if let Some(data) = &entry.data {
                        let doc_value: serde_json::Value = serde_json::from_slice(data)?;
                        // We use insert_internal or similar that DOES NOT generate new log entry?
                        // Or we use insert but suppress replication?
                        // If we use standard insert, it might trigger handlers->record_write->log->loop!
                        // Be careful! 
                        // StorageEngine::insert writes to backend. It doesn't know about replication.
                        // Handlers layer adds replication.
                        // So calling collection.insert() is SAFE from recursion, 
                        // BUT collection.insert might trigger triggers/indexes.
                        collection.insert(doc_value)?;
                    }
                }
                Operation::Delete => {
                     let db = storage.get_database(&entry.database)?;
                     let collection = db.get_collection(&entry.collection)?;
                     // Ignore error if doc doesn't exist (idempotency)
                     let _ = collection.delete(&entry.key);
                }
                Operation::DeleteCollection => {
                    let db = storage.get_database(&entry.database)?;
                    // Ignore error if collection doesn't exist (idempotency)
                    let _ = db.delete_collection(&entry.collection);
                }
                Operation::TruncateCollection => {
                    let db = storage.get_database(&entry.database)?;
                    if let Ok(collection) = db.get_collection(&entry.collection) {
                        let _ = collection.truncate();
                    }
                }
                Operation::CreateDatabase => {
                    // Ignore error if database already exists (idempotency)
                    let _ = storage.create_database(entry.database.clone());
                }
                Operation::DeleteDatabase => {
                    // Ignore error if database doesn't exist (idempotency)
                    let _ = storage.delete_database(&entry.database);
                }
                _ => {
                    // PutBlobChunk, DeleteBlob - implement later if needed
                    tracing::debug!("Unhandled replication operation: {:?}", entry.operation);
                }
             }
        }
        Ok(())
    }
}
