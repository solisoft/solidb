//! Shard routing utilities

/// Router for determining shard assignments
pub struct ShardRouter;

impl ShardRouter {
    /// Route a key to a shard ID using seahash for uniform distribution
    ///
    /// seahash provides much better distribution than DefaultHasher for
    /// modulo operations on small numbers (like shard counts)
    pub fn route(key: &str, num_shards: u16) -> u16 {
        if num_shards == 0 {
            return 0;
        }

        // Use seahash for uniform distribution across shards
        let hash = seahash::hash(key.as_bytes());
        (hash % num_shards as u64) as u16
    }

    /// Check if a node should store a replica of a shard
    pub fn is_shard_replica(
        shard_id: u16,
        node_index: usize,
        replication_factor: u16,
        num_nodes: usize,
    ) -> bool {
        if num_nodes == 0 || replication_factor == 0 {
            return false;
        }

        let primary_node = (shard_id as usize) % num_nodes;

        for r in 0..replication_factor {
            let replica_node = (primary_node + r as usize) % num_nodes;
            if replica_node == node_index {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_consistency() {
        let shard1 = ShardRouter::route("key1", 10);
        let shard2 = ShardRouter::route("key1", 10);
        assert_eq!(shard1, shard2);
    }

    #[test]
    fn test_is_shard_replica() {
        // Shard 0, RF=2, 3 nodes: nodes 0 and 1 should have it
        assert!(ShardRouter::is_shard_replica(0, 0, 2, 3));
        assert!(ShardRouter::is_shard_replica(0, 1, 2, 3));
        assert!(!ShardRouter::is_shard_replica(0, 2, 2, 3));
    }
}
