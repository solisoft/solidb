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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_new() {
        let node = Node::new(
            "node1".to_string(),
            "127.0.0.1:8000".to_string(),
            "127.0.0.1:9000".to_string(),
        );
        
        assert_eq!(node.id, "node1");
        assert_eq!(node.address, "127.0.0.1:8000");
        assert_eq!(node.api_address, "127.0.0.1:9000");
        assert!(node.started_at > 0);
    }

    #[test]
    fn test_node_started_at_is_recent() {
        let before = chrono::Utc::now().timestamp_millis() as u64;
        let node = Node::new("n".to_string(), "a".to_string(), "b".to_string());
        let after = chrono::Utc::now().timestamp_millis() as u64;
        
        assert!(node.started_at >= before);
        assert!(node.started_at <= after);
    }

    #[test]
    fn test_node_clone() {
        let node = Node::new("n1".to_string(), "addr".to_string(), "api".to_string());
        let cloned = node.clone();
        
        assert_eq!(node.id, cloned.id);
        assert_eq!(node.address, cloned.address);
        assert_eq!(node.api_address, cloned.api_address);
        assert_eq!(node.started_at, cloned.started_at);
    }

    #[test]
    fn test_node_equality() {
        let node1 = Node {
            id: "n1".to_string(),
            address: "addr1".to_string(),
            api_address: "api1".to_string(),
            started_at: 1000,
        };
        let node2 = Node {
            id: "n1".to_string(),
            address: "addr1".to_string(),
            api_address: "api1".to_string(),
            started_at: 1000,
        };
        let node3 = Node {
            id: "n2".to_string(),
            address: "addr1".to_string(),
            api_address: "api1".to_string(),
            started_at: 1000,
        };
        
        assert_eq!(node1, node2);
        assert_ne!(node1, node3);
    }

    #[test]
    fn test_node_serialization() {
        let node = Node::new("test".to_string(), "addr".to_string(), "api".to_string());
        
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("test"));
        assert!(json.contains("addr"));
        
        let deserialized: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(node, deserialized);
    }

    #[test]
    fn test_node_debug() {
        let node = Node::new("debug_test".to_string(), "a".to_string(), "b".to_string());
        let debug = format!("{:?}", node);
        assert!(debug.contains("debug_test"));
    }
}

