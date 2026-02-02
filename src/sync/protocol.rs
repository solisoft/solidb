//! Binary protocol for P2P master-master synchronization
//!
//! Uses bincode for efficient binary serialization over TCP.
//! Includes LZ4 compression for large batches.
//!
//! Extended for offline-first sync with version vectors and client sessions.

use crate::sync::version_vector::VersionVector;
use serde::{Deserialize, Serialize};

/// Type of operation in the replication log
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum Operation {
    Insert,
    Update,
    Delete,
    CreateCollection,
    DeleteCollection,
    TruncateCollection,
    CreateDatabase,
    DeleteDatabase,
    PutBlobChunk,
    DeleteBlob,
    // Columnar collection operations
    ColumnarInsert,
    ColumnarDelete,
    ColumnarCreateCollection,
    ColumnarDropCollection,
    ColumnarTruncate,
}

/// A single entry in the sync log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncEntry {
    /// Local sequence number on this node (legacy, for compatibility)
    pub sequence: u64,
    /// Node that originated this entry
    pub origin_node: String,
    /// Sequence on the origin node (legacy, for compatibility)
    pub origin_sequence: u64,
    /// HLC timestamp (physical time component)
    pub hlc_ts: u64,
    /// HLC counter component
    pub hlc_count: u32,
    /// Database name
    pub database: String,
    /// Collection name
    pub collection: String,
    /// Type of operation
    pub operation: Operation,
    /// Document key
    pub document_key: String,
    /// Document data (binary, not JSON)
    #[serde(with = "serde_bytes")]
    pub document_data: Option<Vec<u8>>,
    /// Shard ID for sharded collections
    pub shard_id: Option<u16>,

    // === New fields for offline-first sync ===
    /// Full version vector (replaces sequence numbers for conflict detection)
    pub version_vector: Option<VersionVector>,
    /// Parent version vectors (causal history)
    pub parent_vectors: Vec<VersionVector>,
    /// Is this a delta (patch) or full document?
    pub is_delta: bool,
    /// Delta patch data (if is_delta is true)
    #[serde(with = "serde_bytes")]
    pub delta_data: Option<Vec<u8>>,
    /// Client session ID (for client-initiated changes)
    pub session_id: Option<String>,
    /// Device ID that made the change
    pub device_id: Option<String>,
}

/// Shard configuration for a collection
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShardConfig {
    /// Number of shards
    pub num_shards: u16,
    /// Replication factor (how many copies)
    pub replication_factor: u16,
    /// Shard key field (default: "_key")
    pub shard_key: String,
}

impl ShardConfig {
    pub fn new(num_shards: u16, replication_factor: u16) -> Self {
        Self {
            num_shards,
            replication_factor,
            shard_key: "_key".to_string(),
        }
    }
}

/// Shard assignment for a single shard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardAssignment {
    pub shard_id: u16,
    pub owner: String,
    pub replicas: Vec<String>,
}

/// Node statistics for health monitoring
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeStats {
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub disk_used: u64,
    pub document_count: u64,
    pub collections_count: u32,
}

