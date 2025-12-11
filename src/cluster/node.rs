use serde::{Deserialize, Serialize};

/// Unique identifier for a node in the cluster
pub type NodeId = String;

/// Information about a node in the cluster
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Node {
    pub id: NodeId,
    pub address: String,
    pub api_address: String, // For public API
    pub started_at: u64,
}

impl Node {
    pub fn new(id: NodeId, address: String, api_address: String) -> Self {
        Self {
            id,
            address,
            api_address,
            started_at: chrono::Utc::now().timestamp_millis() as u64,
        }
    }
}
