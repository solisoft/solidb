#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use crate::sharding::coordinator::ShardAssignment;
#[cfg(test)]
use crate::sharding::distribution::compute_assignments;

#[test]
fn test_reproduce_user_overload_issue() {
    // Initial state: Nodes 6745, 6746, 6747
    // S0: P=6745, R=[6746]
    // S1: P=6746, R=[6747]
    // S2: P=6747, R=[6745]
    let mut old_assignments = HashMap::new();
    old_assignments.insert(0, ShardAssignment { shard_id: 0, primary_node: "6745".to_string(), replica_nodes: vec!["6746".to_string()] });
    old_assignments.insert(1, ShardAssignment { shard_id: 1, primary_node: "6746".to_string(), replica_nodes: vec!["6747".to_string()] });
    old_assignments.insert(2, ShardAssignment { shard_id: 2, primary_node: "6747".to_string(), replica_nodes: vec!["6745".to_string()] });

    // 6745 goes DOWN. 6748 comes UP.
    let nodes = vec!["6746".to_string(), "6747".to_string(), "6748".to_string()];

    // Compute RF=2
    let assignments = compute_assignments(&nodes, 3, 2, Some(&old_assignments)).unwrap();

    // Print results for debugging
    for id in 0..3 {
        let a = &assignments[&id];
        println!("Shard {}: P={}, R={:?}", id, a.primary_node, a.replica_nodes);
    }

    // Check for "assigned twice" as primary
    let mut primary_counts = HashMap::new();
    for a in assignments.values() {
        *primary_counts.entry(a.primary_node.clone()).or_insert(0) += 1;
    }

    // If 6746 is primary for S0 (promoted) and S1 (preserved), it has count 2.
    // While 6748 has count 0.
    assert!(primary_counts.get("6748").copied().unwrap_or(0) > 0, 
        "Node 6748 should be recruited as primary for one of the shards to balance load. 6748 count: {:?}", 
        primary_counts.get("6748"));
}
