use solidb::storage::engine::StorageEngine;
use solidb::sharding::coordinator::{CollectionShardConfig, ShardAssignment};
use solidb::sharding::migration::BatchSender;
use solidb::sharding::distribution;
use tempfile::TempDir;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use async_trait::async_trait;

#[tokio::test]
async fn verify_distribution_logic() {
    // 1. Verify Even Distribution (Primary)
    let nodes = vec!["node1".to_string(), "node2".to_string(), "node3".to_string()];
    let num_shards = 6; // Divisible by 3
    let replication_factor = 2; // 1 primary + 1 replica

    let assignments = distribution::compute_assignments(&nodes, num_shards, replication_factor, None).unwrap();

    let mut primary_counts = HashMap::new();
    for (_id, assign) in &assignments {
        *primary_counts.entry(assign.primary_node.clone()).or_insert(0) += 1;
    }

    // Each node should have exactly 2 primaries
    for node in &nodes {
        assert_eq!(*primary_counts.get(node).unwrap(), 2, "Node {} should have 2 primaries", node);
    }

    // 2. Verify Anti-Affinity (Replica never on primary node)
    for (id, assign) in &assignments {
        assert!(!assign.replica_nodes.contains(&assign.primary_node),
            "Shard {}: Primary {} found in replicas {:?}", id, assign.primary_node, assign.replica_nodes);

        // Also verify replica count
        assert_eq!(assign.replica_nodes.len(), (replication_factor - 1) as usize);
    }
}

// Mock Sender to verify migration
struct MockSender {
    sent_batches: Arc<Mutex<Vec<(String, Vec<(String, serde_json::Value)>)>>>,
}

#[async_trait]
impl BatchSender for MockSender {
    async fn send_batch(
        &self,
        _db: &str,
        coll: &str,
        _config: &CollectionShardConfig,
        batch: Vec<(String, serde_json::Value)>
    ) -> Result<Vec<String>, String> {
        let mut sent = self.sent_batches.lock().unwrap();
        sent.push((coll.to_string(), batch.clone()));
        // Return all keys as successfully processed
        Ok(batch.into_iter().map(|(key, _)| key).collect())
    }
}

