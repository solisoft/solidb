use std::sync::Arc;
use tracing::{error, info};

use super::health::HealthMonitor;
use super::node::Node;
use super::state::{ClusterState, NodeStatus};
use super::stats::NodeBasicStats;
use super::transport::{ClusterMessage, Transport};

pub struct ClusterManager {
    local_node: Node,
    state: ClusterState,
    transport: Arc<dyn Transport>,
    health_monitor: Option<HealthMonitor>,
    replication_log: Option<Arc<crate::sync::log::SyncLog>>,
    storage: Option<Arc<crate::storage::engine::StorageEngine>>,
}

impl ClusterManager {
    pub fn new(
        local_node: Node,
        state: ClusterState,
        transport: Arc<dyn Transport>,
        replication_log: Option<Arc<crate::sync::log::SyncLog>>,
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

    pub fn set_replication_log(&mut self, log: Arc<crate::sync::log::SyncLog>) {
        self.replication_log = Some(log);
    }

    pub fn local_node_id(&self) -> String {
        self.local_node.id.clone()
    }

    /// Get the local node's replication address (for when node isn't in member list yet)
    pub fn get_local_address(&self) -> String {
        self.local_node.address.clone()
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

    /// Check if a node is considered healthy based on heartbeat timeout
    /// Default timeout: 30 seconds
    pub fn is_node_healthy(&self, node_id: &str) -> bool {
        // Local node is always healthy
        if node_id == self.local_node.id {
            return true;
        }

        if let Some(member) = self.state.get_member(node_id) {
            // Check status
            if member.status == NodeStatus::Dead
                || member.status == NodeStatus::Leaving
                || member.status == NodeStatus::Syncing
                || member.status == NodeStatus::Joining
                || member.status == NodeStatus::Suspected
            {
                return false;
            }

            // Check heartbeat timeout (30 seconds)
            let now = chrono::Utc::now().timestamp_millis() as u64;
            let timeout_ms = 30_000; // 30 seconds
            if now - member.last_heartbeat > timeout_ms {
                return false;
            }

            true
        } else {
            // Unknown node is considered unhealthy
            false
        }
    }

    /// Get all healthy node IDs
    pub fn get_healthy_nodes(&self) -> Vec<String> {
        let local_id = self.local_node.id.clone();

        self.state
            .get_all_members()
            .into_iter()
            .filter(|m| {
                // Local node is always considered healthy
                if m.node.id == local_id {
                    return true;
                }

                if m.status == NodeStatus::Dead
                    || m.status == NodeStatus::Leaving
                    || m.status == NodeStatus::Syncing
                    || m.status == NodeStatus::Joining
                    || m.status == NodeStatus::Suspected
                {
                    return false;
                }
                let now = chrono::Utc::now().timestamp_millis() as u64;
                let timeout_ms = 30_000;
                now - m.last_heartbeat <= timeout_ms
            })
            .map(|m| m.node.id)
            .collect()
    }

    pub fn set_health_monitor(&mut self, monitor: HealthMonitor) {
        self.health_monitor = Some(monitor);
    }

    pub async fn start(&self) {
        info!("Starting ClusterManager for node {}", self.local_node.id);

        // Start health monitor if configured
        if let Some(_monitor) = &self.health_monitor {
            // In a real impl, we'd spawn this. For now, since monitor.start consumes self,
            // we need to handle ownership or cloning.
            // tokio::spawn(async move { monitor.start().await });
            // For this stubs we just leave it for now
        }

        // Heartbeat Loop
        let transport = self.transport.clone();
        let state = self.state.clone();
        let local_node_id = self.local_node.id.clone();
        let storage_opt = self.storage.clone();
        let replication_log = self.replication_log.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;

                // 1. Collect Local Stats
                let stats = if let Some(storage) = &storage_opt {
                    let databases = storage.list_databases();
                    let mut total_chunk_count = 0;
                    let mut total_file_count = 0;
                    let mut storage_bytes = 0;
                    let mut total_memtable_size = 0;
                    let mut total_live_size = 0;

                    for db_name in databases {
                        if let Ok(db) = storage.get_database(&db_name) {
                            for coll_name in db.list_collections() {
                                if let Ok(coll) = db.get_collection(&coll_name) {
                                    let s = coll.stats();
                                    total_chunk_count += s.chunk_count as u64;
                                    total_file_count += s.disk_usage.num_sst_files;
                                    storage_bytes += s.disk_usage.sst_files_size;
                                    total_memtable_size += s.disk_usage.memtable_size;
                                    total_live_size += s.disk_usage.live_data_size;
                                }
                            }
                        }
                    }

                    Some(NodeBasicStats {
                        total_chunk_count,
                        total_file_count,
                        storage_bytes,
                        total_memtable_size,
                        total_live_size,
                        cpu_usage_percent: 0.0, // TODO: Add sysinfo if needed
                        memory_used_mb: 0,      // TODO: Add sysinfo if needed
                    })
                } else {
                    None
                };

                // 2. Get peers to send to
                let peers = state.active_nodes();
                let current_seq = if let Some(log) = &replication_log {
                    log.current_sequence()
                } else {
                    0
                };

                // 3. Send Heartbeat
                for peer in peers {
                    if peer.id == local_node_id {
                        continue;
                    }

                    let msg = ClusterMessage::Heartbeat {
                        from: local_node_id.clone(),
                        sequence: current_seq,
                        stats: stats.clone(),
                    };

                    // We don't await strictly or handle error to prevent blocking loop
                    let _ = transport.send(&peer.address, msg).await;
                }
            }
        });
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
                    peers,
                };

                // We would need to send back response here.
                // Currently Transport specific send requires address.
                let _ = self.transport.send(&node.address, response).await;
            }
            ClusterMessage::JoinResponse { success, peers } => {
                if success {
                    info!(
                        "Successfully joined cluster. Received {} peers.",
                        peers.len()
                    );
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
                                        error!(
                                            "Failed to handshake with new peer {}: {}",
                                            peer_addr, e
                                        );
                                    }
                                });
                            }
                        }
                    }
                }
            }
            ClusterMessage::Heartbeat {
                from,
                sequence,
                stats,
            } => {
                // trace!("Received heartbeat from {} seq {}", from, sequence);
                // self.state.update_heartbeat(&from, sequence);
                // Using trace! for heartbeats
                tracing::trace!(
                    "Received heartbeat from {} seq {} stats {:?}",
                    from,
                    sequence,
                    stats
                );
                self.state.update_heartbeat(&from, sequence, stats);
            }
            ClusterMessage::Leave { from } => {
                info!("Node {} is leaving", from);
                self.state.remove_member(&from);
            }
            ClusterMessage::Replication(sync_msg) => {
                self.handle_sync_message(sync_msg).await;
            }
        }
    }

    async fn handle_sync_message(&self, msg: crate::sync::SyncMessage) {
        use crate::sync::{Operation, SyncMessage};
        use std::collections::HashMap;

        if let SyncMessage::SyncBatch { entries, .. } = msg {
            let total_entries = entries.len();
            if total_entries == 0 {
                return;
            }

            tracing::debug!("Received {} sync entries", total_entries);

            // Filter entries (loop detection)
            let mut valid_entries = Vec::with_capacity(total_entries);
            for entry in entries {
                // Loop Detection: If we originated this entry, ignore it.
                if entry.origin_node == self.local_node.id {
                    continue;
                }

                // Cycle Detection via sequence
                if !self.state.check_and_update_origin_sequence(
                    entry.origin_node.clone(),
                    entry.origin_sequence,
                ) {
                    continue;
                }
                valid_entries.push(entry);
            }

            if valid_entries.is_empty() {
                return;
            }

            // Group entries by (database, collection) for batched application
            let mut insert_update_groups: HashMap<
                (String, String),
                Vec<(String, serde_json::Value)>,
            > = HashMap::new();
            let mut other_entries = Vec::new();

            for entry in &valid_entries {
                match entry.operation {
                    Operation::Insert | Operation::Update => {
                        if let Some(data) = &entry.document_data {
                            if let Ok(doc_value) = serde_json::from_slice::<serde_json::Value>(data)
                            {
                                let key = (entry.database.clone(), entry.collection.clone());
                                insert_update_groups
                                    .entry(key)
                                    .or_default()
                                    .push((entry.document_key.clone(), doc_value));
                            }
                        }
                    }
                    _ => {
                        other_entries.push(entry.clone());
                    }
                }
            }

            // Apply batched inserts/updates
            if let Some(storage) = &self.storage {
                let storage = storage.clone();
                let insert_update_groups = insert_update_groups;

                // Compute node topology
                let mut all_nodes: Vec<String> = self
                    .state
                    .get_all_members()
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
                                            filtered
                                        } else {
                                            docs
                                        }
                                    } else {
                                        docs
                                    }
                                } else {
                                    docs
                                };

                                if docs_to_apply.is_empty() {
                                    continue;
                                }

                                let doc_count = docs_to_apply.len();
                                let _ = collection.upsert_batch(docs_to_apply);
                                tracing::debug!("Batch upserted {} docs to {}/{}", doc_count, db_name, coll_name);
                            }
                        }
                    }
                }).await;

                if let Err(e) = result {
                    error!("Blocking task panicked: {}", e);
                }

                // Apply non-insert/update operations
                for entry in other_entries {
                    if let Err(e) = self.apply_sync_entry(&entry) {
                        error!("Failed to apply sync entry: {}", e);
                    }
                }
            }

            info!("Applied {} sync entries", valid_entries.len());
        }
    }

    fn apply_sync_entry(&self, entry: &crate::sync::SyncEntry) -> anyhow::Result<()> {
        use crate::sync::Operation;
        if let Some(storage) = &self.storage {
            match entry.operation {
                Operation::CreateCollection => {
                    let db = storage.get_database(&entry.database)?;

                    // Extract collection type from metadata
                    let collection_type = if let Some(data) = &entry.document_data {
                        let metadata: serde_json::Value =
                            serde_json::from_slice(data).unwrap_or_default();
                        metadata
                            .get("type")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string())
                    } else {
                        None
                    };

                    // Create the collection
                    if let Err(e) =
                        db.create_collection(entry.collection.clone(), collection_type.clone())
                    {
                        // Ignore if already exists (idempotency)
                        if !e.to_string().contains("already exists") {
                            return Err(anyhow::anyhow!("Create collection failed: {}", e));
                        }
                    }

                    // Apply shard config if present
                    if let Some(data) = &entry.document_data {
                        let metadata: serde_json::Value =
                            serde_json::from_slice(data).unwrap_or_default();

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
                                    let config =
                                        crate::sharding::coordinator::CollectionShardConfig {
                                            num_shards: shard_config_val
                                                .get("num_shards")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(1)
                                                as u16,
                                            shard_key: shard_config_val
                                                .get("shard_key")
                                                .and_then(|v| v.as_str())
                                                .unwrap_or("_key")
                                                .to_string(),
                                            replication_factor: shard_config_val
                                                .get("replication_factor")
                                                .and_then(|v| v.as_u64())
                                                .unwrap_or(1)
                                                as u16,
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
                    if let Some(data) = &entry.document_data {
                        let doc_value: serde_json::Value = serde_json::from_slice(data)?;
                        collection.insert(doc_value)?;
                    }
                }
                Operation::Delete => {
                    let db = storage.get_database(&entry.database)?;
                    let collection = db.get_collection(&entry.collection)?;
                    // Ignore error if doc doesn't exist (idempotency)
                    let _ = collection.delete(&entry.document_key);
                }
                Operation::DeleteCollection => {
                    let db = storage.get_database(&entry.database)?;
                    // Ignore error if collection doesn't exist (idempotency)
                    let _ = db.delete_collection(&entry.collection);
                }
                Operation::TruncateCollection => {
                    let db = storage.get_database(&entry.database)?;
                    if let Ok(collection) = db.get_collection(&entry.collection) {
                        // Check if sharded and truncate physical shards
                        if let Some(shard_config) = collection.get_shard_config() {
                            if shard_config.num_shards > 0 {
                                tracing::info!(
                                    "TRUNCATE: Truncating {} shards for {}.{}",
                                    shard_config.num_shards,
                                    entry.database,
                                    entry.collection
                                );
                                for shard_id in 0..shard_config.num_shards {
                                    let physical_name =
                                        format!("{}_s{}", entry.collection, shard_id);
                                    if let Ok(shard_coll) = db.get_collection(&physical_name) {
                                        let _ = shard_coll.truncate();
                                    }
                                }
                            }
                        }

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
                    tracing::debug!("Unhandled sync operation: {:?}", entry.operation);
                }
            }
        }
        Ok(())
    }
}
