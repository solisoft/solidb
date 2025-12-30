//! Sync Protocol Coverage Tests
//!
//! Additional tests for sync/protocol.rs covering:
//! - All SyncMessage variants
//! - SyncEntry creation and serialization
//! - Operation enum variants
//! - ShardConfig and ShardAssignment
//! - Edge cases

use solidb::sync::protocol::{
    compute_shard_id, NodeStats, Operation, ShardAssignment, ShardConfig, SyncEntry, SyncMessage,
};

// ============================================================================
// Operation Enum Tests
// ============================================================================

#[test]
fn test_operation_variants() {
    // Verify all operation variants exist and can be serialized
    let ops = vec![
        Operation::Insert,
        Operation::Update,
        Operation::Delete,
        Operation::CreateCollection,
        Operation::DeleteCollection,
        Operation::TruncateCollection,
        Operation::CreateDatabase,
        Operation::DeleteDatabase,
        Operation::PutBlobChunk,
        Operation::DeleteBlob,
    ];
    
    for op in ops {
        // Serialize and deserialize
        let serialized = bincode::serialize(&op).unwrap();
        let deserialized: Operation = bincode::deserialize(&serialized).unwrap();
        assert_eq!(op, deserialized);
    }
}

// ============================================================================
// SyncEntry Tests
// ============================================================================

#[test]
fn test_sync_entry_new() {
    let entry = SyncEntry::new(
        1,                          // sequence
        "node1".to_string(),        // origin_node
        1,                          // origin_sequence
        1234567890,                 // hlc_ts
        0,                          // hlc_count
        "testdb".to_string(),       // database
        "users".to_string(),        // collection
        Operation::Insert,          // operation
        "doc1".to_string(),         // document_key
        Some(vec![1, 2, 3, 4]),     // document_data
        Some(0),                    // shard_id
    );
    
    assert_eq!(entry.sequence, 1);
    assert_eq!(entry.origin_node, "node1");
    assert_eq!(entry.database, "testdb");
    assert_eq!(entry.collection, "users");
    assert_eq!(entry.document_key, "doc1");
    assert_eq!(entry.operation, Operation::Insert);
    assert!(entry.document_data.is_some());
    assert_eq!(entry.shard_id, Some(0));
}

#[test]
fn test_sync_entry_without_data() {
    let entry = SyncEntry::new(
        1,
        "node1".to_string(),
        1,
        1234567890,
        0,
        "testdb".to_string(),
        "users".to_string(),
        Operation::Delete,
        "doc1".to_string(),
        None,  // No document data for delete
        None,  // No shard_id
    );
    
    assert!(entry.document_data.is_none());
    assert!(entry.shard_id.is_none());
}

#[test]
fn test_sync_entry_serialization() {
    let entry = SyncEntry::new(
        42,
        "origin_node".to_string(),
        100,
        1234567890,
        5,
        "mydb".to_string(),
        "mycol".to_string(),
        Operation::Update,
        "key123".to_string(),
        Some(b"document data".to_vec()),
        Some(3),
    );
    
    let serialized = bincode::serialize(&entry).unwrap();
    let deserialized: SyncEntry = bincode::deserialize(&serialized).unwrap();
    
    assert_eq!(deserialized.sequence, 42);
    assert_eq!(deserialized.origin_node, "origin_node");
    assert_eq!(deserialized.database, "mydb");
    assert_eq!(deserialized.operation, Operation::Update);
}

// ============================================================================
// ShardConfig Tests
// ============================================================================

#[test]
fn test_shard_config_new() {
    let config = ShardConfig::new(8, 3);
    
    assert_eq!(config.num_shards, 8);
    assert_eq!(config.replication_factor, 3);
    assert_eq!(config.shard_key, "_key");
}

#[test]
fn test_shard_config_default() {
    let config = ShardConfig::default();
    
    assert_eq!(config.num_shards, 0);
    assert_eq!(config.replication_factor, 0);
    assert!(config.shard_key.is_empty());
}

