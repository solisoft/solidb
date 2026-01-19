//! Shard assignment and routing logic

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Route a key to a shard ID using consistent hashing
pub fn route_key(key: &str, num_shards: u16) -> u16 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();
    (hash % u64::from(num_shards)) as u16
}

/// Get replica nodes for a given key and config
pub fn get_replicas_for_key(
    key: &str,
    assignments: &HashMap<u16, crate::sharding::ShardAssignment>,
) -> Vec<String> {
    let num_shards = assignments.len() as u16;
    let shard_id = route_key(key, num_shards);
    if let Some(assignment) = assignments.get(&shard_id) {
        let mut replicas = vec![assignment.primary_node.clone()];
        replicas.extend(assignment.replica_nodes.clone());
        replicas
    } else {
        Vec::new()
    }
}
