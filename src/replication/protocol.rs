use serde::{Deserialize, Serialize};

/// Type of operation in the replication log
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
}

/// A single entry in the replication log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub sequence: u64,
    pub node_id: String,
    pub database: String,
    pub collection: String,
    pub operation: Operation,
    pub key: String,
    pub data: Option<Vec<u8>>,
    pub timestamp: u64,
    #[serde(default)]
    pub origin_sequence: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReplicationMessage {
    SyncRequest {
        from_node: String,
        after_sequence: u64,
    },
    SyncResponse {
        entries: Vec<LogEntry>,
        current_sequence: u64,
    },
}