#[tokio::test]
async fn verify_resharding_migration() {
    let temp_dir = TempDir::new().unwrap();
    let storage = StorageEngine::new(temp_dir.path()).unwrap();
    storage.initialize().unwrap();

    let db_name = "test_db";
    storage.create_database(db_name.to_string()).unwrap();
    let db = storage.get_database(db_name).unwrap();

    let coll_name = "migrated_coll";
    db.create_collection(coll_name.to_string(), None).unwrap();
    let main_coll = db.get_collection(coll_name).unwrap();

    // Scenario: Expanding from 2 to 4 shards
    // Node 1 was owner of s0, s1 (since 2 shards, 1 node?)
    // Let's assume we are Node 1 and we have data in s0 and s1.
    // New config has 4 shards.
    // Documents in s0/s1 need to be re-routed.
    // Some might stay in s0/s1 (if hash matches), others go to s2/s3 (moved).
    // Or even from s0 -> s1 if hash modulo changes.

    // 1. Setup Data
    let old_shards = 2;
    let new_shards = 4;

    // Set config to NEW count so router uses it
    let config = CollectionShardConfig {
        num_shards: new_shards,
        replication_factor: 1,
        shard_key: "_key".to_string(),
    };
    main_coll.set_shard_config(&config).unwrap();

    // Populate "physical" shards s0 and s1 manually
    let s0_name = format!("{}_s0", coll_name);
    let s1_name = format!("{}_s1", coll_name);
    db.create_collection(s0_name.clone(), None).unwrap();
    db.create_collection(s1_name.clone(), None).unwrap();

    let s0 = db.get_collection(&s0_name).unwrap();
    let s1 = db.get_collection(&s1_name).unwrap();

    // Create docs.
    // We want some docs that currently reside in s0 but SHOULD route to s2 or s3 (or s1) with new modulus.
    // Using default consistent hasher (DefaultHasher).
    // Let's create enough docs to ensure some move.

    for i in 0..100 {
        let doc = serde_json::json!({
            "_key": format!("doc_{}", i),
            "val": i
        });
        // We put them in s0 initially (simulating old distribution where mod 2 sent them here)
        // Actually we should put them where they would live with mod 2.
        // But for migration test, we just check if it detects MISPLACED docs.
        // reshard_collection iterates ALL local shards.
        // If doc routes to 's_new', and 's_new' != 'current_s', it moves.
        // So we can put ALL docs in s0, and expect approx 75% to move (to s1, s2, s3).
        // (Assuming uniform distribution 1/4 per shard. s0 keeps 1/4).

        s0.insert(doc).unwrap();
    }

    // 2. Setup Mock Sender
    let sent_batches = Arc::new(Mutex::new(Vec::new()));
    let sender = MockSender { sent_batches: sent_batches.clone() };

    // 3. Run Resharding
    // We impersonate the primary for s0 and s1 (local node)
    let my_node = "node1";
    let mut current_assignments = HashMap::new();
    // We claim to be primary for shards 0 and 1
    current_assignments.insert(0, ShardAssignment { shard_id: 0, primary_node: my_node.to_string(), replica_nodes: vec![] });
    current_assignments.insert(1, ShardAssignment { shard_id: 1, primary_node: my_node.to_string(), replica_nodes: vec![] });
    // Also claim 2 and 3 for completeness (though they are empty locally)
    current_assignments.insert(2, ShardAssignment { shard_id: 2, primary_node: my_node.to_string(), replica_nodes: vec![] });
    current_assignments.insert(3, ShardAssignment { shard_id: 3, primary_node: my_node.to_string(), replica_nodes: vec![] });

    let old_assignments = HashMap::new(); // Not contraction, so empty

    solidb::sharding::migration::reshard_collection(
        &storage,
        &sender,
        db_name,
        coll_name,
        old_shards,
        new_shards,
        my_node,
        &old_assignments,
        &current_assignments
    ).await.unwrap();

    // 4. Verify Results
    let sent = sent_batches.lock().unwrap();
    let total_moved: usize = sent.iter().map(|(_, batch)| batch.len()).sum();

    println!("Total docs moved: {}", total_moved);
    assert!(total_moved > 0, "Should have moved some documents");
    assert!(total_moved < 100, "Should not move ALL documents (some should stay in s0)");

    // Verify deleted from s0
    let remaining = s0.count();
    println!("Remaining in s0: {}", remaining);
    assert_eq!(remaining + total_moved, 100, "Docs should be conserved (remaining + moved = total)");

    // Check destination routing of moved docs?
    // The MockSender just captures them. The logic inside reshard_collection determined they NEED to move.
    // The logic is: new_shard_id != current_s.
    // So every doc in 'sent' was definitely routed to NOT s0.

    // Verify one doc
    if let Some((_, batch)) = sent.first() {
        let (key, _) = &batch[0];
        // this key was moved. Check route.
        use solidb::sharding::router::ShardRouter;
        let route = ShardRouter::route(key, new_shards);
        assert_ne!(route, 0, "Moved doc should routed to somewhere other than s0");
    }
}

#[tokio::test]
async fn verify_cleanup_logic() {
    let temp_dir = TempDir::new().unwrap();
    let storage = Arc::new(StorageEngine::new(temp_dir.path()).unwrap());
    storage.initialize().unwrap();

    let coordinator = solidb::sharding::coordinator::ShardCoordinator::new(
        storage.clone(),
        None,
        None
    );

    let db_name = "test_db";
    storage.create_database(db_name.to_string()).unwrap();
    let db = storage.get_database(db_name).unwrap();

    let coll_name = "cleanup_coll";

    // Manually create shards: s0, s1, s2, s3
    for i in 0..4 {
        let s_name = format!("{}_s{}", coll_name, i);
        db.create_collection(s_name, None).unwrap();
    }

    // Define table with ONLY s0, s1, s2 (s3 is orphaned)
    let config = CollectionShardConfig {
        num_shards: 3,
        replication_factor: 1,
        shard_key: "_key".to_string(),
    };

    // Init coordinator with this config for s0, s1, s2
    // This will calculate assignments for 0..2
    // Since we are single node, all will be assigned to "local" (or whatever my_node_id returns)
    // my_node_id defaults to "local" without cluster manager.
    let _ = coordinator.init_collection(db_name, coll_name, &config).unwrap();

    // Verify s3 exists before cleanup
    assert!(db.get_collection(&format!("{}_s3", coll_name)).is_ok());

    // Run cleanup
    let cleaned = coordinator.cleanup_orphaned_shards().await.unwrap();

    println!("Cleaned count: {}", cleaned);
    assert_eq!(cleaned, 1, "Should clean exactly 1 orphaned shard (s3)");

    // Verify s3 is gone
    assert!(db.get_collection(&format!("{}_s3", coll_name)).is_err(), "s3 should be deleted");

    // Verify s0, s1, s2 still exist
    assert!(db.get_collection(&format!("{}_s0", coll_name)).is_ok());
    assert!(db.get_collection(&format!("{}_s1", coll_name)).is_ok());
    assert!(db.get_collection(&format!("{}_s2", coll_name)).is_ok());
}


