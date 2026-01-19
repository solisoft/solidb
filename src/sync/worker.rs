//! Sync worker for background replication
//!
//! Handles:
//! - Incremental sync with peers
//! - Full sync for new nodes
//! - Heartbeat sending/receiving
//! - Dead node detection and removal
//! - Shard rebalancing

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use super::protocol::{NodeStats, Operation, SyncEntry, SyncMessage};
use super::state::SyncState;
use super::transport::{ConnectionPool, SyncServer, TransportError};
use crate::storage::StorageEngine;

/// Configuration for the sync worker
#[derive(Clone)]
pub struct SyncConfig {
    /// Heartbeat interval
    pub heartbeat_interval: Duration,
    /// Timeout before considering a node dead
    pub dead_node_timeout: Duration,
    /// Maximum batch size in bytes
    pub max_batch_bytes: u32,
    /// Sync interval (how often to check for updates)
    pub sync_interval: Duration,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            heartbeat_interval: Duration::from_secs(5),
            dead_node_timeout: Duration::from_secs(15),
            max_batch_bytes: 1024 * 1024, // 1 MB
            sync_interval: Duration::from_millis(1000),
        }
    }
}

/// Command to send to the sync worker
pub enum SyncCommand {
    /// Request full sync from a peer
    RequestFullSync { peer_addr: String },
    /// Add a new peer
    AddPeer {
        node_id: String,
        sync_addr: String,
        http_addr: String,
    },
    /// Remove a peer
    RemovePeer { node_id: String },
    /// Shutdown the worker
    Shutdown,
}

/// Sync worker running in background
pub struct SyncWorker {
    storage: Arc<StorageEngine>,
    state: Arc<SyncState>,
    pool: Arc<ConnectionPool>,
    sync_log: Arc<super::log::SyncLog>,
    config: SyncConfig,
    command_rx: mpsc::Receiver<SyncCommand>,
    local_node_id: String,
    keyfile_path: String,
    listen_addr: String,
    incoming_rx: Option<mpsc::Receiver<(super::transport::SyncStream, String)>>,
    cluster_manager: Option<Arc<crate::cluster::manager::ClusterManager>>,
    shard_coordinator: Option<Arc<crate::sharding::ShardCoordinator>>,
    system: sysinfo::System,
}

