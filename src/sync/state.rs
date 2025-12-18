//! Sync state management
//!
//! Tracks synchronization state including:
//! - Local sequence numbers
//! - Origin sequences for deduplication
//! - Peer tracking and health
//! - Persisted in _system._sync collection

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use crate::storage::StorageEngine;
use super::protocol::NodeStats;

/// Sync state persisted in _system database
pub struct SyncState {
    storage: Arc<StorageEngine>,
    node_id: String,
    
    /// Current local sequence number
    local_sequence: RwLock<u64>,
    
    /// Highest sequence received from each origin node (for deduplication)
    origin_sequences: RwLock<HashMap<String, u64>>,
    
    /// Last sequence we sent to each peer
    sent_sequences: RwLock<HashMap<String, u64>>,
    
    /// Last heartbeat time from each peer
    last_heartbeat: RwLock<HashMap<String, Instant>>,
    
    /// Stats from each node
    node_stats: RwLock<HashMap<String, NodeStats>>,
    
    /// Known peer addresses
    peers: RwLock<HashMap<String, PeerInfo>>,
}

/// Information about a peer node
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub node_id: String,
    pub sync_address: String,
    pub http_address: String,
    pub last_seen: Instant,
    pub is_connected: bool,
}

impl SyncState {
    const SYNC_COLLECTION: &'static str = "_sync";
    const STATE_KEY: &'static str = "sync_state";
    const PEERS_KEY: &'static str = "peers";
    
    /// Create new sync state, loading persisted data if available
    pub fn new(storage: Arc<StorageEngine>, node_id: String) -> Self {
        let state = Self {
            storage,
            node_id,
            local_sequence: RwLock::new(0),
            origin_sequences: RwLock::new(HashMap::new()),
            sent_sequences: RwLock::new(HashMap::new()),
            last_heartbeat: RwLock::new(HashMap::new()),
            node_stats: RwLock::new(HashMap::new()),
            peers: RwLock::new(HashMap::new()),
        };
        state.load();
        state
    }
    
    /// Load state from _system._sync
    fn load(&self) {
        // Try to get the sync collection
        let sync_coll = match self.storage.get_database("_system")
            .and_then(|db| db.get_collection(Self::SYNC_COLLECTION)) {
            Ok(c) => c,
            Err(_) => return, // Collection doesn't exist yet, that's fine
        };
        
        // Try to load persisted state
        if let Ok(doc) = sync_coll.get(Self::STATE_KEY) {
            if let Some(data) = doc.data.as_object() {
                if let Some(seq) = data.get("sequence").and_then(|v| v.as_u64()) {
                    *self.local_sequence.write().unwrap() = seq;
                }
                if let Some(origins) = data.get("origin_sequences").and_then(|v| v.as_object()) {
                    let mut map = self.origin_sequences.write().unwrap();
                    for (k, v) in origins {
                        if let Some(seq) = v.as_u64() {
                            map.insert(k.clone(), seq);
                        }
                    }
                }
            }
        }
        
        // Load peers
        if let Ok(doc) = sync_coll.get(Self::PEERS_KEY) {
            if let Some(peers) = doc.data.as_object() {
                let mut map = self.peers.write().unwrap();
                for (node_id, info) in peers {
                    if let Some(obj) = info.as_object() {
                        let sync_addr = obj.get("sync_address").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let http_addr = obj.get("http_address").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        map.insert(node_id.clone(), PeerInfo {
                            node_id: node_id.clone(),
                            sync_address: sync_addr,
                            http_address: http_addr,
                            last_seen: Instant::now(),
                            is_connected: false,
                        });
                    }
                }
            }
        }
    }
    
    /// Persist state to _system._sync
    pub fn persist(&self) {
        // Get or create the sync collection
        let sync_coll = match self.storage.get_database("_system") {
            Ok(db) => {
                match db.get_collection(Self::SYNC_COLLECTION) {
                    Ok(c) => c,
                    Err(_) => {
                        // Try to create it
                        let _ = db.create_collection(Self::SYNC_COLLECTION.to_string(), None);
                        match db.get_collection(Self::SYNC_COLLECTION) {
                            Ok(c) => c,
                            Err(_) => return,
                        }
                    }
                }
            }
            Err(_) => return,
        };
        
        let seq = *self.local_sequence.read().unwrap();
        let origins: HashMap<String, u64> = self.origin_sequences.read().unwrap().clone();
        
        let state_doc = serde_json::json!({
            "_key": Self::STATE_KEY,
            "sequence": seq,
            "origin_sequences": origins,
        });
        
        // Delete and insert to upsert
        let _ = sync_coll.delete(Self::STATE_KEY);
        let _ = sync_coll.insert(state_doc);
        
        // Persist peers
        let peers = self.peers.read().unwrap();
        let peers_doc = serde_json::json!({
            "_key": Self::PEERS_KEY,
            "peers": peers.iter().map(|(id, info)| {
                (id.clone(), serde_json::json!({
                    "sync_address": info.sync_address,
                    "http_address": info.http_address,
                }))
            }).collect::<HashMap<_, _>>(),
        });
        
        let _ = sync_coll.delete(Self::PEERS_KEY);
        let _ = sync_coll.insert(peers_doc);
    }
    