/// Messages exchanged between nodes over TCP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SyncMessage {
    // === Authentication ===
    /// Server sends challenge
    AuthChallenge { challenge: Vec<u8> },
    /// Client responds with HMAC
    AuthResponse { hmac: Vec<u8> },
    /// Server confirms auth result
    AuthResult { success: bool, message: String },

    // === Incremental Sync ===
    /// Request entries after a sequence
    IncrementalSyncRequest {
        from_node: String,
        after_sequence: u64,
        /// Max batch size in bytes (default 1MB)
        max_batch_bytes: u32,
    },

    // === Full Sync (for new nodes) ===
    /// Request full sync
    FullSyncRequest { from_node: String },
    /// Start of full sync
    FullSyncStart {
        total_databases: u32,
        total_collections: u32,
        total_documents: u64,
    },
    /// Database definition
    FullSyncDatabase { name: String },
    /// Collection definition
    FullSyncCollection {
        database: String,
        name: String,
        shard_config: Option<ShardConfig>,
    },
    /// Batch of documents (LZ4 compressed if large)
    FullSyncDocuments {
        database: String,
        collection: String,
        /// Raw bincode-encoded documents, possibly LZ4 compressed
        data: Vec<u8>,
        compressed: bool,
        doc_count: u32,
    },
    /// End of full sync
    FullSyncComplete { final_sequence: u64 },

    // === Batch Sync Response ===
    /// Batch of sync entries
    SyncBatch {
        entries: Vec<SyncEntry>,
        has_more: bool,
        current_sequence: u64,
        /// Compressed data (if large)
        compressed: bool,
    },

    // === Health & Heartbeat ===
    /// Periodic heartbeat
    Heartbeat {
        node_id: String,
        sequence: u64,
        stats: NodeStats,
    },
    /// Heartbeat acknowledgment
    HeartbeatAck { node_id: String },

    // === Node Management ===
    /// Node joining cluster
    NodeJoin {
        node_id: String,
        address: String,
        http_address: String,
    },
    /// Node leaving cluster (graceful)
    NodeLeave { node_id: String },
    /// Node detected as dead (after timeout)
    NodeDead { node_id: String },

    // === Shard Management ===
    /// Shard rebalance after node failure/join
    ShardRebalance {
        database: String,
        collection: String,
        assignments: Vec<ShardAssignment>,
    },

    // === Client Sync (Offline-First) ===
    /// Register a new client sync session
    ClientRegisterSession {
        device_id: String,
        api_key: String,
        /// Optional: filter query for partial sync
        filter_query: Option<String>,
        /// Collections to subscribe to
        subscriptions: Vec<String>,
    },
    /// Response to session registration
    ClientSessionRegistered {
        session_id: String,
        /// Server's current version vector
        server_vector: VersionVector,
        /// Server capabilities
        supports_delta_sync: bool,
        supports_crdt: bool,
    },
    /// Client pulling changes from server
    ClientPullRequest {
        session_id: String,
        /// Client's current version vector
        client_vector: VersionVector,
        /// Maximum number of changes to return
        limit: Option<usize>,
    },
    /// Server response with changes
    ClientPullResponse {
        /// Changes for client to apply
        changes: Vec<SyncEntry>,
        /// Server's version vector after these changes
        server_vector: VersionVector,
        /// Whether there are more changes
        has_more: bool,
        /// Conflicts detected (if any)
        conflicts: Vec<ConflictEntry>,
    },
    /// Client pushing changes to server
    ClientPushRequest {
        session_id: String,
        /// Changes from client
        changes: Vec<SyncEntry>,
        /// Client's vector before these changes
        client_vector: VersionVector,
    },
    /// Server response to push
    ClientPushResponse {
        /// Server's new version vector
        server_vector: VersionVector,
        /// Conflicts that need resolution
        conflicts: Vec<ConflictEntry>,
        /// Number of changes accepted
        accepted: usize,
        /// Number of changes rejected
        rejected: usize,
    },
    /// Acknowledge receipt of changes
    ClientSyncAck {
        session_id: String,
        /// Vector up to which client has applied changes
        applied_vector: VersionVector,
    },
    /// Real-time subscription request
    ClientSubscribe {
        session_id: String,
        collections: Vec<String>,
    },
    /// Unsubscribe from collections
    ClientUnsubscribe {
        session_id: String,
        collections: Vec<String>,
    },
    /// Server notifying client of new changes (push)
    ClientNotifyChanges {
        session_id: String,
        /// Brief notification, client should pull
        has_changes: bool,
        /// Collections with changes
        collections: Vec<String>,
    },
}

