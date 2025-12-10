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

    /// Optional keyfile content for authentication (read from path)
    #[serde(skip)]
    pub keyfile: Option<String>,
}

impl ClusterConfig {
    /// Create a new cluster configuration
    pub fn new(node_id: Option<String>, peers: Vec<String>, replication_port: u16, keyfile_path: Option<String>) -> Self {
        let node_id = node_id.unwrap_or_else(|| {
            // Generate a stable node ID based on hostname + random suffix
            let hostname = hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            format!("{}-{}", hostname, &Uuid::new_v4().to_string()[..8])
        });

        // Read keyfile content if path is provided
        let keyfile = keyfile_path.and_then(|path| {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    let trimmed = content.trim().to_string();
                    if trimmed.is_empty() {
                        tracing::warn!("Keyfile at {} is empty", path);
                        None
                    } else {
                        tracing::debug!("Loaded keyfile from {}", path);
                        Some(trimmed)
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to read keyfile at {}: {}", path, e);
                    None
                }
            }
        });

        // Trim peer addresses and strip protocol prefixes to handle copy-pasted URLs
        let peers = peers.into_iter().map(|p| {
            let mut s = p.trim().to_string();
            if s.starts_with("http://") {
                s = s[7..].to_string();
            } else if s.starts_with("https://") {
                s = s[8..].to_string();
            }
            s
        }).collect();

        Self {
            node_id,
            peers,
            replication_port,
            keyfile,
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

    /// Check if keyfile authentication is enabled
    pub fn requires_auth(&self) -> bool {
        self.keyfile.is_some()
    }
}

impl Default for ClusterConfig {
    fn default() -> Self {
        Self {
            node_id: Uuid::new_v4().to_string(),
            peers: Vec::new(),
            replication_port: 6746,
            keyfile: None,
        }
    }
}
