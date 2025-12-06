use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::hlc::HlcGenerator;
use super::{ClusterConfig, Operation, PersistentReplicationLog, ReplicationEntry};
use crate::StorageEngine;

/// Messages exchanged between nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReplicationMessage {
    /// Request entries after a sequence number
    SyncRequest {
        from_node: String,
        after_sequence: u64,
    },

    /// Response with entries
    SyncResponse {
        from_node: String,
        entries: Vec<ReplicationEntry>,
        current_sequence: u64,
    },

    /// Push new entries to peers
    PushEntries {
        from_node: String,
        entries: Vec<ReplicationEntry>,
    },

    /// Acknowledge received entries
    Ack {
        from_node: String,
        up_to_sequence: u64,
    },

    /// Heartbeat/ping
    Ping {
        from_node: String,
        /// The sender's replication address so others can connect back
        replication_addr: Option<String>,
    },

    /// Heartbeat response
    Pong {
        from_node: String,
        current_sequence: u64,
        /// List of known peer addresses for discovery
        known_peers: Vec<String>,
    },

    /// Leave notification
    LeaveNotification { from_node: String },

    // ==================== Full Sync Messages ====================
    /// Request full sync (for new nodes)
    FullSyncRequest { from_node: String },

    /// Start of full sync - metadata
    FullSyncStart {
        from_node: String,
        total_databases: usize,
        total_collections: usize,
        total_documents: usize,
        current_sequence: u64,
    },

    /// Database definition
    FullSyncDatabase { name: String },

    /// Collection definition
    FullSyncCollection { database: String, name: String },

    /// Batch of documents
    FullSyncDocuments {
        database: String,
        collection: String,
        documents: Vec<Value>,
    },

    /// End of full sync
    FullSyncComplete {
        from_node: String,
        current_sequence: u64,
    },

    /// Full sync progress update
    FullSyncProgress {
        from_node: String,
        phase: String,
        current: usize,
        total: usize,
    },

    // ==================== Authentication Messages ====================
    /// Authentication challenge (sent by server when keyfile is configured)
    AuthChallenge {
        challenge: String,
    },

    /// Authentication response (client responds with HMAC of challenge)
    AuthResponse {
        response: String,
    },

    /// Authentication result
    AuthResult {
        success: bool,
        message: String,
    },
}

impl ReplicationMessage {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(self).expect("Failed to serialize message");
        bytes.push(b'\n'); // Line delimiter
        bytes
    }
}

/// Tracks the state of each peer
#[derive(Debug, Clone)]
pub struct PeerState {
    pub address: String,
    pub node_id: Option<String>, // Learned from Ping/Pong messages
    pub last_seen: std::time::Instant,
    pub last_sequence_sent: u64,  // Highest of OUR sequences sent to them
    pub last_sequence_acked: u64, // Highest of OUR sequences they confirmed receiving (for lag)
    pub last_sequence_received: u64, // Highest of THEIR sequences we received (for sync requests)
    pub is_connected: bool,
}

/// The replication service handles peer-to-peer communication
pub struct ReplicationService {
    storage: StorageEngine,
    config: ClusterConfig,
    replication_log: PersistentReplicationLog,
    hlc_generator: Arc<HlcGenerator>,
    peer_states: Arc<RwLock<HashMap<String, PeerState>>>,
    shutdown_tx: Arc<RwLock<Option<mpsc::Sender<()>>>>,
}

impl ReplicationService {
    /// Key used to store peer addresses in _system._config
    const PEERS_CONFIG_KEY: &'static str = "cluster_peers";

    pub fn new(storage: StorageEngine, config: ClusterConfig, data_dir: &str) -> Self {
        let node_id = config.node_id.clone();

        // Create persistent replication log
        let replication_log = PersistentReplicationLog::new(
            node_id.clone(),
            data_dir,
            100000, // Keep last 100k entries
        )
        .expect("Failed to create replication log");

        let hlc_generator = Arc::new(HlcGenerator::new(node_id.clone()));

        // Initialize peer states from config
        let mut peer_states = HashMap::new();
        for peer in &config.peers {
            peer_states.insert(
                peer.clone(),
                PeerState {
                    address: peer.clone(),
                    node_id: None,
                    last_seen: std::time::Instant::now(),
                    last_sequence_sent: 0,
                    last_sequence_acked: 0,
                    last_sequence_received: 0,
                    is_connected: false,
                },
            );
        }

        // Load saved peers from _system._config
        let saved_peers = Self::load_saved_peers(&storage);
        for peer in saved_peers {
            // Skip if already in config or if it's our own address
            if peer_states.contains_key(&peer) || peer == config.replication_addr() {
                continue;
            }
            tracing::debug!("[PEER] Loading saved peer from config: {}", peer);
            peer_states.insert(
                peer.clone(),
                PeerState {
                    address: peer,
                    node_id: None,
                    last_seen: std::time::Instant::now(),
                    last_sequence_sent: 0,
                    last_sequence_acked: 0,
                    last_sequence_received: 0,
                    is_connected: false,
                },
            );
        }

        Self {
            storage,
            config,
            replication_log,
            hlc_generator,
            peer_states: Arc::new(RwLock::new(peer_states)),
            shutdown_tx: Arc::new(RwLock::new(None)),
        }
    }

    /// Load saved peer addresses from _system._config collection
    fn load_saved_peers(storage: &StorageEngine) -> Vec<String> {
        if let Ok(db) = storage.get_database("_system") {
            if let Ok(config_coll) = db.get_collection("_config") {
                if let Ok(doc) = config_coll.get(Self::PEERS_CONFIG_KEY) {
                    if let Some(peers) = doc.data.get("peers").and_then(|p| p.as_array()) {
                        let saved: Vec<String> = peers
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect();
                        tracing::debug!(
                            "[PEER] Loaded {} saved peers from _system._config: {:?}",
                            saved.len(),
                            saved
                        );
                        return saved;
                    }
                }
            }
        }
        tracing::debug!("[PEER] No saved peers found in _system._config");
        Vec::new()
    }

    /// Save peer addresses to _system._config collection
    fn save_peers(&self) {
        let peers: Vec<String> = self.peer_states.read().unwrap().keys().cloned().collect();

        tracing::debug!(
            "[PEER] Saving {} peers to _system._config: {:?}",
            peers.len(),
            peers
        );

        if let Ok(db) = self.storage.get_database("_system") {
            // Create _config collection if it doesn't exist
            if db.get_collection("_config").is_err() {
                tracing::debug!("[PEER] Creating _config collection in _system database");
                if let Err(e) = db.create_collection("_config".to_string()) {
                    tracing::warn!("[PEER] Failed to create _config collection: {}", e);
                    return;
                }
            }

            if let Ok(config_coll) = db.get_collection("_config") {
                let peer_doc = serde_json::json!({
                    "_key": Self::PEERS_CONFIG_KEY,
                    "peers": peers
                });

                // Try update first, then insert if not exists
                if config_coll.get(Self::PEERS_CONFIG_KEY).is_ok() {
                    if let Err(e) = config_coll.update(Self::PEERS_CONFIG_KEY, peer_doc) {
                        tracing::warn!("[PEER] Failed to update saved peers: {}", e);
                    } else {
                        tracing::debug!("[PEER] Updated saved peers in _system._config");
                    }
                } else if let Err(e) = config_coll.insert(peer_doc) {
                    tracing::warn!("[PEER] Failed to save peers: {}", e);
                } else {
                    tracing::debug!("[PEER] Saved peers to _system._config");
                }
            }
        } else {
            tracing::warn!("[PEER] Failed to get _system database for saving peers");
        }
    }