    /// Get the local node ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }
    
    /// Get and increment the next sequence number
    pub fn next_sequence(&self) -> u64 {
        let mut seq = self.local_sequence.write().unwrap();
        *seq += 1;
        *seq
    }
    
    /// Get current sequence without incrementing
    pub fn current_sequence(&self) -> u64 {
        *self.local_sequence.read().unwrap()
    }
    
    /// Update the sequence received from an origin node
    pub fn update_origin_sequence(&self, origin: &str, seq: u64) {
        let mut origins = self.origin_sequences.write().unwrap();
        let current = origins.get(origin).copied().unwrap_or(0);
        if seq > current {
            origins.insert(origin.to_string(), seq);
        }
    }
    
    /// Get highest sequence received from an origin
    pub fn get_origin_sequence(&self, origin: &str) -> u64 {
        self.origin_sequences.read().unwrap().get(origin).copied().unwrap_or(0)
    }
    
    /// Check if an entry is a duplicate (already applied)
    pub fn is_duplicate(&self, origin: &str, origin_seq: u64) -> bool {
        let origins = self.origin_sequences.read().unwrap();
        origins.get(origin).copied().unwrap_or(0) >= origin_seq
    }
    
    /// Update last sequence sent to a peer
    pub fn update_sent_sequence(&self, peer: &str, seq: u64) {
        self.sent_sequences.write().unwrap().insert(peer.to_string(), seq);
    }
    
    /// Get last sequence sent to a peer
    pub fn get_sent_sequence(&self, peer: &str) -> u64 {
        self.sent_sequences.read().unwrap().get(peer).copied().unwrap_or(0)
    }
    
    /// Record heartbeat from a peer
    pub fn update_heartbeat(&self, node_id: &str, stats: NodeStats) {
        self.last_heartbeat.write().unwrap().insert(node_id.to_string(), Instant::now());
        self.node_stats.write().unwrap().insert(node_id.to_string(), stats);
    }
    
    /// Get list of nodes that haven't sent heartbeat in timeout duration
    pub fn dead_nodes(&self, timeout: Duration) -> Vec<String> {
        let now = Instant::now();
        let heartbeats = self.last_heartbeat.read().unwrap();
        heartbeats
            .iter()
            .filter(|(_, last)| now.duration_since(**last) > timeout)
            .map(|(id, _)| id.clone())
            .collect()
    }
    
    /// Get list of all known peers
    pub fn get_peers(&self) -> Vec<PeerInfo> {
        self.peers.read().unwrap().values().cloned().collect()
    }
    
    /// Add or update a peer
    pub fn add_peer(&self, node_id: String, sync_address: String, http_address: String) {
        let mut peers = self.peers.write().unwrap();
        peers.insert(node_id.clone(), PeerInfo {
            node_id,
            sync_address,
            http_address,
            last_seen: Instant::now(),
            is_connected: false,
        });
    }
    
    /// Remove a peer
    pub fn remove_peer(&self, node_id: &str) {
        self.peers.write().unwrap().remove(node_id);
        self.last_heartbeat.write().unwrap().remove(node_id);
        self.node_stats.write().unwrap().remove(node_id);
        self.sent_sequences.write().unwrap().remove(node_id);
    }
    
    /// Mark peer as connected/disconnected
    pub fn set_peer_connected(&self, node_id: &str, connected: bool) {
        if let Some(peer) = self.peers.write().unwrap().get_mut(node_id) {
            peer.is_connected = connected;
            if connected {
                peer.last_seen = Instant::now();
            }
        }
    }
    
    /// Get stats for all nodes
    pub fn get_all_stats(&self) -> HashMap<String, NodeStats> {
        self.node_stats.read().unwrap().clone()
    }
}

impl Clone for SyncState {
    fn clone(&self) -> Self {
        Self {
            storage: self.storage.clone(),
            node_id: self.node_id.clone(),
            local_sequence: RwLock::new(*self.local_sequence.read().unwrap()),
            origin_sequences: RwLock::new(self.origin_sequences.read().unwrap().clone()),
            sent_sequences: RwLock::new(self.sent_sequences.read().unwrap().clone()),
            last_heartbeat: RwLock::new(self.last_heartbeat.read().unwrap().clone()),
            node_stats: RwLock::new(self.node_stats.read().unwrap().clone()),
            peers: RwLock::new(self.peers.read().unwrap().clone()),
        }
    }
}
