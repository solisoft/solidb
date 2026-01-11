use super::node::{Node, NodeId};
use super::stats::NodeBasicStats;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeStatus {
    Joining,
    Syncing,
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
    pub stats: Option<NodeBasicStats>,
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
        members.insert(
            node.id.clone(),
            ClusterMember {
                node,
                status,
                last_heartbeat: chrono::Utc::now().timestamp_millis() as u64,
                last_sequence: 0,
                stats: None,
            },
        );
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

    pub fn update_heartbeat(&self, node_id: &str, sequence: u64, stats: Option<NodeBasicStats>) {
        let mut members = self.members.write().unwrap();
        if let Some(member) = members.get_mut(node_id) {
            member.last_heartbeat = chrono::Utc::now().timestamp_millis() as u64;
            member.last_sequence = sequence;
            if let Some(s) = stats {
                member.stats = Some(s);
            }
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
        members
            .values()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_node(id: &str) -> Node {
        Node::new(
            id.to_string(),
            format!("127.0.0.1:{}", 8000),
            format!("127.0.0.1:{}", 9000),
        )
    }

    #[test]
    fn test_cluster_state_new() {
        let state = ClusterState::new("node1".to_string());

        assert_eq!(state.local_node_id, "node1");
        assert!(state.get_all_members().is_empty());
    }

    #[test]
    fn test_add_member() {
        let state = ClusterState::new("local".to_string());
        let node = create_test_node("node1");

        let is_new = state.add_member(node.clone(), NodeStatus::Active);

        assert!(is_new);
        assert_eq!(state.get_all_members().len(), 1);
    }

    #[test]
    fn test_add_member_duplicate() {
        let state = ClusterState::new("local".to_string());
        let node = create_test_node("node1");

        let first = state.add_member(node.clone(), NodeStatus::Active);
        let second = state.add_member(node.clone(), NodeStatus::Active);

        assert!(first); // First add is new
        assert!(!second); // Second add is not new (already exists)
        assert_eq!(state.get_all_members().len(), 1);
    }

    #[test]
    fn test_remove_member() {
        let state = ClusterState::new("local".to_string());
        let node = create_test_node("node1");

        state.add_member(node, NodeStatus::Active);
        assert_eq!(state.get_all_members().len(), 1);

        state.remove_member("node1");
        assert!(state.get_all_members().is_empty());
    }

    #[test]
    fn test_get_member() {
        let state = ClusterState::new("local".to_string());
        let node = create_test_node("node1");

        state.add_member(node.clone(), NodeStatus::Active);

        let member = state.get_member("node1");
        assert!(member.is_some());
        assert_eq!(member.unwrap().node.id, "node1");

        assert!(state.get_member("nonexistent").is_none());
    }

    #[test]
    fn test_mark_status() {
        let state = ClusterState::new("local".to_string());
        let node = create_test_node("node1");

        state.add_member(node, NodeStatus::Active);

        state.mark_status("node1", NodeStatus::Suspected);
        let member = state.get_member("node1").unwrap();
        assert_eq!(member.status, NodeStatus::Suspected);

        state.mark_status("node1", NodeStatus::Dead);
        let member = state.get_member("node1").unwrap();
        assert_eq!(member.status, NodeStatus::Dead);
    }

    #[test]
    fn test_active_nodes() {
        let state = ClusterState::new("local".to_string());

        state.add_member(create_test_node("node1"), NodeStatus::Active);
        state.add_member(create_test_node("node2"), NodeStatus::Suspected);
        state.add_member(create_test_node("node3"), NodeStatus::Active);

        let active = state.active_nodes();
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn test_update_heartbeat() {
        let state = ClusterState::new("local".to_string());
        let node = create_test_node("node1");

        state.add_member(node, NodeStatus::Suspected);

        // Heartbeat should update status from Suspected to Active
        state.update_heartbeat("node1", 10, None);

        let member = state.get_member("node1").unwrap();
        assert_eq!(member.status, NodeStatus::Active);
        assert_eq!(member.last_sequence, 10);
    }

    #[test]
    fn test_check_and_update_origin_sequence() {
        let state = ClusterState::new("local".to_string());

        // First update should succeed
        assert!(state.check_and_update_origin_sequence("origin1".to_string(), 5));

        // Same sequence should fail
        assert!(!state.check_and_update_origin_sequence("origin1".to_string(), 5));

        // Lower sequence should fail
        assert!(!state.check_and_update_origin_sequence("origin1".to_string(), 3));

        // Higher sequence should succeed
        assert!(state.check_and_update_origin_sequence("origin1".to_string(), 10));
    }

    #[test]
    fn test_sent_to_peer() {
        let state = ClusterState::new("local".to_string());

        // Default is 0
        assert_eq!(state.get_sent_to_peer("peer1"), 0);

        // Update
        state.update_sent_to_peer("peer1", 42);
        assert_eq!(state.get_sent_to_peer("peer1"), 42);

        // Different peer still 0
        assert_eq!(state.get_sent_to_peer("peer2"), 0);
    }

    #[test]
    fn test_node_status_variants() {
        assert_ne!(NodeStatus::Joining, NodeStatus::Active);
        assert_ne!(NodeStatus::Syncing, NodeStatus::Dead);
        assert_eq!(NodeStatus::Leaving.clone(), NodeStatus::Leaving);
    }

    #[test]
    fn test_cluster_state_clone() {
        let state1 = ClusterState::new("node1".to_string());
        state1.add_member(create_test_node("n1"), NodeStatus::Active);

        let state2 = state1.clone();

        // Both share the same underlying data
        assert_eq!(
            state1.get_all_members().len(),
            state2.get_all_members().len()
        );
    }
}