impl SyncWorker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        storage: Arc<StorageEngine>,
        state: Arc<SyncState>,
        pool: Arc<ConnectionPool>,
        sync_log: Arc<super::log::SyncLog>,
        config: SyncConfig,
        command_rx: mpsc::Receiver<SyncCommand>,
        local_node_id: String,
        keyfile_path: String,
        listen_addr: String,
    ) -> Self {
        Self {
            storage,
            state,
            pool,
            sync_log,
            config,
            command_rx,
            local_node_id,
            keyfile_path,
            listen_addr,
            incoming_rx: None,
            cluster_manager: None,
            shard_coordinator: None,
            system: sysinfo::System::new(),
        }
    }

    pub fn with_cluster_manager(
        mut self,
        manager: Arc<crate::cluster::manager::ClusterManager>,
    ) -> Self {
        self.cluster_manager = Some(manager);
        self
    }

    pub fn with_shard_coordinator(
        mut self,
        coordinator: Arc<crate::sharding::ShardCoordinator>,
    ) -> Self {
        self.shard_coordinator = Some(coordinator);
        self
    }

    pub fn with_incoming_channel(
        mut self,
        rx: mpsc::Receiver<(super::transport::SyncStream, String)>,
    ) -> Self {
        self.incoming_rx = Some(rx);
        self
    }

    /// Start the sync worker
    pub async fn run(self) {
        info!("Starting sync worker for node {}", self.local_node_id);

        // Start TCP server
        let server = match SyncServer::bind(
            &self.listen_addr,
            self.keyfile_path.clone(),
            self.local_node_id.clone(),
        )
        .await
        {
            Ok(s) => Arc::new(s),
            Err(e) => {
                error!("Failed to start sync server: {}", e);
                return;
            }
        };

        // Spawn server accept loop
        let accept_pool = self.pool.clone();
        let accept_state = self.state.clone();
        let accept_storage = self.storage.clone();
        let accept_sync_log = self.sync_log.clone();
        let accept_cluster_manager = self.cluster_manager.clone();
        let server_clone = server.clone();
        tokio::spawn(async move {
            loop {
                match server_clone.accept().await {
                    Ok((stream, addr)) => {
                        let pool = accept_pool.clone();
                        let state = accept_state.clone();
                        let storage = accept_storage.clone();
                        let sync_log = accept_sync_log.clone();
                        let cluster_manager = accept_cluster_manager.clone();
                        tokio::spawn(async move {
                            if let Err(e) = Self::handle_connection(
                                stream,
                                addr,
                                pool,
                                state,
                                storage,
                                sync_log,
                                cluster_manager,
                            )
                            .await
                            {
                                error!("Connection handler error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Accept error: {}", e);
                    }
                }
            }
        });

        self.run_background().await;
    }

    /// Run background tasks (without binding a port)
    pub async fn run_background(mut self) {
        // Start incoming channel handler if present
        if let Some(mut rx) = self.incoming_rx.take() {
            let pool = self.pool.clone();
            let state = self.state.clone();
            let storage = self.storage.clone();
            let sync_log = self.sync_log.clone();
            let keyfile_path = self.keyfile_path.clone();
            let cluster_manager = self.cluster_manager.clone();

            tokio::spawn(async move {
                while let Some((stream, addr)) = rx.recv().await {
                    let pool = pool.clone();
                    let state = state.clone();
                    let storage = storage.clone();
                    let sync_log = sync_log.clone();
                    let keyfile = keyfile_path.clone();
                    let cluster_manager = cluster_manager.clone();

                    tokio::spawn(async move {
                        // We must authenticate the stream first, as it comes raw from the multiplexer
                        // The multiplexer already verified the magic header, so skip reading it again
                        match super::transport::SyncServer::authenticate_standalone_skip_magic(
                            stream, &keyfile,
                        )
                        .await
                        {
                            Ok(auth_stream) => {
                                if let Err(e) = Self::handle_connection(
                                    auth_stream,
                                    addr,
                                    pool,
                                    state,
                                    storage,
                                    sync_log,
                                    cluster_manager,
                                )
                                .await
                                {
                                    error!("Connection handler error: {}", e);
                                }
                            }
                            Err(e) => {
                                error!("Authentication failed for {}: {}", addr, e);
                            }
                        }
                    });
                }
            });
        }

        // Main worker loop
        let mut sync_interval = tokio::time::interval(self.config.sync_interval);
        let mut heartbeat_interval = tokio::time::interval(self.config.heartbeat_interval);
        let mut health_check_interval = tokio::time::interval(self.config.dead_node_timeout / 2);

        loop {
            tokio::select! {
                // Handle commands
                Some(cmd) = self.command_rx.recv() => {
                    match cmd {
                        SyncCommand::Shutdown => {
                            info!("Sync worker shutting down");
                            break;
                        }
                        SyncCommand::AddPeer { node_id, sync_addr, http_addr } => {
                            self.state.add_peer(node_id, sync_addr, http_addr);
                            self.state.persist();
                        }
                        SyncCommand::RemovePeer { node_id } => {
                            self.state.remove_peer(&node_id);
                            self.state.persist();
                        }
                        SyncCommand::RequestFullSync { peer_addr } => {
                            if let Err(e) = self.request_full_sync(&peer_addr).await {
                                error!("Full sync request failed: {}", e);
                            }
                        }
                    }
                }

                // Periodic sync
                _ = sync_interval.tick() => {
                    self.sync_with_peers().await;
                }

                // Periodic heartbeat
                _ = heartbeat_interval.tick() => {
                    self.send_heartbeats().await;
                }

                // Health check
                _ = health_check_interval.tick() => {
                    self.check_dead_nodes().await;
                }
            }
        }

        // Persist state on shutdown
        self.state.persist();
    }

    /// Sync with all connected peers
    async fn sync_with_peers(&self) {
        // Get peers from both SyncState (persisted) and ClusterManager (discovered)
        let mut peers = self.state.get_peers();
        debug!("sync_with_peers: found {} persisted peers", peers.len());

        // Also get peers from ClusterManager if available
        if let Some(ref manager) = self.cluster_manager {
            let cluster_members = manager.state().get_all_members();
            debug!(
                "sync_with_peers: ClusterManager has {} members",
                cluster_members.len()
            );
            for member in cluster_members {
                // Skip self
                if member.node.id == self.local_node_id {
                    continue;
                }
                // Check if we already have this peer
                let already_known = peers.iter().any(|p| p.node_id == member.node.id);
                if !already_known {
                    debug!(
                        "sync_with_peers: discovered peer {} at {}",
                        member.node.id, member.node.address
                    );
                    // Add to SyncState for future syncs
                    self.state.add_peer(
                        member.node.id.clone(),
                        member.node.address.clone(),     // sync address
                        member.node.api_address.clone(), // http address
                    );
                    // Add to local list for this sync cycle
                    peers.push(super::state::PeerInfo {
                        node_id: member.node.id,
                        sync_address: member.node.address,
                        http_address: member.node.api_address,
                        last_seen: std::time::Instant::now(),
                        is_connected: false,
                    });
                }
            }
        } else {
            debug!("sync_with_peers: no ClusterManager");
        }

        debug!("sync_with_peers: {} peers total", peers.len());

        for peer in peers {
            debug!(
                "sync_with_peers: syncing with {} at {}",
                peer.node_id, peer.sync_address
            );
            if !peer.is_connected {
                // Try to connect
                if self.pool.connect(&peer.sync_address).await.is_ok() {
                    self.state.set_peer_connected(&peer.node_id, true);
                }
            }

            if peer.is_connected || self.pool.connect(&peer.sync_address).await.is_ok() {
                self.state.set_peer_connected(&peer.node_id, true);

                // Mark ourselves as syncing in cluster state
                if let Some(ref mgr) = self.cluster_manager {
                    mgr.state().mark_status(
                        &self.local_node_id,
                        crate::cluster::state::NodeStatus::Syncing,
                    );
                }

                // Sync loop - keep fetching while there's more data
                // User requested no page limit to sync millions of documents
                let mut pages = 0;
                let mut has_more = true;

                while has_more {
                    match self.incremental_sync(&peer.sync_address).await {
                        Ok(more) => {
                            has_more = more;
                            pages += 1;
                        }
                        Err(e) => {
                            warn!("Sync with {} failed: {}", peer.node_id, e);
                            self.state.set_peer_connected(&peer.node_id, false);
                            self.pool.disconnect(&peer.sync_address).await;
                            has_more = false;
                        }
                    }
                }

                // Mark ourselves as active again
                if let Some(ref mgr) = self.cluster_manager {
                    mgr.state().mark_status(
                        &self.local_node_id,
                        crate::cluster::state::NodeStatus::Active,
                    );
                }

                if pages > 0 {
                    debug!(
                        "Synced {} batches from {} (finished, has_more={})",
                        pages, peer.node_id, has_more
                    );
                }
            } else {
                debug!("sync_with_peers: failed to connect to {}", peer.node_id);
            }
        }
    }

    /// Incremental sync with a peer
    /// Pulls entries FROM the peer that we haven't received yet
    /// Returns true if there are more entries to sync
    async fn incremental_sync(&self, peer_addr: &str) -> Result<bool, TransportError> {
        // Get the last sequence we received from this peer's perspective
        // For pull-based sync, we ask the peer: "give me entries after sequence X from YOUR log"
        // We track this using get_origin_sequence keyed by peer address
        let after_seq = self.state.get_origin_sequence(peer_addr);

        debug!("incremental_sync: {} after_seq={}", peer_addr, after_seq);

        // Request sync from peer
        let request = SyncMessage::IncrementalSyncRequest {
            from_node: self.local_node_id.clone(),
            after_sequence: after_seq,
            max_batch_bytes: self.config.max_batch_bytes,
        };

        self.pool.send(peer_addr, &request).await?;

        // Wait for response
        let response = self.pool.receive(peer_addr).await?;

        match response {
            SyncMessage::SyncBatch {
                entries,
                has_more,
                current_sequence,
                ..
            } => {
                debug!(
                    "incremental_sync: {} entries from {} (seq={}) has_more={}",
                    entries.len(),
                    peer_addr,
                    current_sequence,
                    has_more
                );

                // Group consecutive entries by (database, collection, operation) for batching
                let mut batch_start = 0;
                while batch_start < entries.len() {
                    let first = &entries[batch_start];

                    // Only batch data operations
                    if matches!(
                        first.operation,
                        Operation::Insert | Operation::Update | Operation::Delete
                    ) {
                        let mut batch_end = batch_start + 1;
                        while batch_end < entries.len() {
                            let next = &entries[batch_end];
                            if next.database == first.database
                                && next.collection == first.collection
                                && next.operation == first.operation
                            {
                                batch_end += 1;
                            } else {
                                break;
                            }
                        }

                        // Process batch
                        let batch = &entries[batch_start..batch_end];
                        self.apply_batch(batch).await?;
                        batch_start = batch_end;
                    } else {
                        // Single entry processing for schema changes
                        self.apply_entry(first).await?;
                        batch_start += 1;
                    }
                }

                // Only update origin_sequence if we ACTUALLY received entries.
                // We must NOT update to current_sequence if entries are empty, because that implies
                // the server hasn't persisted the data yet (race condition) or we'd skip data.
                if let Some(max_seq) = entries.iter().map(|e| e.sequence).max() {
                    if max_seq > after_seq {
                        self.state.update_origin_sequence(peer_addr, max_seq);
                    }
                }

                debug!("Applied {} entries from {}", entries.len(), peer_addr);

                // Calculate has_more based on what the server claims is the head vs what we have
                // If server has seq 100, and we are at 90 (either via max_seq or after_seq), we have more.
                // Note: current_sequence from server is the "head", entries max is what we just got.
                let latest_we_have = entries
                    .iter()
                    .map(|e| e.sequence)
                    .max()
                    .unwrap_or(after_seq);
                let actual_has_more = current_sequence > latest_we_have;

                Ok(actual_has_more)
            }
            _ => {
                warn!("Unexpected response from {}", peer_addr);
                Ok(false)
            }
        }
    }

    /// Apply a batch of sync entries to local storage
    async fn apply_batch(&self, entries: &[SyncEntry]) -> Result<(), TransportError> {
        if entries.is_empty() {
            return Ok(());
        }

        let first = &entries[0];
        let database = &first.database;
        let collection = &first.collection;
        let operation = first.operation;

        // Skip physical shard collections - sharded data is partitioned, NOT replicated cluster-wide
        // Physical shards have names like "users_s0", "users_s1" etc.
        let is_physical_shard = collection.contains("_s")
            && collection
                .chars()
                .last()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
        if is_physical_shard {
            debug!(
                "apply_batch: Skipping physical shard collection {} (partitioned, not replicated)",
                collection
            );
            return Ok(());
        }

        // Ensure database and collection exist for Write operations
        if matches!(operation, Operation::Insert | Operation::Update) {
            // Create database if it doesn't exist
            if self.storage.get_database(database).is_err() {
                let _ = self.storage.create_database(database.clone());
            }

            if let Ok(db) = self.storage.get_database(database) {
                if db.get_collection(collection).is_err() {
                    let _ = db.create_collection(collection.clone(), None);
                }
            }
        }

        match operation {
            Operation::Insert | Operation::Update => {
                if let Ok(db) = self.storage.get_database(database) {
                    if let Ok(coll) = db.get_collection(collection) {
                        let mut batch_data = Vec::with_capacity(entries.len());

                        for entry in entries {
                            // Check for duplicate
                            if self
                                .state
                                .is_duplicate(&entry.origin_node, entry.origin_sequence)
                            {
                                continue;
                            }

                            if let Some(ref data) = entry.document_data {
                                let doc: serde_json::Value =
                                    serde_json::from_slice(data).map_err(|e| {
                                        TransportError::DecodeError(format!(
                                            "Invalid document: {}",
                                            e
                                        ))
                                    })?;
                                batch_data.push((entry.document_key.clone(), doc));
                            }
                        }

                        if !batch_data.is_empty() {
                            if let Err(e) = coll.upsert_batch(batch_data) {
                                warn!(
                                    "apply_batch: upsert failed for {}.{}: {}",
                                    database, collection, e
                                );
                            }
                        }
                    }
                }
            }
            Operation::Delete => {
                if let Ok(db) = self.storage.get_database(database) {
                    if let Ok(coll) = db.get_collection(collection) {
                        let mut keys_to_delete = Vec::with_capacity(entries.len());

                        for entry in entries {
                            // Check for duplicate
                            if self
                                .state
                                .is_duplicate(&entry.origin_node, entry.origin_sequence)
                            {
                                continue;
                            }
                            keys_to_delete.push(entry.document_key.clone());
                        }

                        if !keys_to_delete.is_empty() {
                            let _ = coll.delete_batch(keys_to_delete);
                        }
                    }
                }
            }
            _ => {
                // Other operations should be handled one by one via apply_entry
                // But if they came here, they are grouped, so we iterate
                for entry in entries {
                    self.apply_entry(entry).await?;
                }
            }
        }

        // Update origin sequence for all processed entries
        for entry in entries {
            self.state
                .update_origin_sequence(&entry.origin_node, entry.origin_sequence);
        }

        Ok(())
    }

    /// Apply a sync entry to local storage
    async fn apply_entry(&self, entry: &SyncEntry) -> Result<(), TransportError> {
        // Check for duplicate
        if self
            .state
            .is_duplicate(&entry.origin_node, entry.origin_sequence)
        {
            return Ok(());
        }

        // Skip physical shard collections - sharded data is partitioned, NOT replicated cluster-wide
        let is_physical_shard = entry.collection.contains("_s")
            && entry
                .collection
                .chars()
                .last()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false);
        if is_physical_shard {
            return Ok(());
        }

        // Apply based on operation type
        match entry.operation {
            Operation::Insert | Operation::Update => {
                if let Some(ref data) = entry.document_data {
                    let doc: serde_json::Value = serde_json::from_slice(data).map_err(|e| {
                        TransportError::DecodeError(format!("Invalid document: {}", e))
                    })?;

                    // Create database if it doesn't exist
                    if self.storage.get_database(&entry.database).is_err() {
                        let _ = self.storage.create_database(entry.database.clone());
                    }

                    if let Ok(db) = self.storage.get_database(&entry.database) {
                        // Create collection if it doesn't exist
                        if db.get_collection(&entry.collection).is_err() {
                            let _ = db.create_collection(entry.collection.clone(), None);
                        }

                        if let Ok(coll) = db.get_collection(&entry.collection) {
                            if let Err(e) =
                                coll.upsert_batch(vec![(entry.document_key.clone(), doc)])
                            {
                                warn!(
                                    "apply_entry: upsert failed for {}: {}",
                                    entry.document_key, e
                                );
                            }
                        }
                    }
                }
            }
            Operation::Delete => {
                if let Ok(db) = self.storage.get_database(&entry.database) {
                    if let Ok(coll) = db.get_collection(&entry.collection) {
                        let _ = coll.delete(&entry.document_key);
                    }
                }
            }
            Operation::CreateDatabase => {
                let _ = self.storage.create_database(entry.database.clone());
            }
            Operation::DeleteDatabase => {
                let _ = self.storage.delete_database(&entry.database);
            }
            Operation::CreateCollection => {
                if let Ok(db) = self.storage.get_database(&entry.database) {
                    // Parse metadata from entry.document_data
                    let metadata: Option<serde_json::Value> = entry
                        .document_data
                        .as_ref()
                        .and_then(|d| serde_json::from_slice(d).ok());

                    // Extract collection type from metadata
                    let collection_type = metadata
                        .as_ref()
                        .and_then(|m| m.get("type"))
                        .and_then(|t| t.as_str())
                        .map(|s| s.to_string());

                    // Create the collection with type
                    if let Ok(()) = db.create_collection(entry.collection.clone(), collection_type)
                    {
                        // Apply shard configuration if present
                        if let Some(ref meta) = metadata {
                            if let Some(shard_config_obj) = meta.get("shardConfig") {
                                if !shard_config_obj.is_null() {
                                    if let Ok(coll) = db.get_collection(&entry.collection) {
                                        // Parse shard config (CollectionShardConfig uses u16)
                                        let num_shards = shard_config_obj
                                            .get("num_shards")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(1)
                                            as u16;
                                        let shard_key = shard_config_obj
                                            .get("shard_key")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("_key")
                                            .to_string();
                                        let replication_factor = shard_config_obj
                                            .get("replication_factor")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(1)
                                            as u16;

                                        let shard_config =
                                            crate::sharding::coordinator::CollectionShardConfig {
                                                num_shards,
                                                shard_key,
                                                replication_factor,
                                            };

                                        let _ = coll.set_shard_config(&shard_config);
                                        debug!(
                                            "Replicated collection {} with shard config: {:?}",
                                            entry.collection, shard_config
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Operation::DeleteCollection => {
                if let Ok(db) = self.storage.get_database(&entry.database) {
                    let _ = db.delete_collection(&entry.collection);
                }
            }
            Operation::TruncateCollection => {
                if let Ok(db) = self.storage.get_database(&entry.database) {
                    if let Ok(coll) = db.get_collection(&entry.collection) {
                        // Check if sharded and truncate physical shards first
                        if let Some(shard_config) = coll.get_shard_config() {
                            if shard_config.num_shards > 0 {
                                for shard_id in 0..shard_config.num_shards {
                                    let physical_name =
                                        format!("{}_s{}", entry.collection, shard_id);
                                    if let Ok(shard_coll) = db.get_collection(&physical_name) {
                                        let _ = shard_coll.truncate();
                                    }
                                }
                            }
                        }
                        // Truncate the logical collection
                        let _ = coll.truncate();
                    }
                }
            }
            Operation::PutBlobChunk | Operation::DeleteBlob => {
                // TODO: Handle blob operations
            }
            Operation::ColumnarInsert => {
                if let Some(ref data) = entry.document_data {
                    let row: serde_json::Value = serde_json::from_slice(data).map_err(|e| {
                        TransportError::DecodeError(format!("Invalid columnar row: {}", e))
                    })?;

                    // Create database if it doesn't exist
                    if self.storage.get_database(&entry.database).is_err() {
                        let _ = self.storage.create_database(entry.database.clone());
                    }

                    if let Ok(db) = self.storage.get_database(&entry.database) {
                        // Load columnar collection and insert with specific UUID
                        match crate::storage::columnar::ColumnarCollection::load(
                            entry.collection.clone(),
                            &entry.database,
                            db.db_arc(),
                        ) {
                            Ok(col) => {
                                // Insert with specific UUID (idempotent)
                                if let Err(e) = col.insert_row_with_id(&entry.document_key, row) {
                                    warn!(
                                        "apply_entry: columnar insert failed for {}: {}",
                                        entry.document_key, e
                                    );
                                }
                            }
                            Err(e) => {
                                warn!(
                                    "apply_entry: failed to load columnar collection {}: {}",
                                    entry.collection, e
                                );
                            }
                        }
                    }
                }
            }
            Operation::ColumnarDelete => {
                if let Ok(db) = self.storage.get_database(&entry.database) {
                    match crate::storage::columnar::ColumnarCollection::load(
                        entry.collection.clone(),
                        &entry.database,
                        db.db_arc(),
                    ) {
                        Ok(col) => {
                            if let Err(e) = col.delete_row(&entry.document_key) {
                                warn!(
                                    "apply_entry: columnar delete failed for {}: {}",
                                    entry.document_key, e
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                "apply_entry: failed to load columnar collection {}: {}",
                                entry.collection, e
                            );
                        }
                    }
                }
            }
            Operation::ColumnarCreateCollection => {
                // Create database if it doesn't exist
                if self.storage.get_database(&entry.database).is_err() {
                    let _ = self.storage.create_database(entry.database.clone());
                }

                if let Ok(db) = self.storage.get_database(&entry.database) {
                    // Parse column definitions from entry.document_data
                    if let Some(ref data) = entry.document_data {
                        if let Ok(columns) =
                            serde_json::from_slice::<Vec<crate::storage::columnar::ColumnDef>>(data)
                        {
                            let _ = crate::storage::columnar::ColumnarCollection::new(
                                entry.collection.clone(),
                                &entry.database,
                                db.db_arc(),
                                columns,
                                crate::storage::columnar::CompressionType::Lz4,
                            );
                        }
                    }
                }
            }
            Operation::ColumnarDropCollection => {
                if let Ok(db) = self.storage.get_database(&entry.database) {
                    if let Ok(col) = crate::storage::columnar::ColumnarCollection::load(
                        entry.collection.clone(),
                        &entry.database,
                        db.db_arc(),
                    ) {
                        let _ = col.drop();
                    }
                }
            }
            Operation::ColumnarTruncate => {
                if let Ok(db) = self.storage.get_database(&entry.database) {
                    if let Ok(col) = crate::storage::columnar::ColumnarCollection::load(
                        entry.collection.clone(),
                        &entry.database,
                        db.db_arc(),
                    ) {
                        if let Err(e) = col.truncate() {
                            warn!(
                                "apply_entry: columnar truncate failed for {}: {}",
                                entry.collection, e
                            );
                        }
                    }
                }
            }
        }

        // Update origin sequence
        self.state
            .update_origin_sequence(&entry.origin_node, entry.origin_sequence);

        Ok(())
    }

    /// Request full sync from a peer (for new nodes)
    async fn request_full_sync(&self, peer_addr: &str) -> Result<(), TransportError> {
        self.pool.connect(peer_addr).await?;

        let request = SyncMessage::FullSyncRequest {
            from_node: self.local_node_id.clone(),
        };

        self.pool.send(peer_addr, &request).await?;

        // Process full sync messages
        loop {
            let msg = self.pool.receive(peer_addr).await?;

            match msg {
                SyncMessage::FullSyncStart {
                    total_databases,
                    total_documents,
                    ..
                } => {
                    info!(
                        "Starting full sync: {} databases, {} documents",
                        total_databases, total_documents
                    );
                }
                SyncMessage::FullSyncDatabase { name } => {
                    let _ = self.storage.create_database(name.clone());
                }
                SyncMessage::FullSyncCollection { database, name, .. } => {
                    if let Ok(db) = self.storage.get_database(&database) {
                        let _ = db.create_collection(name.clone(), None);
                    }
                }
                SyncMessage::FullSyncDocuments {
                    database,
                    collection,
                    data,
                    compressed,
                    doc_count,
                } => {
                    let docs_data = if compressed {
                        lz4_flex::decompress_size_prepended(&data).map_err(|e| {
                            TransportError::DecodeError(format!("Decompression failed: {}", e))
                        })?
                    } else {
                        data
                    };

                    let docs: Vec<serde_json::Value> = bincode::deserialize(&docs_data)
                        .map_err(|e| TransportError::DecodeError(e.to_string()))?;

                    for doc in docs {
                        if let Some(key) = doc.get("_key").and_then(|k| k.as_str()) {
                            if let Ok(db) = self.storage.get_database(&database) {
                                if let Ok(coll) = db.get_collection(&collection) {
                                    let _ = coll.upsert_batch(vec![(key.to_string(), doc)]);
                                }
                            }
                        }
                    }
                    debug!(
                        "Synced {} documents to {}.{}",
                        doc_count, database, collection
                    );
                }
                SyncMessage::FullSyncComplete { final_sequence } => {
                    info!("Full sync complete, final sequence: {}", final_sequence);
                    break;
                }
                _ => {
                    warn!("Unexpected message during full sync");
                }
            }
        }

        Ok(())
    }

    /// Send heartbeats to all peers
    async fn send_heartbeats(&mut self) {
        let peers = self.state.get_peers();
        if peers.is_empty() {
            return;
        }

        let stats = self.collect_local_stats();
        let heartbeat = SyncMessage::Heartbeat {
            node_id: self.local_node_id.clone(),
            sequence: self.state.current_sequence(),
            stats,
        };

        for peer in self.state.get_peers() {
            if peer.is_connected {
                if let Err(e) = self.pool.send(&peer.sync_address, &heartbeat).await {
                    debug!("Failed to send heartbeat to {}: {}", peer.node_id, e);
                }
            }
        }
    }

    /// Check for dead nodes and remove them
    async fn check_dead_nodes(&self) {
        let dead = self.state.dead_nodes(self.config.dead_node_timeout);

        if dead.is_empty() {
            return;
        }

        for node_id in &dead {
            warn!("Node {} is dead, removing from cluster", node_id);
            self.state.remove_peer(node_id);

            // Also update cluster manager if present
            if let Some(ref mgr) = self.cluster_manager {
                mgr.state().remove_member(node_id);
            }
        }

        if !self.state.get_peers().is_empty() {
            self.state.persist();
        }

        // Trigger shard rebalancing if we have a coordinator
        if let Some(ref coordinator) = self.shard_coordinator {
            info!("Triggering automatic shard rebalance after node death");
            let coordinator = coordinator.clone();
            tokio::spawn(async move {
                if let Err(e) = coordinator.rebalance().await {
                    error!("Failed to rebalance shards after node death: {}", e);
                }
            });
        }
    }

    /// Collect local node statistics
    fn collect_local_stats(&mut self) -> NodeStats {
        self.system.refresh_cpu_usage();
        self.system.refresh_memory();

        let cpu_usage = self
            .system
            .cpus()
            .first()
            .map(|c| c.cpu_usage())
            .unwrap_or(0.0);
        let memory_used = self.system.used_memory();
        let disk_used = 0; // Disk stats require different API in newer sysinfo

        // Count documents (estimate)
        let document_count = 0; // TODO: Add actual count
        let collections_count = 0; // TODO: Add actual count

        NodeStats {
            cpu_usage,
            memory_used,
            disk_used,
            document_count,
            collections_count,
        }
    }

    /// Handle incoming connection
    pub async fn handle_connection(
        mut stream: super::transport::SyncStream,
        _addr: String,
        _pool: Arc<ConnectionPool>,
        state: Arc<SyncState>,
        storage: Arc<StorageEngine>,
        sync_log: Arc<super::log::SyncLog>,
        cluster_manager: Option<Arc<crate::cluster::manager::ClusterManager>>,
    ) -> Result<(), TransportError> {
        use crate::cluster::HybridLogicalClock;
        let hlc = HybridLogicalClock::now(sync_log.node_id());

        loop {
            // Read message header
            let mut header = [0u8; 5];
            if tokio::io::AsyncReadExt::read_exact(&mut stream, &mut header)
                .await
                .is_err()
            {
                break;
            }

            let compressed = header[0] == 1;
            let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);

            if len > 10 * 1024 * 1024 {
                break;
            }

            let mut data = vec![0u8; len as usize];
            if tokio::io::AsyncReadExt::read_exact(&mut stream, &mut data)
                .await
                .is_err()
            {
                break;
            }

            let payload = if compressed {
                lz4_flex::decompress_size_prepended(&data).unwrap_or_default()
            } else {
                data
            };

            let msg: SyncMessage = match bincode::deserialize(&payload) {
                Ok(m) => m,
                Err(_) => break,
            };

            // Handle message
            match msg {
                SyncMessage::Heartbeat {
                    node_id,
                    sequence,
                    stats,
                } => {
                    // Update sync state heartbeat
                    state.update_heartbeat(&node_id, stats);

                    // Also update cluster state heartbeat so admin UI shows nodes as connected
                    if let Some(ref cm) = cluster_manager {
                        cm.state().update_heartbeat(&node_id, sequence, None);
                    }
                }
                SyncMessage::IncrementalSyncRequest {
                    from_node: _,
                    after_sequence,
                    max_batch_bytes,
                } => {
                    // Fetch entries from log
                    let limit = (max_batch_bytes / 1024).max(100) as usize; // Rough estimate
                    let log_entries = sync_log.get_entries_after(after_sequence, limit);
                    let current_seq = sync_log.current_sequence();

                    debug!(
                        "IncrementalSyncRequest: after_seq={}, current_seq={}, found {} entries",
                        after_sequence,
                        current_seq,
                        log_entries.len()
                    );

                    // Convert to SyncEntry
                    let entries: Vec<SyncEntry> =
                        log_entries.iter().map(|e| e.to_sync_entry(&hlc)).collect();

                    let has_more = !entries.is_empty()
                        && entries.last().map(|e| e.sequence).unwrap_or(0) < current_seq;

                    let response = SyncMessage::SyncBatch {
                        entries,
                        has_more,
                        current_sequence: current_seq,
                        compressed: false,
                    };

                    // Use proper message framing that the client expects
                    if let Err(e) =
                        super::transport::ConnectionPool::write_message(&mut stream, &response)
                            .await
                    {
                        warn!("Failed to send SyncBatch response: {}", e);
                        break;
                    }
                }
                SyncMessage::FullSyncRequest { from_node } => {
                    info!("Full sync request from {}", from_node);

                    // Enumerate databases and collections
                    let databases = storage.list_databases();
                    let mut total_collections = 0u32;
                    let mut total_documents = 0u64;

                    for db_name in &databases {
                        if let Ok(db) = storage.get_database(db_name) {
                            let colls = db.list_collections();
                            total_collections += colls.len() as u32;
                            for coll_name in &colls {
                                if let Ok(coll) = db.get_collection(coll_name) {
                                    total_documents += coll.count() as u64;
                                }
                            }
                        }
                    }

                    // Send start message
                    let start = SyncMessage::FullSyncStart {
                        total_databases: databases.len() as u32,
                        total_collections,
                        total_documents,
                    };
                    let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, &start.encode()).await;

                    // Send each database
                    for db_name in &databases {
                        let db_msg = SyncMessage::FullSyncDatabase {
                            name: db_name.clone(),
                        };
                        let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, &db_msg.encode())
                            .await;

                        if let Ok(db) = storage.get_database(db_name) {
                            let colls = db.list_collections();
                            for coll_name in colls {
                                // Send collection
                                let coll_msg = SyncMessage::FullSyncCollection {
                                    database: db_name.clone(),
                                    name: coll_name.clone(),
                                    shard_config: None, // TODO: get from collection
                                };
                                let _ = tokio::io::AsyncWriteExt::write_all(
                                    &mut stream,
                                    &coll_msg.encode(),
                                )
                                .await;

                                // Send documents in batches
                                if let Ok(coll) = db.get_collection(&coll_name) {
                                    let mut batch = Vec::new();
                                    let mut batch_count = 0u32;

                                    for doc in coll.scan(None) {
                                        batch.push(doc.to_value());
                                        batch_count += 1;

                                        if batch.len() >= 1000 {
                                            // Send batch
                                            let data =
                                                bincode::serialize(&batch).unwrap_or_default();
                                            let compress = data.len() > 10 * 1024;
                                            let final_data = if compress {
                                                lz4_flex::compress_prepend_size(&data)
                                            } else {
                                                data
                                            };

                                            let doc_msg = SyncMessage::FullSyncDocuments {
                                                database: db_name.clone(),
                                                collection: coll_name.clone(),
                                                data: final_data,
                                                compressed: compress,
                                                doc_count: batch_count,
                                            };
                                            let _ = tokio::io::AsyncWriteExt::write_all(
                                                &mut stream,
                                                &doc_msg.encode(),
                                            )
                                            .await;

                                            batch.clear();
                                            batch_count = 0;
                                        }
                                    }

                                    // Send remaining
                                    if !batch.is_empty() {
                                        let data = bincode::serialize(&batch).unwrap_or_default();
                                        let compress = data.len() > 10 * 1024;
                                        let final_data = if compress {
                                            lz4_flex::compress_prepend_size(&data)
                                        } else {
                                            data
                                        };

                                        let doc_msg = SyncMessage::FullSyncDocuments {
                                            database: db_name.clone(),
                                            collection: coll_name.clone(),
                                            data: final_data,
                                            compressed: compress,
                                            doc_count: batch_count,
                                        };
                                        let _ = tokio::io::AsyncWriteExt::write_all(
                                            &mut stream,
                                            &doc_msg.encode(),
                                        )
                                        .await;
                                    }
                                }
                            }
                        }
                    }

                    // Send complete
                    let complete = SyncMessage::FullSyncComplete {
                        final_sequence: sync_log.current_sequence(),
                    };
                    let _ =
                        tokio::io::AsyncWriteExt::write_all(&mut stream, &complete.encode()).await;
                }
                _ => {}
            }
        }

        Ok(())
    }
}

/// Create a command channel for the sync worker
pub fn create_command_channel() -> (mpsc::Sender<SyncCommand>, mpsc::Receiver<SyncCommand>) {
    mpsc::channel(100)
}