    /// Start the replication service
    pub async fn start(&self) -> anyhow::Result<()> {
        let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
        *self.shutdown_tx.write().unwrap() = Some(shutdown_tx);

        // Start TCP listener for incoming connections
        let listen_addr = self.config.replication_addr();
        let listener = TcpListener::bind(&listen_addr).await?;

        // Get all peers (configured + saved)
        let all_peers: Vec<String> = self.peer_states.read().unwrap().keys().cloned().collect();

        tracing::debug!("╔════════════════════════════════════════════════════════════╗");
        tracing::debug!("║           REPLICATION SERVICE STARTED                       ║");
        tracing::debug!("╠════════════════════════════════════════════════════════════╣");
        tracing::debug!("║ Node ID: {:<49} ║", self.config.node_id);
        tracing::debug!("║ Listening on: {:<44} ║", listen_addr);
        tracing::debug!(
            "║ Current sequence: {:<40} ║",
            self.replication_log.current_sequence()
        );
        tracing::debug!("║ Total peers (config + saved): {:<28} ║", all_peers.len());
        for peer in &all_peers {
            let source = if self.config.peers.contains(peer) {
                "config"
            } else {
                "saved"
            };
            tracing::debug!("║   - {:<44} ({}) ║", peer, source);
        }
        tracing::debug!("╚════════════════════════════════════════════════════════════╝");

        if all_peers.is_empty() {
            tracing::debug!("[PEER] No peers configured - waiting for incoming connections");
        }

        // Spawn peer connection tasks for all peers (configured + saved)
        for peer in all_peers {
            let service = self.clone();
            tracing::debug!("[PEER] Starting sync loop for peer: {}", peer);
            tokio::spawn(async move {
                service.peer_sync_loop(peer).await;
            });
        }

        // Accept incoming connections
        loop {
            tokio::select! {
                result = listener.accept() => {
                    match result {
                        Ok((socket, addr)) => {
                            tracing::debug!("[CONNECT] Incoming connection from {}", addr);
                            let service = self.clone();
                            tokio::spawn(async move {
                                if let Err(e) = service.handle_connection(socket, addr.to_string()).await {
                                    tracing::error!("[CONNECT] Connection error from {}: {}", addr, e);
                                }
                            });
                        }
                        Err(e) => {
                            tracing::error!("[CONNECT] Accept error: {}", e);
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    tracing::debug!("[SHUTDOWN] Replication service shutting down...");
                    self.broadcast_leave().await;
                    tracing::debug!("[SHUTDOWN] Sent leave notifications to peers");
                    break;
                }
            }
        }

        Ok(())
    }

    /// Broadcast leave notification to all peers
    async fn broadcast_leave(&self) {
        let leave_msg = ReplicationMessage::LeaveNotification {
            from_node: self.config.node_id.clone(),
        };

        let peer_addresses: Vec<String> = self
            .peer_states
            .read()
            .unwrap()
            .values()
            .filter(|p| p.is_connected)
            .map(|p| p.address.clone())
            .collect();

        for addr in peer_addresses {
            tracing::debug!("[LEAVE] Notifying peer {} of departure", addr);
            if let Ok(mut stream) = TcpStream::connect(&addr).await {
                let _ = stream.write_all(&leave_msg.to_bytes()).await;
            }
        }
    }

    /// Handle an incoming connection
    async fn handle_connection(&self, socket: TcpStream, addr: String) -> anyhow::Result<()> {
        let (reader, mut writer) = socket.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        // Authentication handshake if keyfile is configured
        if let Some(ref keyfile) = self.config.keyfile {
            // Generate a random challenge
            let challenge = uuid::Uuid::new_v4().to_string();
            tracing::debug!("[AUTH] Sending challenge to {}", addr);
            
            let challenge_msg = ReplicationMessage::AuthChallenge {
                challenge: challenge.clone(),
            };
            writer.write_all(&challenge_msg.to_bytes()).await?;

            // Wait for response
            line.clear();
            let bytes_read = tokio::time::timeout(
                Duration::from_secs(10),
                reader.read_line(&mut line)
            ).await??;
            
            if bytes_read == 0 {
                tracing::warn!("[AUTH] Connection closed during auth from {}", addr);
                return Ok(());
            }

            let response: ReplicationMessage = match serde_json::from_str(&line) {
                Ok(msg) => msg,
                Err(e) => {
                    tracing::warn!("[AUTH] Invalid auth response from {}: {}", addr, e);
                    let result = ReplicationMessage::AuthResult {
                        success: false,
                        message: "Invalid message format".to_string(),
                    };
                    writer.write_all(&result.to_bytes()).await?;
                    return Ok(());
                }
            };

            // Verify the response
            if let ReplicationMessage::AuthResponse { response: auth_response } = response {
                let expected = Self::compute_auth_response(&challenge, keyfile);
                if auth_response == expected {
                    tracing::debug!("[AUTH] Authentication successful from {}", addr);
                    let result = ReplicationMessage::AuthResult {
                        success: true,
                        message: "Authentication successful".to_string(),
                    };
                    writer.write_all(&result.to_bytes()).await?;
                } else {
                    tracing::warn!("[AUTH] Authentication failed from {} - invalid response", addr);
                    let result = ReplicationMessage::AuthResult {
                        success: false,
                        message: "Authentication failed".to_string(),
                    };
                    writer.write_all(&result.to_bytes()).await?;
                    return Ok(());
                }
            } else {
                tracing::warn!("[AUTH] Expected AuthResponse from {}, got {:?}", addr, response);
                let result = ReplicationMessage::AuthResult {
                    success: false,
                    message: "Expected AuthResponse message".to_string(),
                };
                writer.write_all(&result.to_bytes()).await?;
                return Ok(());
            }
        }

        // Track the peer's replication address (learned from Ping messages)
        let mut peer_repl_addr: Option<String> = None;

        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line).await?;
            if bytes_read == 0 {
                tracing::debug!("[DISCONNECT] Connection closed from {}", addr);
                // Mark the peer as disconnected using their replication address
                if let Some(ref repl_addr) = peer_repl_addr {
                    self.set_peer_connected(repl_addr, false);
                    tracing::debug!("[DISCONNECT] Marked peer {} as disconnected", repl_addr);
                }
                break;
            }

            let message: ReplicationMessage = match serde_json::from_str(&line) {
                Ok(msg) => msg,
                Err(e) => {
                    tracing::warn!("[MESSAGE] Invalid message from {}: {}", addr, e);
                    continue;
                }
            };

            // Extract peer's replication address from Ping messages
            if let ReplicationMessage::Ping {
                replication_addr: Some(ref advertised_addr),
                ..
            } = &message
            {
                if let Some(port) = advertised_addr.split(':').last() {
                    if let Some(ip) = addr.split(':').next() {
                        peer_repl_addr = Some(format!("{}:{}", ip, port));
                    }
                }
            }

            // Handle full sync specially - it sends multiple messages
            if let ReplicationMessage::FullSyncRequest { from_node } = &message {
                tracing::debug!("[FULL-SYNC] Request from {}", from_node);
                self.send_full_sync(&mut writer).await?;
                continue;
            }

            // Use the learned replication address if available, otherwise fall back to socket address
            let effective_addr = peer_repl_addr.as_deref().unwrap_or(&addr);
            if let Some(response) = self.handle_message(message, effective_addr).await {
                writer.write_all(&response.to_bytes()).await?;
            }
        }

        Ok(())
    }