#[test]
fn test_shard_config_serialization() {
    let config = ShardConfig::new(16, 2);
    
    let serialized = bincode::serialize(&config).unwrap();
    let deserialized: ShardConfig = bincode::deserialize(&serialized).unwrap();
    
    assert_eq!(deserialized.num_shards, 16);
    assert_eq!(deserialized.replication_factor, 2);
}

// ============================================================================
// ShardAssignment Tests
// ============================================================================

#[test]
fn test_shard_assignment_serialization() {
    let assignment = ShardAssignment {
        shard_id: 5,
        owner: "node1".to_string(),
        replicas: vec!["node2".to_string(), "node3".to_string()],
    };
    
    let serialized = bincode::serialize(&assignment).unwrap();
    let deserialized: ShardAssignment = bincode::deserialize(&serialized).unwrap();
    
    assert_eq!(deserialized.shard_id, 5);
    assert_eq!(deserialized.owner, "node1");
    assert_eq!(deserialized.replicas.len(), 2);
}

// ============================================================================
// NodeStats Tests
// ============================================================================

#[test]
fn test_node_stats_default() {
    let stats = NodeStats::default();
    
    assert_eq!(stats.cpu_usage, 0.0);
    assert_eq!(stats.memory_used, 0);
    assert_eq!(stats.disk_used, 0);
    assert_eq!(stats.document_count, 0);
    assert_eq!(stats.collections_count, 0);
}

#[test]
fn test_node_stats_with_values() {
    let stats = NodeStats {
        cpu_usage: 45.5,
        memory_used: 1024 * 1024 * 512,
        disk_used: 1024 * 1024 * 1024 * 10,
        document_count: 1_000_000,
        collections_count: 50,
    };
    
    let serialized = bincode::serialize(&stats).unwrap();
    let deserialized: NodeStats = bincode::deserialize(&serialized).unwrap();
    
    assert_eq!(deserialized.cpu_usage, 45.5);
    assert_eq!(deserialized.document_count, 1_000_000);
}

// ============================================================================
// SyncMessage Tests - All Variants
// ============================================================================

