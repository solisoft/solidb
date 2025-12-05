//! Replication queue for storing failed operations to replay on node recovery

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use serde_json::Value;

/// A failed operation that needs to be replayed
#[derive(Debug, Clone)]
pub struct FailedOperation {
    pub db_name: String,
    pub collection: String,
    pub doc: Value,
    pub timestamp: Instant,
}

/// Queue of failed operations for offline nodes
#[derive(Clone, Default)]
pub struct ReplicationQueue {
    // Map of node_address -> List of failed operations
    queues: Arc<RwLock<HashMap<String, Vec<FailedOperation>>>>,
}

impl ReplicationQueue {
    /// Create a new replication queue
    pub fn new() -> Self {
        Self {
            queues: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Push a failed operation to the queue for a specific node
    pub fn push(&self, node_addr: &str, op: FailedOperation) {
        let mut queues = self.queues.write().unwrap();
        queues
            .entry(node_addr.to_string())
            .or_insert_with(Vec::new)
            .push(op);
    }

    /// Pop all operations for a node (to replay them)
    pub fn pop_all(&self, node_addr: &str) -> Vec<FailedOperation> {
        let mut queues = self.queues.write().unwrap();
        queues.remove(node_addr).unwrap_or_default()
    }

    /// Check if a node has pending operations
    pub fn has_pending(&self, node_addr: &str) -> bool {
        let queues = self.queues.read().unwrap();
        queues.get(node_addr).map(|q| !q.is_empty()).unwrap_or(false)
    }

    /// Get total number of pending operations across all nodes
    pub fn total_pending(&self) -> usize {
        let queues = self.queues.read().unwrap();
        queues.values().map(|q| q.len()).sum()
    }
}