    /// Compute authentication response using SHA256(challenge + keyfile)
    fn compute_auth_response(challenge: &str, keyfile: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        challenge.hash(&mut hasher);
        keyfile.hash(&mut hasher);
        format!("{:x}", hasher.finish())
    }

    /// Send full database sync to a new node
    async fn send_full_sync<W: tokio::io::AsyncWrite + Unpin>(
        &self,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        tracing::debug!("╔════════════════════════════════════════════════════════════╗");
        tracing::debug!("║              SENDING FULL SYNC                             ║");
        tracing::debug!("╚════════════════════════════════════════════════════════════╝");

        // Count totals first
        let databases = self.storage.list_databases();
        let mut total_collections = 0;
        let mut total_documents = 0;

        for db_name in &databases {
            if let Ok(db) = self.storage.get_database(db_name) {
                let collections = db.list_collections();
                total_collections += collections.len();
                for coll_name in &collections {
                    if let Ok(coll) = db.get_collection(coll_name) {
                        total_documents += coll.count();
                    }
                }
            }
        }

        // Send start message
        let start_msg = ReplicationMessage::FullSyncStart {
            from_node: self.config.node_id.clone(),
            total_databases: databases.len(),
            total_collections,
            total_documents,
            current_sequence: self.replication_log.current_sequence(),
        };
        writer.write_all(&start_msg.to_bytes()).await?;

        tracing::debug!(
            "[FULL-SYNC] Sending {} databases, {} collections, {} documents",
            databases.len(),
            total_collections,
            total_documents
        );

        let mut docs_sent = 0;

        // Send each database
        for db_name in &databases {
            let db_msg = ReplicationMessage::FullSyncDatabase {
                name: db_name.clone(),
            };
            writer.write_all(&db_msg.to_bytes()).await?;
            tracing::debug!("[FULL-SYNC] Sending database: {}", db_name);

            if let Ok(db) = self.storage.get_database(db_name) {
                let collections = db.list_collections();

                for coll_name in &collections {
                    // Send collection definition
                    let coll_msg = ReplicationMessage::FullSyncCollection {
                        database: db_name.clone(),
                        name: coll_name.clone(),
                    };
                    writer.write_all(&coll_msg.to_bytes()).await?;
                    tracing::debug!("[FULL-SYNC] Sending collection: {}/{}", db_name, coll_name);

                    // Send documents in batches
                    if let Ok(coll) = db.get_collection(coll_name) {
                        let all_docs = coll.all();
                        let batch_size = 100;

                        for chunk in all_docs.chunks(batch_size) {
                            let doc_values: Vec<Value> =
                                chunk.iter().map(|d| d.to_value()).collect();

                            let docs_msg = ReplicationMessage::FullSyncDocuments {
                                database: db_name.clone(),
                                collection: coll_name.clone(),
                                documents: doc_values,
                            };
                            writer.write_all(&docs_msg.to_bytes()).await?;

                            docs_sent += chunk.len();

                            // Send progress update every 1000 docs
                            if docs_sent % 1000 == 0 {
                                let progress_msg = ReplicationMessage::FullSyncProgress {
                                    from_node: self.config.node_id.clone(),
                                    phase: "documents".to_string(),
                                    current: docs_sent,
                                    total: total_documents,
                                };
                                writer.write_all(&progress_msg.to_bytes()).await?;
                                tracing::debug!(
                                    "[FULL-SYNC] Progress: {}/{} documents",
                                    docs_sent,
                                    total_documents
                                );
                            }
                        }
                    }
                }
            }
        }

        // Send completion message
        let complete_msg = ReplicationMessage::FullSyncComplete {
            from_node: self.config.node_id.clone(),
            current_sequence: self.replication_log.current_sequence(),
        };
        writer.write_all(&complete_msg.to_bytes()).await?;

        tracing::debug!("╔════════════════════════════════════════════════════════════╗");
        tracing::debug!("║              FULL SYNC COMPLETE                            ║");
        tracing::debug!("╠════════════════════════════════════════════════════════════╣");
        tracing::debug!("║ Documents sent: {:<42} ║", docs_sent);
        tracing::debug!("╚════════════════════════════════════════════════════════════╝");

        Ok(())
    }

