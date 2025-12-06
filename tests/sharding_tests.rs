use solidb::StorageEngine;
use solidb::sharding::coordinator::{ShardCoordinator, CollectionShardConfig};
use solidb::sharding::ShardRouter; // Added import
use std::sync::Arc;
use tempfile::TempDir;

// Helper to create a test storage engine
fn create_test_storage() -> (Arc<StorageEngine>, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (Arc::new(storage), temp_dir)
}

#[test]
fn test_shard_routing_distribution() {
    let (storage, _dir) = create_test_storage();
    let nodes = vec![
        "http://n1:80".to_string(),
        "http://n2:80".to_string(),
        "http://n3:80".to_string(),
    ];
    let coordinator = ShardCoordinator::new(storage, 0, nodes);
    
    let config = CollectionShardConfig {
        num_shards: 16,
        replication_factor: 1,
        shard_key: "_key".to_string(),
    };

    // Verify deterministic routing
    let shard_id = ShardRouter::route("key_a", config.num_shards);
    let node_a = coordinator.get_shard_address(shard_id); 
    let node_b = coordinator.get_shard_address(shard_id);
    assert_eq!(node_a, node_b);
}

#[tokio::test] // Need tokio because remove_node is async
async fn test_topology_change_remove_node() {
    let (storage, _dir) = create_test_storage();
    let initial_nodes = vec![
        "http://node1:8000".to_string(),
        "http://node2:8000".to_string(),
        "http://node3:8000".to_string(),
    ];
    
    // Node 1 is self
    let coordinator = ShardCoordinator::new(storage, 0, initial_nodes.clone());
    
    let config = CollectionShardConfig {
        num_shards: 8,
        replication_factor: 2, // 2 replicas
        shard_key: "_key".to_string(),
    };

    // 1. Initial State Check
    // Get replicas for a specific key
    let replicas_initial = coordinator.get_replicas("test_doc_1", &config);
    assert!(replicas_initial.len() > 0);
    // Should be subset of initial_nodes
    for r in &replicas_initial {
        assert!(initial_nodes.contains(r));
    }

    // 2. Remove a valid node (node2)
    coordinator.remove_node("http://node2:8000").await.ok(); // Ignore error if rebalance network fails

    // 3. Verify Topology Update
    // Get replicas for same key
    let replicas_after = coordinator.get_replicas("test_doc_1", &config);
    
    // Node 2 should NOT be in the new replica set
    assert!(!replicas_after.contains(&"http://node2:8000".to_string()));
    
    // All returned replicas must be from remaining nodes (node1, node3)
    let remaining_nodes = vec![
        "http://node1:8000".to_string(),
        "http://node3:8000".to_string(),
    ];
    for r in &replicas_after {
        assert!(remaining_nodes.contains(r));
    }
}

#[tokio::test]
async fn test_get_shard_address_failover() {
    // Test failover logic in read path
    let (storage, _dir) = create_test_storage();
    let nodes = vec![
        "http://primary:80".to_string(),
        "http://secondary:80".to_string(),
    ];
    // setup failure threshold
    let coordinator = ShardCoordinator::with_health_tracking(storage, 0, nodes, 3);
    
    // We don't need config for pure shard_address check via shard_id
    let shard_id: u16 = 0;

    // Initially maps to Primary (index 0)
    let addr_initial = coordinator.get_shard_address(shard_id);
    assert_eq!(addr_initial, Some("http://primary:80".to_string()));

    // Ideally we would mock NodeHealth status here to force failover.
    // However, NodeHealth logic is internal/background.
    // We can simulate failover by removing the node (permanent failure).
    
    coordinator.remove_node("http://primary:80").await.ok();
    
    // After removal, index 0 maps to the remaining node (Secondary)
    let addr_after = coordinator.get_shard_address(shard_id);
    assert_eq!(addr_after, Some("http://secondary:80".to_string()));
}

#[test]
fn test_shard_config_update() {
    let (storage, _dir) = create_test_storage();
    
    // Create a database and collection
    storage.create_database("test_db".to_string()).expect("Failed to create database");
    let db = storage.get_database("test_db").expect("Failed to get database");
    db.create_collection("test_coll".to_string()).expect("Failed to create collection");
    let coll = db.get_collection("test_coll").expect("Failed to get collection");
    
    // Set initial shard config
    let initial_config = CollectionShardConfig {
        num_shards: 4,
        replication_factor: 1,
        shard_key: "_key".to_string(),
    };
    coll.set_shard_config(&initial_config).expect("Failed to set shard config");
    
    // Verify initial config
    let read_config = coll.get_shard_config().expect("Config should exist");
    assert_eq!(read_config.num_shards, 4);
    assert_eq!(read_config.replication_factor, 1);
    
    // Update shard count
    let updated_config = CollectionShardConfig {
        num_shards: 8,  // Changed from 4 to 8
        replication_factor: 2,  // Changed from 1 to 2
        shard_key: "_key".to_string(),
    };
    coll.set_shard_config(&updated_config).expect("Failed to update shard config");
    
    // Verify updated config persisted
    let read_updated = coll.get_shard_config().expect("Config should exist");
    assert_eq!(read_updated.num_shards, 8);
    assert_eq!(read_updated.replication_factor, 2);
    assert_eq!(read_updated.shard_key, "_key");
}
