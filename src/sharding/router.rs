use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Routes document keys to specific shard IDs
#[derive(Debug, Clone)]
pub struct ShardRouter;

impl ShardRouter {
    /// Calculate the shard ID for a given document key
    /// Uses consistent hashing: hash(key) % num_shards
    pub fn route(key: &str, num_shards: u16) -> u16 {
        if num_shards == 0 {
            return 0;
        }
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        (hasher.finish() % num_shards as u64) as u16
    }
    /// Check if a node index is a replica (or primary) for a given shard ID
    /// Uses deterministic logic: (shard_id + offset) % num_nodes == node_idx
    /// where offset is in [0, replication_factor)
    pub fn is_shard_replica(
        shard_id: u16,
        node_idx: usize,
        replication_factor: u16,
        num_nodes: usize,
    ) -> bool {
        if num_nodes == 0 {
            return false;
        }

        // Check if this node is assigned as primary or any replica
        for i in 0..replication_factor {
            let target_node_idx = (shard_id as usize + i as usize) % num_nodes;
            if target_node_idx == node_idx {
                return true;
            }
        }
        false
    }
}