    /// Handle a replication message
    async fn handle_message(
        &self,
        message: ReplicationMessage,
        from_addr: &str,
    ) -> Option<ReplicationMessage> {
        match message {
            ReplicationMessage::Ping {
                from_node,
                replication_addr,
            } => {
                tracing::debug!("[PING] Received from {} ({})", from_node, from_addr);

                // If they provided their replication port, construct their actual address
                // using the IP from the incoming connection
                if let Some(addr) = replication_addr {
                    // Extract port from their advertised address (format: "0.0.0.0:port")
                    if let Some(port) = addr.split(':').last() {
                        // Extract IP from the incoming connection address
                        if let Some(ip) = from_addr.split(':').next() {
                            let peer_repl_addr = format!("{}:{}", ip, port);
                            // Register with their actual replication address, not the ephemeral port
                            self.register_peer(&peer_repl_addr, &from_node);
                            // Try to connect back to their replication port
                            self.try_connect_to_peer(peer_repl_addr);
                        }
                    }
                }

                // Return list of known peers for discovery (only replication addresses, not ephemeral)
                let known_peers: Vec<String> = self
                    .peer_states
                    .read()
                    .unwrap()
                    .values()
                    .filter(|p| p.is_connected)
                    .map(|p| p.address.clone())
                    .collect();

                Some(ReplicationMessage::Pong {
                    from_node: self.config.node_id.clone(),
                    current_sequence: self.replication_log.current_sequence(),
                    known_peers,
                })
            }

            ReplicationMessage::Pong {
                from_node,
                current_sequence,
                known_peers,
            } => {
                tracing::debug!(
                    "[PONG] From {} - their sequence: {}, our sequence: {}, known peers: {:?}",
                    from_node,
                    current_sequence,
                    self.replication_log.current_sequence(),
                    known_peers
                );

                // Update node_id for this peer (from_addr is the address we're connected to)
                {
                    let mut peers = self.peer_states.write().unwrap();
                    if let Some(state) = peers.get_mut(from_addr) {
                        if state.node_id.is_none() {
                            tracing::debug!(
                                "[PONG] Learning node_id {} for peer {}",
                                from_node,
                                from_addr
                            );
                            state.node_id = Some(from_node.clone());
                        }
                    }
                }

                // Try to connect to any newly discovered peers
                for peer_addr in known_peers {
                    tracing::debug!(
                        "[DISCOVERY] Received peer {} from {}, attempting connection",
                        peer_addr,
                        from_node
                    );
                    self.try_connect_to_peer(peer_addr);
                }

                None
            }

            ReplicationMessage::LeaveNotification { from_node } => {
                tracing::debug!("╔════════════════════════════════════════════════════════════╗");
                tracing::debug!("║                    NODE LEAVING                            ║");
                tracing::debug!("╠════════════════════════════════════════════════════════════╣");
                tracing::debug!("║ Node: {:<52} ║", from_node);
                tracing::debug!("╚════════════════════════════════════════════════════════════╝");
                None
            }

            ReplicationMessage::SyncRequest {
                from_node,
                after_sequence,
            } => {
                // Note: Peer registration is handled in Ping handler with proper replication address
                let _ = from_node; // Used for logging

                let entries = self.replication_log.get_entries_after(after_sequence);
                let current_seq = self.replication_log.current_sequence();

                tracing::debug!("[SYNC-REQ] From {} requesting entries after seq {}. Sending {} entries (our seq: {})",
                    from_node, after_sequence, entries.len(), current_seq);

                Some(ReplicationMessage::SyncResponse {
                    from_node: self.config.node_id.clone(),
                    entries,
                    current_sequence: current_seq,
                })
            }

            ReplicationMessage::SyncResponse {
                from_node,
                entries,
                current_sequence,
            } => {
                if entries.is_empty() {
                    tracing::debug!(
                        "[SYNC-RESP] From {} - no new entries (their seq: {}, our seq: {})",
                        from_node,
                        current_sequence,
                        self.replication_log.current_sequence()
                    );
                } else {
                    tracing::debug!(
                        "╔════════════════════════════════════════════════════════════╗"
                    );
                    tracing::debug!(
                        "║                  SYNC DATA RECEIVED                        ║"
                    );
                    tracing::debug!(
                        "╠════════════════════════════════════════════════════════════╣"
                    );
                    tracing::debug!("║ From: {:<52} ║", from_node);
                    tracing::debug!("║ Entries received: {:<40} ║", entries.len());
                    tracing::debug!("║ Their sequence: {:<42} ║", current_sequence);
                    tracing::debug!(
                        "║ Our sequence before: {:<37} ║",
                        self.replication_log.current_sequence()
                    );

                    for entry in &entries {
                        tracing::debug!(
                            "║   {:?} {}/{} key={:<30} ║",
                            entry.operation,
                            entry.database,
                            entry.collection,
                            entry.document_key
                        );
                    }
                    tracing::debug!(
                        "╚════════════════════════════════════════════════════════════╝"
                    );
                }

                self.apply_entries(&entries).await;

                if let Some(last) = entries.last() {
                    // Update our tracking of what we've received FROM this peer (their sequence numbers)
                    self.update_peer_received(from_addr, last.sequence);

                    Some(ReplicationMessage::Ack {
                        from_node: self.config.node_id.clone(),
                        up_to_sequence: last.sequence,
                    })
                } else {
                    None
                }
            }

            ReplicationMessage::PushEntries { from_node, entries } => {
                // Note: Peer registration is handled in Ping handler with proper replication address
                let _ = &from_node; // Used for logging

                if !entries.is_empty() {
                    tracing::debug!(
                        "╔════════════════════════════════════════════════════════════╗"
                    );
                    tracing::debug!(
                        "║                  PUSH DATA RECEIVED                        ║"
                    );
                    tracing::debug!(
                        "╠════════════════════════════════════════════════════════════╣"
                    );
                    tracing::debug!("║ From: {:<52} ║", from_node);
                    tracing::debug!("║ Entries pushed: {:<42} ║", entries.len());

                    for entry in &entries {
                        tracing::debug!(
                            "║   {:?} {}/{} key={:<30} ║",
                            entry.operation,
                            entry.database,
                            entry.collection,
                            entry.document_key
                        );
                    }
                    tracing::debug!(
                        "╚════════════════════════════════════════════════════════════╝"
                    );
                }

                self.apply_entries(&entries).await;

                if let Some(last) = entries.last() {
                    // Update our tracking of what we've received FROM this peer (their sequence numbers)
                    self.update_peer_received(from_addr, last.sequence);

                    tracing::debug!(
                        "[ACK] Sending Ack to {} for sequence {}",
                        from_addr,
                        last.sequence
                    );
                    Some(ReplicationMessage::Ack {
                        from_node: self.config.node_id.clone(),
                        up_to_sequence: last.sequence,
                    })
                } else {
                    None
                }
            }

            ReplicationMessage::Ack {
                from_node,
                up_to_sequence,
            } => {
                tracing::debug!(
                    "[ACK] Received from {} (addr: {}) - they acked up to sequence {}",
                    from_node,
                    from_addr,
                    up_to_sequence
                );
                // Use from_addr (the peer's address) rather than from_node (the node ID)
                // because peer_states is keyed by address
                self.update_peer_acked(from_addr, up_to_sequence);
                None
            }

            // Full sync messages are handled in receive_full_sync
            ReplicationMessage::FullSyncStart {
                from_node,
                total_databases,
                total_collections,
                total_documents,
                current_sequence,
            } => {
                tracing::debug!("╔════════════════════════════════════════════════════════════╗");
                tracing::debug!("║              FULL SYNC STARTING                            ║");
                tracing::debug!("╠════════════════════════════════════════════════════════════╣");
                tracing::debug!("║ From: {:<52} ║", from_node);
                tracing::debug!("║ Databases: {:<47} ║", total_databases);
                tracing::debug!("║ Collections: {:<45} ║", total_collections);
                tracing::debug!("║ Documents: {:<47} ║", total_documents);
                tracing::debug!("║ Sequence: {:<48} ║", current_sequence);
                tracing::debug!("╚════════════════════════════════════════════════════════════╝");
                None
            }

            ReplicationMessage::FullSyncDatabase { name } => {
                tracing::debug!("[FULL-SYNC] Creating database: {}", name);
                if let Err(e) = self.storage.create_database(name.clone()) {
                    // Ignore if already exists
                    tracing::debug!("[FULL-SYNC] Database creation: {:?}", e);
                }
                None
            }

            ReplicationMessage::FullSyncCollection { database, name } => {
                tracing::debug!("[FULL-SYNC] Creating collection: {}/{}", database, name);
                if let Ok(db) = self.storage.get_database(&database) {
                    if let Err(e) = db.create_collection(name.clone()) {
                        tracing::debug!("[FULL-SYNC] Collection creation: {:?}", e);
                    }
                }
                None
            }

            ReplicationMessage::FullSyncDocuments {
                database,
                collection,
                documents,
            } => {
                tracing::debug!(
                    "[FULL-SYNC] Receiving {} documents for {}/{}",
                    documents.len(),
                    database,
                    collection
                );

                if let Ok(db) = self.storage.get_database(&database) {
                    if let Ok(coll) = db.get_collection(&collection) {
                        for doc_value in documents {
                            // Extract key from document
                            if let Some(key) = doc_value.get("_key").and_then(|k| k.as_str()) {
                                // Check if document already exists
                                if coll.get(key).is_ok() {
                                    // Update existing
                                    if let Err(e) = coll.update(key, doc_value.clone()) {
                                        tracing::debug!("[FULL-SYNC] Update error: {:?}", e);
                                    }
                                } else {
                                    // Insert new
                                    if let Err(e) = coll.insert(doc_value.clone()) {
                                        tracing::debug!("[FULL-SYNC] Insert error: {:?}", e);
                                    }
                                }
                            }
                        }
                    }
                }
                None
            }

            ReplicationMessage::FullSyncProgress {
                from_node: _,
                phase,
                current,
                total,
            } => {
                tracing::debug!("[FULL-SYNC] Progress: {} {}/{}", phase, current, total);
                None
            }

            ReplicationMessage::FullSyncComplete {
                from_node,
                current_sequence,
            } => {
                tracing::debug!("╔════════════════════════════════════════════════════════════╗");
                tracing::debug!("║              FULL SYNC COMPLETE                            ║");
                tracing::debug!("╠════════════════════════════════════════════════════════════╣");
                tracing::debug!("║ From: {:<52} ║", from_node);
                tracing::debug!("║ Their sequence: {:<42} ║", current_sequence);
                tracing::debug!("╚════════════════════════════════════════════════════════════╝");
                None
            }

            // FullSyncRequest is handled separately in handle_connection
            ReplicationMessage::FullSyncRequest { .. } => None,

            // Auth messages are handled during connection handshake, not in main message loop
            ReplicationMessage::AuthChallenge { .. } => {
                tracing::debug!("[AUTH] Unexpected AuthChallenge in message loop");
                None
            }
            ReplicationMessage::AuthResponse { .. } => {
                tracing::debug!("[AUTH] Unexpected AuthResponse in message loop");
                None
            }
            ReplicationMessage::AuthResult { .. } => {
                tracing::debug!("[AUTH] Unexpected AuthResult in message loop");
                None
            }
        }
    }

