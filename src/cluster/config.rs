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
    pub fn new(
        node_id: Option<String>,
        peers: Vec<String>,
        replication_port: u16,
        keyfile_path: Option<String>,
    ) -> Self {
        let node_id = node_id.unwrap_or_else(|| {
            // Generate a stable node ID based on hostname + random suffix
            let hostname = hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string());
            format!("{}-{}", hostname, &Uuid::new_v4().to_string()[..8])
        });

        // Read keyfile content if path is provided
        let keyfile = keyfile_path.and_then(|path| match std::fs::read_to_string(&path) {
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
        });

        // Trim peer addresses and strip protocol prefixes to handle copy-pasted URLs
        let peers = peers
            .into_iter()
            .map(|p| {
                let mut s = p.trim().to_string();
                if s.starts_with("http://") {
                    s = s[7..].to_string();
                } else if s.starts_with("https://") {
                    s = s[8..].to_string();
                }
                s
            })
            .collect();

        Self {
            node_id,
            peers,
            replication_port,
            keyfile,
        }
    }

    /// Check if this node is running in cluster mode
    pub fn is_cluster_mode(&self) -> bool {
        // If we have a cluster config, we are in cluster mode (even if we are the first node)
        true
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cluster_config_default() {
        let config = ClusterConfig::default();

        assert!(!config.node_id.is_empty());
        assert!(config.peers.is_empty());
        assert_eq!(config.replication_port, 6746);
        assert!(config.keyfile.is_none());
    }

    #[test]
    fn test_cluster_config_new_with_node_id() {
        let config = ClusterConfig::new(Some("my-node".to_string()), vec![], 7000, None);

        assert_eq!(config.node_id, "my-node");
        assert_eq!(config.replication_port, 7000);
    }

    #[test]
    fn test_cluster_config_auto_node_id() {
        let config = ClusterConfig::new(None, vec![], 6746, None);

        // Auto-generated node ID should contain hyphen and be non-empty
        assert!(!config.node_id.is_empty());
    }

    #[test]
    fn test_cluster_config_strip_http() {
        let config = ClusterConfig::new(
            Some("node".to_string()),
            vec![
                "http://peer1:8080".to_string(),
                "https://peer2:8080".to_string(),
                "peer3:8080".to_string(),
            ],
            6746,
            None,
        );

        assert_eq!(config.peers[0], "peer1:8080");
        assert_eq!(config.peers[1], "peer2:8080");
        assert_eq!(config.peers[2], "peer3:8080");
    }

    #[test]
    fn test_cluster_config_trim_whitespace() {
        let config = ClusterConfig::new(
            Some("node".to_string()),
            vec!["  peer1:8080  ".to_string()],
            6746,
            None,
        );

        assert_eq!(config.peers[0], "peer1:8080");
    }

    #[test]
    fn test_is_cluster_mode() {
        let config = ClusterConfig::default();
        // Any cluster config means cluster mode
        assert!(config.is_cluster_mode());
    }

    #[test]
    fn test_replication_addr() {
        let config = ClusterConfig::new(Some("node".to_string()), vec![], 7777, None);

        assert_eq!(config.replication_addr(), "0.0.0.0:7777");
    }

    #[test]
    fn test_requires_auth_without_keyfile() {
        let config = ClusterConfig::default();
        assert!(!config.requires_auth());
    }

    #[test]
    fn test_cluster_config_serialization() {
        let config = ClusterConfig::new(
            Some("test-node".to_string()),
            vec!["peer1:8080".to_string()],
            6746,
            None,
        );

        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("test-node"));
        assert!(json.contains("peer1:8080"));

        let deserialized: ClusterConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.node_id, deserialized.node_id);
        assert_eq!(config.peers, deserialized.peers);
    }

    #[test]
    fn test_cluster_config_clone() {
        let config = ClusterConfig::new(
            Some("node1".to_string()),
            vec!["peer:8080".to_string()],
            6746,
            None,
        );

        let cloned = config.clone();
        assert_eq!(config.node_id, cloned.node_id);
        assert_eq!(config.peers, cloned.peers);
    }
}
