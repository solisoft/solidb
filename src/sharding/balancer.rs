use std::collections::HashMap;
use super::table::{ShardTable, ShardId};
use crate::cluster::node::Node;

/// Logic to distribute shards across available nodes
pub struct ShardBalancer;

impl ShardBalancer {
    /// Create a new balanced table for a set of nodes
    pub fn create_balanced_table(
        num_shards: u16,
        replication_factor: u16,
        nodes: &[Node],
    ) -> ShardTable {
        let mut table = ShardTable::new(num_shards, replication_factor);
        
        if nodes.is_empty() {
            return table;
        }

        for shard_id in 0..num_shards {
            // Round-robin assignment for primary
            let primary_idx = (shard_id as usize) % nodes.len();
            let primary = nodes[primary_idx].id.clone();
            
            let mut replicas = Vec::new();
            if replication_factor > 1 {
                // Assign replicas to subsequent nodes
                for i in 1..replication_factor {
                    let replica_idx = (primary_idx + i as usize) % nodes.len();
                    // Avoid assigning replica to same node if RF > num_nodes
                    if replica_idx != primary_idx {
                        replicas.push(nodes[replica_idx].id.clone());
                    }
                }
            }
            
            table.assign(shard_id, primary, replicas);
        }
        
        table
    }

    /// Calculate migrations needed to move from old_table to new_table
    /// Returns list of (ShardId, SourceNode, TargetNode)
    pub fn calculate_migrations(
        old_table: &ShardTable,
        new_table: &ShardTable,
    ) -> Vec<(ShardId, String, String)> {
        // TODO: Implement smart diffing for minimal movement
        // For now, this is a placeholder for future logic
        vec![]
    }
}