/// Entry describing a conflict for client resolution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictEntry {
    /// Document key
    pub document_key: String,
    /// Collection name
    pub collection: String,
    /// Local (server) version vector
    pub local_vector: VersionVector,
    /// Remote (client) version vector
    pub remote_vector: VersionVector,
    /// Server document data
    pub local_data: Option<Vec<u8>>,
    /// Client document data
    pub remote_data: Option<Vec<u8>>,
    /// Timestamp when conflict was detected
    pub detected_at: u64,
}

impl SyncMessage {
    /// Encode message to bincode bytes with length prefix
    pub fn encode(&self) -> Vec<u8> {
        let payload = bincode::serialize(self).expect("Failed to serialize SyncMessage");
        let len = payload.len() as u32;
        let mut result = Vec::with_capacity(4 + payload.len());
        result.extend_from_slice(&len.to_be_bytes());
        result.extend(payload);
        result
    }

    /// Decode message from bincode bytes (without length prefix)
    pub fn decode(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }
}

impl SyncEntry {
    /// Create a new sync entry for a document operation
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sequence: u64,
        origin_node: String,
        origin_sequence: u64,
        hlc_ts: u64,
        hlc_count: u32,
        database: String,
        collection: String,
        operation: Operation,
        document_key: String,
        document_data: Option<Vec<u8>>,
        shard_id: Option<u16>,
    ) -> Self {
        Self {
            sequence,
            origin_node,
            origin_sequence,
            hlc_ts,
            hlc_count,
            database,
            collection,
            operation,
            document_key,
            document_data,
            shard_id,
            // New fields for offline sync
            version_vector: None,
            parent_vectors: Vec::new(),
            is_delta: false,
            delta_data: None,
            session_id: None,
            device_id: None,
        }
    }

    /// Create a new sync entry with version vector support
    pub fn with_version_vector(
        origin_node: String,
        database: String,
        collection: String,
        operation: Operation,
        document_key: String,
        document_data: Option<Vec<u8>>,
        version_vector: VersionVector,
    ) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            sequence: 0,
            origin_node: origin_node.clone(),
            origin_sequence: 0,
            hlc_ts: now,
            hlc_count: 0,
            database,
            collection,
            operation,
            document_key,
            document_data,
            shard_id: None,
            version_vector: Some(version_vector),
            parent_vectors: Vec::new(),
            is_delta: false,
            delta_data: None,
            session_id: None,
            device_id: Some(origin_node),
        }
    }

    /// Set the version vector
    pub fn set_version_vector(&mut self, vector: VersionVector) {
        self.version_vector = Some(vector);
    }

    /// Add a parent vector (causal history)
    pub fn add_parent_vector(&mut self, vector: VersionVector) {
        self.parent_vectors.push(vector);
    }

    /// Mark this entry as a delta (patch)
    pub fn set_delta(&mut self, patch_data: Vec<u8>) {
        self.is_delta = true;
        self.delta_data = Some(patch_data);
    }

    /// Get the effective version vector (new style or legacy)
    pub fn effective_vector(&self) -> Option<VersionVector> {
        self.version_vector.clone()
    }
}

/// Compute shard ID for a document key
pub fn compute_shard_id(key: &str, num_shards: u16) -> u16 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    (hasher.finish() % num_shards as u64) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_message_encode_decode() {
        let msg = SyncMessage::Heartbeat {
            node_id: "node1".to_string(),
            sequence: 42,
            stats: NodeStats::default(),
        };

        let encoded = msg.encode();
        // Skip length prefix (4 bytes)
        let decoded = SyncMessage::decode(&encoded[4..]).unwrap();

        match decoded {
            SyncMessage::Heartbeat {
                node_id, sequence, ..
            } => {
                assert_eq!(node_id, "node1");
                assert_eq!(sequence, 42);
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_compute_shard_id() {
        let shard = compute_shard_id("doc123", 8);
        assert!(shard < 8);

        // Same key should give same shard
        assert_eq!(compute_shard_id("doc123", 8), shard);
    }
}
