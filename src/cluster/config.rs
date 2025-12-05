use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Configuration for cluster mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterConfig {
    /// Unique identifier for this node
    pub node_id: String,

    /// List of peer node addresses (host:port)
    pub peers: Vec<String>,

    /// Port for replication traffic
    pub replication_port: u16,
}

impl ClusterConfig {
    /// Create a new cluster configuration
    pub fn new(node_id: Option<String>, peers: Vec<String>, replication_port: u16) -> Self {
        let node_id = node_id.unwrap_or_else(|| {
            // Generate a stable node ID based on hostname + random suffix
            let hostname = hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            format!("{}-{}", hostname, &Uuid::new_v4().to_string()[..8])
        });

        Self {
            node_id,
            peers,
            replication_port,
        }
    }

    /// Check if this node is running in cluster mode
    pub fn is_cluster_mode(&self) -> bool {
        !self.peers.is_empty()
    }

    /// Get the replication listen address
    pub fn replication_addr(&self) -> String {
        format!("0.0.0.0:{}", self.replication_port)
    }
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            node_id: Uuid::new_v4().to_string(),
            peers: Vec::new(),
            replication_port: 6746,
        }
    }
}
