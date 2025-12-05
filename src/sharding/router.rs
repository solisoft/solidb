//! Shard routing using consistent hashing on document keys

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Routes documents to shards based on key hash
#[derive(Debug, Clone)]
pub struct ShardRouter;

impl ShardRouter {
    /// Calculate which shard a document key belongs to
    /// Uses consistent hashing: hash(key) % num_shards
    pub fn route(key: &str, num_shards: u16) -> u16 {
        if num_shards == 0 {
            return 0;
        }
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() % num_shards as u64) as u16
    }

    /// Get the node address responsible for a shard
    /// Shards are distributed round-robin across nodes
    pub fn shard_to_node<'a>(shard_id: u16, nodes: &'a [String]) -> Option<&'a str> {
        if nodes.is_empty() {
            return None;
        }
        let node_idx = shard_id as usize % nodes.len();
        Some(&nodes[node_idx])
    }

    /// Get all nodes for a shard (primary + replicas)
    /// Replicas are assigned round-robin starting from the primary node
    /// 
    /// Example with 3 nodes and replication_factor=2:
    /// - Shard 0: [node0, node1]
    /// - Shard 1: [node1, node2]  
    /// - Shard 2: [node2, node0]
    pub fn shard_to_nodes<'a>(
        shard_id: u16,
        replication_factor: u16,
        nodes: &'a [String],
    ) -> Vec<&'a str> {
        if nodes.is_empty() {
            return vec![];
        }
        
        let num_nodes = nodes.len();
        let primary_idx = shard_id as usize % num_nodes;
        
        // Cap replication factor to number of nodes
        let actual_rf = (replication_factor as usize).min(num_nodes);
        
        (0..actual_rf)
            .map(|offset| {
                let node_idx = (primary_idx + offset) % num_nodes;
                nodes[node_idx].as_str()
            })
            .collect()
    }

    /// Check if this node holds a replica of the shard
    pub fn is_shard_replica(
        shard_id: u16,
        node_index: usize,
        replication_factor: u16,
        num_nodes: usize,
    ) -> bool {
        if num_nodes == 0 {
            return true; // Single node mode
        }
        
        let primary_idx = shard_id as usize % num_nodes;
        let actual_rf = (replication_factor as usize).min(num_nodes);
        
        for offset in 0..actual_rf {
            if (primary_idx + offset) % num_nodes == node_index {
                return true;
            }
        }
        false
    }

    /// Check if a given node owns a specific shard
    pub fn is_shard_local(shard_id: u16, node_index: usize, num_nodes: usize) -> bool {
        if num_nodes == 0 {
            return true; // Single node mode
        }
        shard_id as usize % num_nodes == node_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_consistency() {
        // Same key should always route to same shard
        let shard1 = ShardRouter::route("test-key", 3);
        let shard2 = ShardRouter::route("test-key", 3);
        assert_eq!(shard1, shard2);
    }

    #[test]
    fn test_route_range() {
        // Shard ID should be in valid range
        for i in 0..100 {
            let key = format!("key-{}", i);
            let shard = ShardRouter::route(&key, 5);
            assert!(shard < 5);
        }
    }

    #[test]
    fn test_route_distribution() {
        // Keys should distribute somewhat evenly
        let mut counts = [0u32; 3];
        for i in 0..3000 {
            let key = format!("document-{}", i);
            let shard = ShardRouter::route(&key, 3);
            counts[shard as usize] += 1;
        }
        // Each shard should have roughly 1000 docs (allow 30% variance)
        for count in counts {
            assert!(count > 700 && count < 1300, "Uneven distribution: {:?}", counts);
        }
    }

    #[test]
    fn test_shard_to_node() {
        let nodes = vec!["node1:6745".to_string(), "node2:6745".to_string(), "node3:6745".to_string()];
        
        assert_eq!(ShardRouter::shard_to_node(0, &nodes), Some("node1:6745"));
        assert_eq!(ShardRouter::shard_to_node(1, &nodes), Some("node2:6745"));
        assert_eq!(ShardRouter::shard_to_node(2, &nodes), Some("node3:6745"));
        assert_eq!(ShardRouter::shard_to_node(3, &nodes), Some("node1:6745")); // Wraps
    }

    #[test]
    fn test_is_shard_local() {
        // Node 0 in 3-node cluster owns shards 0, 3, 6...
        assert!(ShardRouter::is_shard_local(0, 0, 3));
        assert!(ShardRouter::is_shard_local(3, 0, 3));
        assert!(!ShardRouter::is_shard_local(1, 0, 3));
        assert!(!ShardRouter::is_shard_local(2, 0, 3));

        // Node 1 owns shards 1, 4, 7...
        assert!(ShardRouter::is_shard_local(1, 1, 3));
        assert!(ShardRouter::is_shard_local(4, 1, 3));
    }

    #[test]
    fn test_single_node_mode() {
        // Single node should always be local
        assert!(ShardRouter::is_shard_local(0, 0, 0));
        assert!(ShardRouter::is_shard_local(5, 0, 0));
    }

    #[test]
    fn test_shard_to_nodes_with_replicas() {
        let nodes = vec![
            "node0:6745".to_string(),
            "node1:6745".to_string(),
            "node2:6745".to_string(),
        ];
        
        // RF=2: primary + 1 replica
        let shard0_nodes = ShardRouter::shard_to_nodes(0, 2, &nodes);
        assert_eq!(shard0_nodes, vec!["node0:6745", "node1:6745"]);
        
        let shard1_nodes = ShardRouter::shard_to_nodes(1, 2, &nodes);
        assert_eq!(shard1_nodes, vec!["node1:6745", "node2:6745"]);
        
        let shard2_nodes = ShardRouter::shard_to_nodes(2, 2, &nodes);
        assert_eq!(shard2_nodes, vec!["node2:6745", "node0:6745"]);
        
        // RF=1: no replicas
        let shard0_primary = ShardRouter::shard_to_nodes(0, 1, &nodes);
        assert_eq!(shard0_primary, vec!["node0:6745"]);
        
        // RF=3: all nodes
        let shard0_all = ShardRouter::shard_to_nodes(0, 3, &nodes);
        assert_eq!(shard0_all.len(), 3);
    }

    #[test]
    fn test_is_shard_replica() {
        // 3 nodes, RF=2
        // Shard 0: [node0, node1] -> nodes 0 and 1 are replicas
        assert!(ShardRouter::is_shard_replica(0, 0, 2, 3));
        assert!(ShardRouter::is_shard_replica(0, 1, 2, 3));
        assert!(!ShardRouter::is_shard_replica(0, 2, 2, 3));
        
        // Shard 1: [node1, node2] -> nodes 1 and 2 are replicas
        assert!(!ShardRouter::is_shard_replica(1, 0, 2, 3));
        assert!(ShardRouter::is_shard_replica(1, 1, 2, 3));
        assert!(ShardRouter::is_shard_replica(1, 2, 2, 3));
        
        // RF=1: only primary
        assert!(ShardRouter::is_shard_replica(0, 0, 1, 3));
        assert!(!ShardRouter::is_shard_replica(0, 1, 1, 3));
    }
}
