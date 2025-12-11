use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use serde::{Deserialize, Serialize};
use super::node::{Node, NodeId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeStatus {
    Joining,
    Active,
    Suspected,
    Dead,
    Leaving,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterMember {
    pub node: Node,
    pub status: NodeStatus,
    pub last_heartbeat: u64,
    pub last_sequence: u64,
}

/// Manages the state of the cluster members
#[derive(Clone)]
pub struct ClusterState {
    pub members: Arc<RwLock<HashMap<NodeId, ClusterMember>>>,
    pub local_node_id: NodeId,
    pub max_origin_sequences: Arc<RwLock<HashMap<String, u64>>>,
    /// Tracks the last sequence from OUR log that we sent to each peer
    /// Key: peer node_id, Value: last sequence we sent from our local log
    pub sent_to_peers: Arc<RwLock<HashMap<String, u64>>>,
}

impl ClusterState {
    pub fn new(local_node_id: NodeId) -> Self {
        Self {
            members: Arc::new(RwLock::new(HashMap::new())),
            local_node_id,
            max_origin_sequences: Arc::new(RwLock::new(HashMap::new())),
            sent_to_peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn check_and_update_origin_sequence(&self, node_id: String, sequence: u64) -> bool {
        let mut seqs = self.max_origin_sequences.write().unwrap();
        let current = seqs.entry(node_id).or_insert(0);
        if sequence > *current {
            *current = sequence;
            true
        } else {
            false
        }
    }

    pub fn add_member(&self, node: Node, status: NodeStatus) -> bool {
        let mut members = self.members.write().unwrap();
        let exists = members.contains_key(&node.id);
        members.insert(node.id.clone(), ClusterMember {
            node,
            status,
            last_heartbeat: chrono::Utc::now().timestamp_millis() as u64,
            last_sequence: 0,
        });
        !exists
    }

    pub fn remove_member(&self, node_id: &str) {
        let mut members = self.members.write().unwrap();
        members.remove(node_id);
    }
    
    pub fn get_member(&self, node_id: &str) -> Option<ClusterMember> {
        let members = self.members.read().unwrap();
        members.get(node_id).cloned()
    }
    
    pub fn get_all_members(&self) -> Vec<ClusterMember> {
        let members = self.members.read().unwrap();
        members.values().cloned().collect()
    }

    pub fn update_heartbeat(&self, node_id: &str, sequence: u64) {
        let mut members = self.members.write().unwrap();
        if let Some(member) = members.get_mut(node_id) {
            member.last_heartbeat = chrono::Utc::now().timestamp_millis() as u64;
            member.last_sequence = sequence;
            if member.status == NodeStatus::Suspected {
                member.status = NodeStatus::Active;
            }
        }
    }
    
    pub fn mark_status(&self, node_id: &str, status: NodeStatus) {
        let mut members = self.members.write().unwrap();
        if let Some(member) = members.get_mut(node_id) {
            member.status = status;
        }
    }
    
    pub fn active_nodes(&self) -> Vec<Node> {
        let members = self.members.read().unwrap();
        members.values()
            .filter(|m| m.status == NodeStatus::Active)
            .map(|m| m.node.clone())
            .collect()
    }
    
    /// Get the last sequence we sent to a peer from our local log
    pub fn get_sent_to_peer(&self, peer_id: &str) -> u64 {
        let sent = self.sent_to_peers.read().unwrap();
        *sent.get(peer_id).unwrap_or(&0)
    }
    
    /// Update the last sequence we sent to a peer from our local log
    pub fn update_sent_to_peer(&self, peer_id: &str, sequence: u64) {
        let mut sent = self.sent_to_peers.write().unwrap();
        sent.insert(peer_id.to_string(), sequence);
    }
}
