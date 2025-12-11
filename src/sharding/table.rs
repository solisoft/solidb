use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Identifies a specific shard
pub type ShardId = u16;

/// Configuration of a single shard's placement
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShardAssignment {
    pub shard_id: ShardId,
    pub primary_node: String,
    pub replica_nodes: Vec<String>,
}

/// Lookup table for all shards in a collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShardTable {
    pub assignments: HashMap<ShardId, ShardAssignment>,
    pub num_shards: u16,
    pub replication_factor: u16,
}

impl ShardTable {
    pub fn new(num_shards: u16, replication_factor: u16) -> Self {
        Self {
            assignments: HashMap::new(),
            num_shards,
            replication_factor,
        }
    }
}

impl Default for ShardTable {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl ShardTable {
    pub fn assign(&mut self, shard_id: ShardId, primary: String, replicas: Vec<String>) {
        self.assignments.insert(shard_id, ShardAssignment {
            shard_id,
            primary_node: primary,
            replica_nodes: replicas,
        });
    }

    pub fn get_primary(&self, shard_id: ShardId) -> Option<&String> {
        self.assignments.get(&shard_id).map(|a| &a.primary_node)
    }

    pub fn get_replicas(&self, shard_id: ShardId) -> Option<&Vec<String>> {
        self.assignments.get(&shard_id).map(|a| &a.replica_nodes)
    }

    pub fn get_all_nodes(&self, shard_id: ShardId) -> Vec<String> {
        if let Some(assign) = self.assignments.get(&shard_id) {
            let mut nodes = vec![assign.primary_node.clone()];
            nodes.extend(assign.replica_nodes.clone());
            nodes
        } else {
            vec![]
        }
    }

    pub fn get_assignment(&self, shard_id: ShardId) -> Option<&ShardAssignment> {
        self.assignments.get(&shard_id)
    }
}