#[test]
fn test_sync_message_auth_challenge() {
    let msg = SyncMessage::AuthChallenge {
        challenge: vec![1, 2, 3, 4, 5, 6, 7, 8],
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::AuthChallenge { challenge } => {
            assert_eq!(challenge.len(), 8);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_auth_response() {
    let msg = SyncMessage::AuthResponse {
        hmac: vec![10, 20, 30, 40],
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::AuthResponse { hmac } => {
            assert_eq!(hmac, vec![10, 20, 30, 40]);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_auth_result() {
    let msg = SyncMessage::AuthResult {
        success: true,
        message: "Authenticated".to_string(),
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::AuthResult { success, message } => {
            assert!(success);
            assert_eq!(message, "Authenticated");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_incremental_sync_request() {
    let msg = SyncMessage::IncrementalSyncRequest {
        from_node: "node1".to_string(),
        after_sequence: 100,
        max_batch_bytes: 1024 * 1024,
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::IncrementalSyncRequest { from_node, after_sequence, max_batch_bytes } => {
            assert_eq!(from_node, "node1");
            assert_eq!(after_sequence, 100);
            assert_eq!(max_batch_bytes, 1024 * 1024);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_full_sync_request() {
    let msg = SyncMessage::FullSyncRequest {
        from_node: "newnode".to_string(),
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::FullSyncRequest { from_node } => {
            assert_eq!(from_node, "newnode");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_full_sync_start() {
    let msg = SyncMessage::FullSyncStart {
        total_databases: 5,
        total_collections: 20,
        total_documents: 100_000,
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::FullSyncStart { total_databases, total_collections, total_documents } => {
            assert_eq!(total_databases, 5);
            assert_eq!(total_collections, 20);
            assert_eq!(total_documents, 100_000);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_heartbeat() {
    let msg = SyncMessage::Heartbeat {
        node_id: "node1".to_string(),
        sequence: 500,
        stats: NodeStats::default(),
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::Heartbeat { node_id, sequence, .. } => {
            assert_eq!(node_id, "node1");
            assert_eq!(sequence, 500);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_heartbeat_ack() {
    let msg = SyncMessage::HeartbeatAck {
        node_id: "node2".to_string(),
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::HeartbeatAck { node_id } => {
            assert_eq!(node_id, "node2");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_node_join() {
    let msg = SyncMessage::NodeJoin {
        node_id: "new_node".to_string(),
        address: "192.168.1.100:4000".to_string(),
        http_address: "192.168.1.100:3000".to_string(),
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::NodeJoin { node_id, address, http_address } => {
            assert_eq!(node_id, "new_node");
            assert_eq!(address, "192.168.1.100:4000");
            assert_eq!(http_address, "192.168.1.100:3000");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_node_leave() {
    let msg = SyncMessage::NodeLeave {
        node_id: "leaving_node".to_string(),
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::NodeLeave { node_id } => {
            assert_eq!(node_id, "leaving_node");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_node_dead() {
    let msg = SyncMessage::NodeDead {
        node_id: "dead_node".to_string(),
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::NodeDead { node_id } => {
            assert_eq!(node_id, "dead_node");
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_sync_batch() {
    let entries = vec![
        SyncEntry::new(
            1, "node1".to_string(), 1, 1234567890, 0,
            "db".to_string(), "col".to_string(),
            Operation::Insert, "key1".to_string(),
            Some(b"data".to_vec()), None,
        ),
    ];
    
    let msg = SyncMessage::SyncBatch {
        entries,
        has_more: true,
        current_sequence: 1,
        compressed: false,
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::SyncBatch { entries, has_more, current_sequence, compressed } => {
            assert_eq!(entries.len(), 1);
            assert!(has_more);
            assert_eq!(current_sequence, 1);
            assert!(!compressed);
        }
        _ => panic!("Wrong message type"),
    }
}

#[test]
fn test_sync_message_shard_rebalance() {
    let assignments = vec![
        ShardAssignment {
            shard_id: 0,
            owner: "node1".to_string(),
            replicas: vec!["node2".to_string()],
        },
        ShardAssignment {
            shard_id: 1,
            owner: "node2".to_string(),
            replicas: vec!["node1".to_string()],
        },
    ];
    
    let msg = SyncMessage::ShardRebalance {
        database: "mydb".to_string(),
        collection: "sharded_col".to_string(),
        assignments,
    };
    
    let encoded = msg.encode();
    let decoded = SyncMessage::decode(&encoded[4..]).unwrap();
    
    match decoded {
        SyncMessage::ShardRebalance { database, collection, assignments } => {
            assert_eq!(database, "mydb");
            assert_eq!(collection, "sharded_col");
            assert_eq!(assignments.len(), 2);
        }
        _ => panic!("Wrong message type"),
    }
}

// ============================================================================
// compute_shard_id Tests
// ============================================================================

#[test]
fn test_compute_shard_id_distribution() {
    // Test that different keys map to different shards
    let num_shards = 16;
    let mut shard_counts = vec![0; num_shards as usize];
    
    for i in 0..1000 {
        let key = format!("document_{}", i);
        let shard = compute_shard_id(&key, num_shards);
        shard_counts[shard as usize] += 1;
    }
    
    // Each shard should have some documents (rough distribution check)
    for count in &shard_counts {
        assert!(*count > 0, "All shards should have some documents");
    }
}

#[test]
fn test_compute_shard_id_deterministic() {
    let key = "test_document_key";
    let num_shards = 8;
    
    // Same key should always give same shard
    let shard1 = compute_shard_id(key, num_shards);
    let shard2 = compute_shard_id(key, num_shards);
    let shard3 = compute_shard_id(key, num_shards);
    
    assert_eq!(shard1, shard2);
    assert_eq!(shard2, shard3);
}

#[test]
fn test_compute_shard_id_bounds() {
    let num_shards = 10;
    
    for i in 0..100 {
        let key = format!("key_{}", i);
        let shard = compute_shard_id(&key, num_shards);
        assert!(shard < num_shards, "Shard ID should be less than num_shards");
    }
}

#[test]
fn test_compute_shard_id_single_shard() {
    // Edge case: only one shard
    let shard = compute_shard_id("any_key", 1);
    assert_eq!(shard, 0);
}
