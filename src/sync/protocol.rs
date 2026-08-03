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

    // ---------------------------------------------------------------------
    // APPEND ONLY BELOW THIS LINE.
    //
    // `SyncMessage` goes over the wire as bincode (see `encode`), which
    // identifies enum variants by their *position*. Inserting a variant in the
    // middle renumbers every variant after it, so a node running the new build
    // would send `ColumnarInsert` and a node running the old build would decode
    // it as whatever now sits at that index — silent misinterpretation during a
    // rolling upgrade. Appending is safe: an old node hits an unknown index and
    // fails the decode loudly instead.
    // ---------------------------------------------------------------------
    /// Index definitions. Without these an index created on one node existed
    /// only on that node: peers ran unindexed scans, and a TTL index on one
    /// node meant documents expired there and lived forever everywhere else.
    /// Payload is a JSON `IndexSpec` (create) or `IndexRef` (drop).
    CreateIndex,
    DropIndex,
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
    /// Server sends challenge with timestamp to prevent replay attacks
    AuthChallenge {
        challenge: Vec<u8>,
        /// Unix timestamp when challenge was generated (milliseconds)
        timestamp: u64,
        /// Nonce to prevent replay (random bytes)
        nonce: Vec<u8>,
    },
    /// Client responds with HMAC (computes HMAC over challenge + timestamp + nonce)
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
        /// "document" / "edge" / "blob" / "timeseries".
        ///
        /// Full sync used to recreate every collection with `None`, so a blob
        /// collection came back as a document collection (making its blobs
        /// unreadable) and a timeseries collection lost the type its insert
        /// path keys off.
        ///
        /// Adding this field changes the bincode layout: bincode is not
        /// self-describing and `#[serde(default)]` cannot fill a field that is
        /// simply absent from the byte stream. Both ends of a full sync must
        /// therefore run the same build. That is acceptable because full sync
        /// is a manual `SyncCommand::RequestFullSync`, and a mixed-version run
        /// already produced a wrongly-typed collection — but it is a real
        /// constraint, not a compatible addition.
        collection_type: Option<String>,
    },
    /// Batch of documents (LZ4 compressed if large)
    FullSyncDocuments {
        database: String,
        collection: String,
        /// The batch, encoded by [`encode_documents`] and possibly LZ4
        /// compressed. Opaque here on purpose: only that pair of functions may
        /// decide what is inside.
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

/// Encodes a batch of documents for [`SyncMessage::FullSyncDocuments`].
///
/// # Why not bincode
///
/// It was bincode, and **every full sync failed at its first batch**. The error
/// is worth quoting because it names its own cause:
///
/// ```text
/// Full sync request failed: Decode error: Bincode does not support the
/// serde::Deserializer::deserialize_any method
/// ```
///
/// A document is a `serde_json::Value`, which is self-describing: deserialising
/// one means asking the format "what comes next?", and bincode cannot answer
/// because it writes no type information. Serialising worked, so the sender
/// reported success and the receiver could never decode a single batch. That
/// asymmetry is why the bug survived: the seed's own view showed the new member
/// as healthy while the member had nothing.
///
/// JSON for the payload, then. It is bulkier, which is what the LZ4 layer above
/// is for, and it is the format the documents are already in.
///
/// Both directions live in one place so the two ends cannot be changed apart —
/// which is exactly how they came to disagree.
pub fn encode_documents(batch: &[serde_json::Value]) -> Result<Vec<u8>, String> {
    serde_json::to_vec(batch).map_err(|e| format!("encoding {} document(s): {e}", batch.len()))
}

/// Decodes what [`encode_documents`] produced.
pub fn decode_documents(data: &[u8]) -> Result<Vec<serde_json::Value>, String> {
    serde_json::from_slice(data).map_err(|e| format!("decoding a document batch: {e}"))
}

impl SyncMessage {
    /// Encode a message in the frame the connection pool reads.
    ///
    /// `[compressed: 1 byte][length: 4 bytes BE][bincode]` — the same shape
    /// `ConnectionPool::send` writes, because the only reader of these bytes is
    /// `ConnectionPool::receive`.
    ///
    /// This used to emit `[length][bincode]` with no leading byte, one byte
    /// short of what the reader expects. Everything shifted: the reader took
    /// the first length byte as the compressed flag, and bincode was handed the
    /// length prefix as the start of a message — "invalid value: integer
    /// 33554432, expected variant index 0 <= i < 27", which is `2u32` big-endian
    /// read as a little-endian discriminant.
    ///
    /// It was never noticed because nothing had ever run a full sync: the only
    /// caller is the responder below, and the request that triggers it could
    /// not be sent — the sync worker's command sender was discarded at
    /// construction.
    ///
    /// The flag is always 0. Compression belongs to the pool, and the one
    /// message that carries bulk data (`FullSyncDocuments`) compresses its own
    /// payload already.
    pub fn encode(&self) -> Vec<u8> {
        let payload = bincode::serialize(self).expect("Failed to serialize SyncMessage");
        let len = payload.len() as u32;
        let mut result = Vec::with_capacity(5 + payload.len());
        result.push(0);
        result.extend_from_slice(&len.to_be_bytes());
        result.extend(payload);
        result
    }

    /// Decode message from bincode bytes (without length prefix)
    pub fn decode(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }

    /// Bytes of frame header [`encode`] writes before the payload.
    ///
    /// Named rather than left as a literal, because thirteen tests hard-coded
    /// `4` and went on hard-coding it after the header grew to five. They kept
    /// failing with the same "expected variant index 0 <= i < 27" error the
    /// header change was made to fix, which reads as a regression in the thing
    /// that was just repaired.
    pub const HEADER_LEN: usize = 5;

    /// Decodes a whole frame, header included.
    ///
    /// The counterpart to [`encode`], and the reason it exists: `encode` returns
    /// a *frame* while `decode` takes a *body*, so every caller had to slice off
    /// a header whose length only lives in `encode`. That asymmetry is what let a
    /// one-byte change break a suite of tests silently — they were slicing a
    /// constant nobody had told them about.
    pub fn decode_frame(frame: &[u8]) -> Result<Self, bincode::Error> {
        use bincode::ErrorKind;
        if frame.len() < Self::HEADER_LEN {
            return Err(Box::new(ErrorKind::Custom(format!(
                "frame is {} bytes, shorter than the {}-byte header",
                frame.len(),
                Self::HEADER_LEN
            ))));
        }
        let declared = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
        let body = &frame[Self::HEADER_LEN..];
        if body.len() != declared {
            // Checked rather than trusted: a body shorter than its own header
            // claims is a truncated read, and decoding it anyway produces the
            // same unhelpful discriminant error as a framing mistake.
            return Err(Box::new(ErrorKind::Custom(format!(
                "frame declares {declared} bytes of payload and carries {}",
                body.len()
            ))));
        }
        Self::decode(body)
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
        let decoded = SyncMessage::decode_frame(&encoded).unwrap();

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

    #[test]
    fn a_document_batch_survives_a_round_trip() {
        // The test that was missing. Encoding used to succeed and decoding used
        // to fail every single time, so nothing short of a round trip would have
        // caught it — and no round trip existed.
        let batch = vec![
            serde_json::json!({"_key": "a", "n": 1, "nested": {"deep": [1, 2, 3]}}),
            serde_json::json!({"_key": "b", "text": "héllo", "flag": true, "nil": null}),
            serde_json::json!({"_key": "c", "float": 1.5, "big": 9007199254740991i64}),
        ];
        let encoded = encode_documents(&batch).expect("encodes");
        let decoded = decode_documents(&encoded).expect("decodes");
        assert_eq!(decoded, batch);
    }

    #[test]
    fn an_empty_batch_round_trips_as_empty() {
        // Distinguishable from a failure that used to produce empty bytes.
        let encoded = encode_documents(&[]).expect("encodes");
        assert!(decode_documents(&encoded).expect("decodes").is_empty());
    }

    #[test]
    fn garbage_fails_to_decode_rather_than_yielding_no_documents() {
        // "Zero documents" and "could not read the batch" must not be the same
        // outcome: the first reports a successful sync of nothing.
        assert!(decode_documents(b"\x00\x01\x02not json").is_err());
    }

    #[test]
    fn the_system_collections_a_second_node_needs_survive_the_round_trip() {
        // The shape that actually mattered: `_admins` is what makes a joining
        // node authenticatable, and it never arrived because this batch could
        // not be decoded.
        let admins = vec![serde_json::json!({
            "_key": "admin",
            "username": "admin",
            "password_hash": "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA",
            "roles": ["admin"],
            "created_at": 1785000000
        })];
        let decoded = decode_documents(&encode_documents(&admins).unwrap()).unwrap();
        assert_eq!(decoded, admins);
        assert_eq!(decoded[0]["username"], "admin");
    }

    #[test]
    fn a_frame_shorter_than_its_own_header_is_refused() {
        // A truncated read. Decoding it anyway yields the same "expected variant
        // index" error a framing mistake does, which is how one gets mistaken
        // for the other.
        let error = SyncMessage::decode_frame(&[0, 0, 0])
            .unwrap_err()
            .to_string();
        assert!(error.contains("shorter than"), "{error}");
    }

    #[test]
    fn a_frame_that_lies_about_its_length_is_refused() {
        let mut frame = SyncMessage::FullSyncComplete { final_sequence: 7 }.encode();
        frame.push(0xff);
        let error = SyncMessage::decode_frame(&frame).unwrap_err().to_string();
        assert!(error.contains("declares"), "{error}");
    }

    #[test]
    fn the_header_length_matches_what_encode_writes() {
        // The constant and the writer must agree. They did not for one commit,
        // and thirteen tests carried the old value.
        let frame = SyncMessage::FullSyncComplete { final_sequence: 1 }.encode();
        let declared = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]) as usize;
        assert_eq!(frame.len(), SyncMessage::HEADER_LEN + declared);
    }
}