    /// Sync loop for a single peer
    async fn peer_sync_loop(&self, peer_addr: String) {
        let mut retry_delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(30);
        let mut consecutive_failures = 0;
        let max_failures_for_discovered = 5; // Remove discovered peers after 5 failures
        let max_failures_for_configured = 100; // Keep trying configured peers much longer

        // Check if this is a configured peer (from --peers)
        let is_configured = self.config.peers.contains(&peer_addr);
        let max_failures = if is_configured {
            max_failures_for_configured
        } else {
            max_failures_for_discovered
        };

        loop {
            consecutive_failures += 1;

            if consecutive_failures == 1 || consecutive_failures % 10 == 0 {
                tracing::debug!(
                    "[PEER] Connecting to {} (attempt {})",
                    peer_addr,
                    consecutive_failures
                );
            }

            match TcpStream::connect(&peer_addr).await {
                Ok(socket) => {
                    tracing::debug!(
                        "╔════════════════════════════════════════════════════════════╗"
                    );
                    tracing::debug!(
                        "║                  PEER CONNECTED                            ║"
                    );
                    tracing::debug!(
                        "╠════════════════════════════════════════════════════════════╣"
                    );
                    tracing::debug!("║ Peer: {:<52} ║", peer_addr);
                    tracing::debug!(
                        "║ Our sequence: {:<44} ║",
                        self.replication_log.current_sequence()
                    );
                    tracing::debug!(
                        "╚════════════════════════════════════════════════════════════╝"
                    );

                    retry_delay = Duration::from_secs(1);
                    consecutive_failures = 0; // Reset on successful connection
                    self.set_peer_connected(&peer_addr, true);
                    tracing::debug!("[PEER] Connected to {}, starting sync", peer_addr);

                    if let Err(e) = self.sync_with_peer(socket, &peer_addr).await {
                        tracing::warn!("[PEER] Sync error with {}: {}", peer_addr, e);
                    }

                    tracing::debug!("[PEER] Disconnected from {}", peer_addr);
                    self.set_peer_connected(&peer_addr, false);
                }
                Err(e) => {
                    if consecutive_failures == 1 || consecutive_failures % 10 == 0 {
                        tracing::warn!(
                            "[PEER] Failed to connect to {}: {} (attempt {}/{})",
                            peer_addr,
                            e,
                            consecutive_failures,
                            max_failures
                        );
                    }

                    // Remove peer after too many failures
                    if consecutive_failures >= max_failures {
                        tracing::warn!(
                            "[PEER] Removing unreachable peer {} after {} failed attempts",
                            peer_addr,
                            consecutive_failures
                        );
                        self.remove_peer(&peer_addr);
                        return; // Exit the loop
                    }
                }
            }

            tokio::time::sleep(retry_delay).await;
            retry_delay = (retry_delay * 2).min(max_delay);
        }
    }

    /// Sync with a connected peer
    async fn sync_with_peer(&self, socket: TcpStream, peer_addr: &str) -> anyhow::Result<()> {
        tracing::debug!("[SYNC] Starting sync_with_peer for {}", peer_addr);
        let (reader, mut writer) = socket.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        // Handle authentication if the peer sends a challenge
        // First, try to read with a short timeout to see if there's an auth challenge
        let auth_result = tokio::time::timeout(
            Duration::from_millis(500),
            reader.read_line(&mut line)
        ).await;

        if let Ok(Ok(bytes_read)) = auth_result {
            if bytes_read > 0 {
                // Check if this is an auth challenge
                if let Ok(ReplicationMessage::AuthChallenge { challenge }) = serde_json::from_str(&line) {
                    tracing::debug!("[AUTH] Received challenge from {}", peer_addr);
                    
                    // We need a keyfile to respond
                    if let Some(ref keyfile) = self.config.keyfile {
                        let response = Self::compute_auth_response(&challenge, keyfile);
                        let auth_response = ReplicationMessage::AuthResponse { response };
                        writer.write_all(&auth_response.to_bytes()).await?;
                        
                        // Wait for auth result
                        line.clear();
                        let bytes_read = tokio::time::timeout(
                            Duration::from_secs(10),
                            reader.read_line(&mut line)
                        ).await??;
                        
                        if bytes_read == 0 {
                            anyhow::bail!("Connection closed during authentication");
                        }
                        
                        if let Ok(ReplicationMessage::AuthResult { success, message }) = serde_json::from_str(&line) {
                            if success {
                                tracing::debug!("[AUTH] Authentication successful with {}", peer_addr);
                            } else {
                                tracing::error!("[AUTH] Authentication failed with {}: {}", peer_addr, message);
                                anyhow::bail!("Authentication failed: {}", message);
                            }
                        } else {
                            anyhow::bail!("Unexpected response during authentication");
                        }
                    } else {
                        tracing::error!("[AUTH] Peer {} requires authentication but no keyfile configured", peer_addr);
                        anyhow::bail!("Peer requires authentication but no keyfile configured");
                    }
                    line.clear();
                } else {
                    // Not an auth challenge, it's some other message - we'll need to handle it in the main loop
                    // For now, just clear and continue - the peer doesn't require auth
                    line.clear();
                }
            }
        }
        // Timeout means no auth challenge was sent - peer doesn't require auth

        // Check if we need a full sync (sequence is 0 and no databases except _system)
        let our_sequence = self.replication_log.current_sequence();
        let databases = self.storage.list_databases();
        let need_full_sync = our_sequence == 0 && databases.len() <= 1; // Only _system or empty

        if need_full_sync {
            tracing::debug!("╔════════════════════════════════════════════════════════════╗");
            tracing::debug!("║          REQUESTING FULL SYNC (NEW NODE)                   ║");
            tracing::debug!("╚════════════════════════════════════════════════════════════╝");

            let full_sync_request = ReplicationMessage::FullSyncRequest {
                from_node: self.config.node_id.clone(),
            };
            writer.write_all(&full_sync_request.to_bytes()).await?;

            // Receive full sync messages
            loop {
                line.clear();
                let bytes_read = reader.read_line(&mut line).await?;
                if bytes_read == 0 {
                    break;
                }

                if let Ok(message) = serde_json::from_str::<ReplicationMessage>(&line) {
                    let is_complete =
                        matches!(message, ReplicationMessage::FullSyncComplete { .. });
                    self.handle_message(message, peer_addr).await;

                    if is_complete {
                        tracing::debug!(
                            "[FULL-SYNC] Full sync completed, switching to incremental sync"
                        );
                        break;
                    }
                }
            }
        }

        // Now do incremental sync
        let last_sequence = self
            .peer_states
            .read()
            .unwrap()
            .get(peer_addr)
            .map(|p| p.last_sequence_received)
            .unwrap_or(0);

        // Send initial ping to announce ourselves and get peer list
        tracing::debug!(
            "[SYNC] Sending initial ping to {} to discover peers",
            peer_addr
        );
        let ping = ReplicationMessage::Ping {
            from_node: self.config.node_id.clone(),
            replication_addr: Some(self.config.replication_addr()),
        };
        writer.write_all(&ping.to_bytes()).await?;

        tracing::debug!(
            "[SYNC] Requesting entries from {} after sequence {}",
            peer_addr,
            last_sequence
        );
        let sync_request = ReplicationMessage::SyncRequest {
            from_node: self.config.node_id.clone(),
            after_sequence: last_sequence,
        };
        writer.write_all(&sync_request.to_bytes()).await?;

        // Sync loop
        let mut interval = tokio::time::interval(Duration::from_secs(5));

        loop {
            tokio::select! {
                result = reader.read_line(&mut line) => {
                    match result {
                        Ok(0) => {
                            tracing::debug!("[SYNC] Connection closed by {}", peer_addr);
                            break;
                        }
                        Ok(_) => {
                            if let Ok(message) = serde_json::from_str::<ReplicationMessage>(&line) {
                                if let Some(response) = self.handle_message(message, peer_addr).await {
                                    writer.write_all(&response.to_bytes()).await?;
                                }
                            }
                            line.clear();
                        }
                        Err(e) => {
                            tracing::error!("[SYNC] Read error from {}: {}", peer_addr, e);
                            break;
                        }
                    }
                }
                _ = interval.tick() => {
                    tracing::debug!("[SYNC] Interval tick for peer {}", peer_addr);
                    // Send heartbeat with our replication address for discovery
                    let ping = ReplicationMessage::Ping {
                        from_node: self.config.node_id.clone(),
                        replication_addr: Some(self.config.replication_addr()),
                    };
                    writer.write_all(&ping.to_bytes()).await?;

                    // Request new entries from peer (PULL)
                    // Use last_sequence_received (what we've received from them)
                    let last_received = self.peer_states.read().unwrap()
                        .get(peer_addr)
                        .map(|p| p.last_sequence_received)
                        .unwrap_or(0);
                    let sync_request = ReplicationMessage::SyncRequest {
                        from_node: self.config.node_id.clone(),
                        after_sequence: last_received,
                    };
                    writer.write_all(&sync_request.to_bytes()).await?;

                    // Push any new entries to peer (PUSH)
                    let last_sent = self.peer_states.read().unwrap()
                        .get(peer_addr)
                        .map(|p| p.last_sequence_sent)
                        .unwrap_or(0);

                    let our_seq = self.replication_log.current_sequence();
                    let new_entries = self.replication_log.get_entries_after(last_sent);
                    tracing::debug!("[SYNC] peer={}, our_seq={}, last_sent={}, entries_to_push={}",
                        peer_addr, our_seq, last_sent, new_entries.len());
                    if !new_entries.is_empty() {
                        tracing::debug!("[PUSH] Sending {} entries to {} (seq {} to {})",
                            new_entries.len(), peer_addr,
                            new_entries.first().map(|e| e.sequence).unwrap_or(0),
                            new_entries.last().map(|e| e.sequence).unwrap_or(0));

                        let push = ReplicationMessage::PushEntries {
                            from_node: self.config.node_id.clone(),
                            entries: new_entries.clone(),
                        };
                        writer.write_all(&push.to_bytes()).await?;

                        if let Some(last) = new_entries.last() {
                            self.update_peer_sent(peer_addr, last.sequence);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Apply received entries using Last-Write-Wins conflict resolution
    async fn apply_entries(&self, entries: &[ReplicationEntry]) {
        for entry in entries {
            // Update our HLC based on received timestamp
            self.hlc_generator.receive(&entry.hlc);

            // Handle database-level operations first (don't need collection)
            match &entry.operation {
                Operation::CreateDatabase => {
                    if let Err(e) = self.storage.create_database(entry.database.clone()) {
                        tracing::debug!(
                            "[APPLY] Create database {} (may already exist): {}",
                            entry.database,
                            e
                        );
                    } else {
                        tracing::debug!("[APPLY] Created database {}", entry.database);
                    }
                    continue;
                }
                Operation::DeleteDatabase => {
                    match self.storage.delete_database(&entry.database) {
                        Ok(_) => tracing::debug!("[APPLY] Deleted database {}", entry.database),
                        Err(e) => tracing::debug!(
                            "[APPLY] Delete database {} skipped: {}",
                            entry.database,
                            e
                        ),
                    }
                    continue;
                }
                _ => {} // Other operations need database/collection
            }

            // Get or create the database
            let db = match self.storage.get_database(&entry.database) {
                Ok(db) => db,
                Err(_) => {
                    // Auto-create the database
                    tracing::debug!("[APPLY] Auto-creating database: {}", entry.database);
                    if let Err(e) = self.storage.create_database(entry.database.clone()) {
                        tracing::error!(
                            "[APPLY] Failed to create database {}: {}",
                            entry.database,
                            e
                        );
                        continue;
                    }
                    match self.storage.get_database(&entry.database) {
                        Ok(db) => db,
                        Err(e) => {
                            tracing::error!(
                                "[APPLY] Database {} still not found after creation: {}",
                                entry.database,
                                e
                            );
                            continue;
                        }
                    }
                }
            };

            // Handle collection-level operations (don't need the collection to exist for delete)
            match &entry.operation {
                Operation::CreateCollection => {
                    if let Err(e) = db.create_collection(entry.collection.clone()) {
                        tracing::debug!(
                            "[APPLY] Create collection {}/{} (may already exist): {}",
                            entry.database,
                            entry.collection,
                            e
                        );
                    } else {
                        tracing::debug!(
                            "[APPLY] Created collection {}/{}",
                            entry.database,
                            entry.collection
                        );
                    }
                    continue;
                }
                Operation::DeleteCollection => {
                    match db.delete_collection(&entry.collection) {
                        Ok(_) => tracing::debug!(
                            "[APPLY] Deleted collection {}/{}",
                            entry.database,
                            entry.collection
                        ),
                        Err(e) => tracing::debug!(
                            "[APPLY] Delete collection {}/{} skipped: {}",
                            entry.database,
                            entry.collection,
                            e
                        ),
                    }
                    continue;
                }
                _ => {} // Document operations need the collection
            }

            // Get or create the collection for document operations
            let collection = match db.get_collection(&entry.collection) {
                Ok(col) => col,
                Err(_) => {
                    // Auto-create the collection
                    tracing::debug!(
                        "[APPLY] Auto-creating collection: {}/{}",
                        entry.database,
                        entry.collection
                    );
                    if let Err(e) = db.create_collection(entry.collection.clone()) {
                        tracing::error!(
                            "[APPLY] Failed to create collection {}/{}: {}",
                            entry.database,
                            entry.collection,
                            e
                        );
                        continue;
                    }
                    match db.get_collection(&entry.collection) {
                        Ok(col) => col,
                        Err(e) => {
                            tracing::error!(
                                "[APPLY] Collection {}/{} still not found after creation: {}",
                                entry.database,
                                entry.collection,
                                e
                            );
                            continue;
                        }
                    }
                }
            };

            // Apply with LWW conflict resolution
            match &entry.operation {
                Operation::Insert | Operation::Update => {
                    if let Some(data) = &entry.document_data {
                        // Check if we already have a newer version
                        if let Ok(existing) = collection.get(&entry.document_key) {
                            let existing_value = existing.to_value();
                            if let Some(existing_updated) = existing_value.get("_updated_at") {
                                let our_time = existing_updated.as_str().unwrap_or("");
                                let their_time = chrono::DateTime::from_timestamp_millis(
                                    entry.hlc.physical_time as i64,
                                )
                                .map(|t| t.to_rfc3339())
                                .unwrap_or_default();

                                if our_time > their_time.as_str() {
                                    tracing::debug!(
                                        "[APPLY] Skipping older update for {}/{}",
                                        entry.collection,
                                        entry.document_key
                                    );
                                    continue;
                                }
                            }
                        }

                        // Parse and apply the document
                        if let Ok(mut doc_value) = serde_json::from_slice::<serde_json::Value>(data)
                        {
                            // Strip system fields to avoid duplication (they get regenerated on insert)
                            if let Some(obj) = doc_value.as_object_mut() {
                                obj.remove("_key");
                                obj.remove("_id");
                                obj.remove("_rev");
                                obj.remove("_created_at");
                                obj.remove("_updated_at");
                                // Re-add only _key which is needed for insert
                                obj.insert(
                                    "_key".to_string(),
                                    serde_json::Value::String(entry.document_key.clone()),
                                );
                            }

                            let result = match collection.get(&entry.document_key) {
                                Ok(_) => collection.update(&entry.document_key, doc_value),
                                Err(_) => collection.insert(doc_value),
                            };

                            match result {
                                Ok(doc) => {
                                    tracing::debug!(
                                        "[APPLY] {} {}/{}/{}",
                                        if entry.operation == Operation::Insert {
                                            "Inserted"
                                        } else {
                                            "Updated"
                                        },
                                        entry.database,
                                        entry.collection,
                                        doc.key
                                    );
                                }
                                Err(e) => {
                                    tracing::error!("[APPLY] Failed: {}", e);
                                }
                            }
                        }
                    }
                }
                Operation::Delete => match collection.delete(&entry.document_key) {
                    Ok(_) => {
                        tracing::debug!(
                            "[APPLY] Deleted {}/{}/{}",
                            entry.database,
                            entry.collection,
                            entry.document_key
                        );
                    }
                    Err(e) => {
                        tracing::debug!("[APPLY] Delete skipped (may not exist): {}", e);
                    }
                },
                Operation::TruncateCollection => match collection.truncate() {
                    Ok(count) => {
                        tracing::debug!(
                            "[APPLY] Truncated {}/{} - {} documents removed",
                            entry.database,
                            entry.collection,
                            count
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            "[APPLY] Truncate failed for {}/{}: {}",
                            entry.database,
                            entry.collection,
                            e
                        );
                    }
                },
                // These are handled earlier in the function
                Operation::CreateCollection
                | Operation::DeleteCollection
                | Operation::CreateDatabase
                | Operation::DeleteDatabase => {
                    unreachable!("Should have been handled earlier");
                }
            }
        }
    }

    /// Record a write operation in the replication log
    pub fn record_write(
        &self,
        database: &str,
        collection: &str,
        operation: Operation,
        document_key: &str,
        document_data: Option<&[u8]>,
        prev_rev: Option<&str>,
    ) -> u64 {
        let hlc = self.hlc_generator.now();

        let entry = ReplicationEntry::new(
            0, // Will be set by append
            self.config.node_id.clone(),
            hlc,
            database.to_string(),
            collection.to_string(),
            operation.clone(),
            document_key.to_string(),
            document_data.map(|d| d.to_vec()),
            prev_rev.map(|s| s.to_string()),
        );

        let seq = self.replication_log.append(entry);
        tracing::debug!(
            "[REPL-LOG] Recorded {:?} {}/{}/{} as seq {}",
            operation,
            database,
            collection,
            document_key,
            seq
        );
        seq
    }

    /// Get the replication log
    pub fn replication_log(&self) -> &PersistentReplicationLog {
        &self.replication_log
    }

    fn update_peer_acked(&self, node_id_or_addr: &str, sequence: u64) {
        let mut peers = self.peer_states.write().unwrap();

        // Log all peer addresses for debugging
        let peer_addrs: Vec<_> = peers.keys().cloned().collect();
        tracing::debug!(
            "[ACK] Looking for peer '{}' in peer_states: {:?}",
            node_id_or_addr,
            peer_addrs
        );

        // Try to find the peer by node_id first, then by address
        let found = peers.values_mut().find(|p| {
            p.node_id.as_deref() == Some(node_id_or_addr) || p.address == node_id_or_addr
        });

        if let Some(state) = found {
            if state.last_sequence_acked < sequence {
                tracing::debug!(
                    "[ACK] Updating peer {} acked sequence: {} -> {}",
                    state.address,
                    state.last_sequence_acked,
                    sequence
                );
                state.last_sequence_acked = sequence;
            }
        } else {
            tracing::warn!(
                "[ACK] ✗ Could not find peer with node_id or address: '{}'",
                node_id_or_addr
            );
        }
    }

    fn update_peer_sent(&self, peer: &str, sequence: u64) {
        if let Some(state) = self.peer_states.write().unwrap().get_mut(peer) {
            state.last_sequence_sent = sequence;
        }
    }

    /// Update the highest sequence we've received FROM this peer (for sync requests)
    fn update_peer_received(&self, peer_addr: &str, sequence: u64) {
        let mut peers = self.peer_states.write().unwrap();
        if let Some(state) = peers.get_mut(peer_addr) {
            if state.last_sequence_received < sequence {
                tracing::debug!(
                    "[RECV] Updating peer {} received sequence: {} -> {}",
                    state.address,
                    state.last_sequence_received,
                    sequence
                );
                state.last_sequence_received = sequence;
            }
        }
    }

    fn set_peer_connected(&self, peer: &str, connected: bool) {
        if let Some(state) = self.peer_states.write().unwrap().get_mut(peer) {
            state.is_connected = connected;
        }
    }

    /// Register or update a peer (used for incoming connections)
    fn register_peer(&self, address: &str, node_id: &str) {
        let is_new = {
            let mut peers = self.peer_states.write().unwrap();

            // Check if peer already exists by address or node_id
            let exists = peers.values().any(|p| p.address == address);

            if !exists {
                tracing::debug!(
                    "[PEER] Registering new incoming peer: {} ({})",
                    node_id,
                    address
                );
                peers.insert(
                    address.to_string(),
                    PeerState {
                        address: address.to_string(),
                        node_id: Some(node_id.to_string()),
                        last_seen: std::time::Instant::now(),
                        last_sequence_sent: 0,
                        last_sequence_acked: 0,
                        last_sequence_received: 0,
                        is_connected: true,
                    },
                );
                true
            } else {
                if let Some(state) = peers.get_mut(address) {
                    state.last_seen = std::time::Instant::now();
                    state.is_connected = true;
                    // Update node_id if we didn't know it before
                    if state.node_id.is_none() {
                        state.node_id = Some(node_id.to_string());
                    }
                }
                false
            }
        };

        // Save peers to _system._config if a new peer was added
        if is_new {
            self.save_peers();
        }
    }

    /// Remove a peer from the peer states
    fn remove_peer(&self, address: &str) {
        let removed = {
            let mut peers = self.peer_states.write().unwrap();
            peers.remove(address).is_some()
        };

        if removed {
            tracing::debug!("[PEER] Removed peer: {}", address);
            // Save updated peer list to _system._config
            self.save_peers();
        }
    }

    /// Try to connect to a discovered peer (non-blocking)
    fn try_connect_to_peer(&self, addr: String) {
        // Skip if it's our own address
        if addr == self.config.replication_addr()
            || addr.ends_with(&format!(":{}", self.config.replication_port))
        {
            tracing::debug!("[DISCOVERY] Skipping {} - it's our own address", addr);
            return;
        }

        // Skip if it's already a configured peer (we already have a sync loop for it)
        if self.config.peers.contains(&addr) {
            tracing::debug!("[DISCOVERY] Skipping {} - already a configured peer", addr);
            return;
        }

        // Check if we're already tracking this peer, and add it if not
        // This prevents spawning multiple sync loops for the same peer
        let should_spawn = {
            let mut peers = self.peer_states.write().unwrap();
            if peers.values().any(|p| p.address == addr) {
                tracing::debug!("[DISCOVERY] Skipping {} - already in peer list", addr);
                false
            } else {
                // Add peer immediately to prevent duplicate sync loops
                tracing::debug!(
                    "[DISCOVERY] Discovered new peer: {}, adding to peer list",
                    addr
                );
                peers.insert(
                    addr.clone(),
                    PeerState {
                        address: addr.clone(),
                        node_id: None,
                        last_seen: std::time::Instant::now(),
                        last_sequence_sent: 0,
                        last_sequence_acked: 0,
                        last_sequence_received: 0,
                        is_connected: false,
                    },
                );
                true
            }
        };

        if should_spawn {
            tracing::debug!("[DISCOVERY] Spawning sync loop for new peer: {}", addr);
            // Spawn a background task to connect
            let service = self.clone();
            tokio::spawn(async move {
                service.peer_sync_loop(addr).await;
            });
        }
    }

    /// Get cluster status information
    pub fn get_status(&self) -> ClusterStatus {
        let peers: Vec<PeerStatus> = self
            .peer_states
            .read()
            .unwrap()
            .values()
            .map(|p| PeerStatus {
                address: p.address.clone(),
                is_connected: p.is_connected,
                last_seen_secs_ago: p.last_seen.elapsed().as_secs(),
                replication_lag: self
                    .replication_log
                    .current_sequence()
                    .saturating_sub(p.last_sequence_acked),
            })
            .collect();

        ClusterStatus {
            node_id: self.config.node_id.clone(),
            is_cluster_mode: self.config.is_cluster_mode(),
            current_sequence: self.replication_log.current_sequence(),
            log_entries: self.replication_log.len(),
            peers,
        }
    }
}

impl Clone for ReplicationService {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            config: self.config.clone(),
            replication_log: self.replication_log.clone(),
            hlc_generator: Arc::clone(&self.hlc_generator),
            peer_states: Arc::clone(&self.peer_states),
            shutdown_tx: Arc::clone(&self.shutdown_tx),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterStatus {
    pub node_id: String,
    pub is_cluster_mode: bool,
    pub current_sequence: u64,
    pub log_entries: usize,
    pub peers: Vec<PeerStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerStatus {
    pub address: String,
    pub is_connected: bool,
    pub last_seen_secs_ago: u64,
    pub replication_lag: u64,
}
