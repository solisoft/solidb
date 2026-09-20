//! Storage Engine Coverage Tests
//!
//! Additional tests for storage/engine.rs covering:
//! - Cluster configuration
//! - Transaction initialization
//! - Database lifecycle
//! - Collection management
//! - Flush operations

use serde_json::json;
use solidb::storage::StorageEngine;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

// ============================================================================
// Basic Engine Tests
// ============================================================================

#[test]
fn test_engine_creation() {
    let (engine, _tmp) = create_test_engine();
    assert!(!engine.is_cluster_mode());
}

#[test]
fn test_engine_data_dir() {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = tmp_dir.path().to_str().unwrap();
    let engine = StorageEngine::new(path).expect("Failed to create engine");

    assert!(engine
        .data_dir()
        .contains(tmp_dir.path().file_name().unwrap().to_str().unwrap()));
}

#[test]
fn test_engine_node_id_standalone() {
    let (engine, _tmp) = create_test_engine();
    assert_eq!(engine.node_id(), "standalone");
}

#[test]
fn test_engine_clone() {
    let (engine, _tmp) = create_test_engine();

    // Clone the engine
    let cloned = engine.clone();

    // Both should work independently
    engine.create_collection("col1".to_string(), None).unwrap();

    // Cloned engine should see the same collection
    assert!(cloned.get_collection("col1").is_ok());
}

// ============================================================================
// Database Operations Tests
// ============================================================================

#[test]
fn test_create_database() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.create_database("mydb".to_string());
    assert!(result.is_ok());
}

#[test]
fn test_create_duplicate_database() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("mydb".to_string()).unwrap();
    let result = engine.create_database("mydb".to_string());

    assert!(result.is_err());
}

#[test]
fn test_list_databases() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("db1".to_string()).unwrap();
    engine.create_database("db2".to_string()).unwrap();
    engine.create_database("db3".to_string()).unwrap();

    let databases = engine.list_databases();

    assert!(databases.contains(&"db1".to_string()));
    assert!(databases.contains(&"db2".to_string()));
    assert!(databases.contains(&"db3".to_string()));
}

#[test]
fn test_get_database() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("testdb".to_string()).unwrap();

    let db = engine.get_database("testdb");
    assert!(db.is_ok());
}

#[test]
fn test_get_nonexistent_database() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.get_database("nonexistent");
    assert!(result.is_err());
}

#[test]
fn test_delete_database() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("to_delete".to_string()).unwrap();

    // Verify it exists
    assert!(engine.get_database("to_delete").is_ok());

    // Delete
    engine.delete_database("to_delete").unwrap();

    // Verify it's gone
    assert!(engine.get_database("to_delete").is_err());
}

#[test]
fn test_delete_database_with_collections() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("dbwithcol".to_string()).unwrap();
    let db = engine.get_database("dbwithcol").unwrap();

    db.create_collection("users".to_string(), None).unwrap();
    db.create_collection("orders".to_string(), None).unwrap();

    // Delete database should remove collections too
    engine.delete_database("dbwithcol").unwrap();

    assert!(engine.get_database("dbwithcol").is_err());
}

#[test]
fn test_recreate_database_after_delete() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("recreated".to_string()).unwrap();
    let db = engine.get_database("recreated").unwrap();
    db.create_collection("users".to_string(), None).unwrap();
    let users = db.get_collection("users").unwrap();
    users
        .insert(json!({"_key": "alice", "name": "Alice"}))
        .unwrap();

    // Drop and immediately recreate the same database + collection. The old
    // CF may still be awaiting its background drop — the recreate must claim
    // it and come back empty, not fail with "already exists" or show stale
    // documents.
    engine.delete_database("recreated").unwrap();
    assert!(engine.get_database("recreated").is_err());

    engine.create_database("recreated".to_string()).unwrap();
    let db = engine.get_database("recreated").unwrap();

    // The doomed collection must not be visible on the fresh database
    assert!(!db.list_collections().contains(&"users".to_string()));
    assert!(db.get_collection("users").is_err());

    db.create_collection("users".to_string(), None).unwrap();
    let users = db.get_collection("users").unwrap();
    assert_eq!(users.count(), 0, "reclaimed collection must be empty");
}

#[test]
fn test_delete_database_twice_returns_not_found() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("twice".to_string()).unwrap();
    engine.delete_database("twice").unwrap();

    // The logical delete is immediate even though CF drops run in the
    // background — a second delete must already see the database as gone.
    assert!(engine.delete_database("twice").is_err());
}

// ============================================================================
// Collection Operations Tests
// ============================================================================

#[test]
fn test_create_collection() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.create_collection("users".to_string(), None);
    assert!(result.is_ok());
}

#[test]
fn test_create_edge_collection() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.create_collection("edges".to_string(), Some("edge".to_string()));
    assert!(result.is_ok());

    let col = engine.get_collection("edges").unwrap();
    assert_eq!(col.get_type(), "edge");
}

#[test]
fn test_list_collections() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("col1".to_string(), None).unwrap();
    engine.create_collection("col2".to_string(), None).unwrap();

    let collections = engine.list_collections();

    assert!(collections.contains(&"col1".to_string()));
    assert!(collections.contains(&"col2".to_string()));
}

#[test]
fn test_delete_collection() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("temp".to_string(), None).unwrap();

    // Verify it exists in list
    assert!(engine.list_collections().contains(&"temp".to_string()));

    // Delete
    engine.delete_collection("temp").unwrap();

    // Verify it's gone from list
    assert!(!engine.list_collections().contains(&"temp".to_string()));
}

#[test]
fn test_save_collection_noop() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("coll".to_string(), None).unwrap();

    // save_collection is a no-op with RocksDB
    let result = engine.save_collection("coll");
    assert!(result.is_ok());
}

// ============================================================================
// Transaction Operations Tests
// ============================================================================

#[test]
fn test_initialize_transactions() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.initialize_transactions();
    assert!(result.is_ok());
}

#[test]
fn test_transaction_manager() {
    let (engine, _tmp) = create_test_engine();

    // First call should initialize and return manager
    let result = engine.transaction_manager();
    assert!(result.is_ok());

    // Second call should return the same manager
    let result2 = engine.transaction_manager();
    assert!(result2.is_ok());
}

#[test]
fn test_begin_and_commit_transaction() {
    use solidb::transaction::IsolationLevel;

    let (engine, _tmp) = create_test_engine();

    let manager = engine.transaction_manager().unwrap();
    let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();

    let result = engine.commit_transaction(tx_id);
    assert!(result.is_ok());
}

#[test]
fn test_begin_and_rollback_transaction() {
    use solidb::transaction::IsolationLevel;

    let (engine, _tmp) = create_test_engine();

    let manager = engine.transaction_manager().unwrap();
    let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();

    let result = engine.rollback_transaction(tx_id);
    assert!(result.is_ok());
}

// ============================================================================
// Flush Operations Tests
// ============================================================================

#[test]
fn test_flush() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();
    let col = engine.get_collection("data").unwrap();

    col.insert(json!({"key": "value"})).unwrap();

    let result = engine.flush();
    assert!(result.is_ok());
}

#[test]
fn test_flush_all_stats() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("stats_test".to_string(), None)
        .unwrap();
    let col = engine.get_collection("stats_test").unwrap();

    for i in 0..10 {
        col.insert(json!({"num": i})).unwrap();
    }

    // Flush stats should not panic
    engine.flush_all_stats();
}

#[test]
fn test_recalculate_all_counts() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("count_test".to_string(), None)
        .unwrap();
    let col = engine.get_collection("count_test").unwrap();

    for i in 0..5 {
        col.insert(json!({"num": i})).unwrap();
    }

    // Recalculate counts should not panic
    engine.recalculate_all_counts();
}

// ============================================================================
// Initialize Tests
// ============================================================================

#[test]
fn test_initialize() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.initialize();
    assert!(result.is_ok());

    // _system database should exist
    assert!(engine.get_database("_system").is_ok());
}

#[test]
fn test_initialize_idempotent() {
    let (engine, _tmp) = create_test_engine();

    // Initialize multiple times
    engine.initialize().unwrap();
    engine.initialize().unwrap();
    engine.initialize().unwrap();

    // Should still work
    assert!(engine.get_database("_system").is_ok());
}

// ============================================================================
// Persistence Tests
// ============================================================================

#[test]
fn test_data_persists_across_reopen() {
    let tmp_dir = TempDir::new().unwrap();
    let path = tmp_dir.path().to_str().unwrap();

    // First session: create and insert
    {
        let engine = StorageEngine::new(path).unwrap();
        engine
            .create_collection("persistent".to_string(), None)
            .unwrap();
        let col = engine.get_collection("persistent").unwrap();
        col.insert(json!({"_key": "doc1", "data": "hello"}))
            .unwrap();
        engine.flush().unwrap();
        engine.flush_all_stats();
    }

    // Second session: verify data exists
    {
        let engine = StorageEngine::new(path).unwrap();
        let col = engine.get_collection("persistent").unwrap();
        let doc = col.get("doc1").unwrap();
        assert_eq!(doc.get("data"), Some(json!("hello")));
    }
}

#[test]
fn test_count_persists_across_reopen() {
    let tmp_dir = TempDir::new().unwrap();
    let path = tmp_dir.path().to_str().unwrap();

    // First session: insert documents
    {
        let engine = StorageEngine::new(path).unwrap();
        engine
            .create_collection("counted".to_string(), None)
            .unwrap();
        let col = engine.get_collection("counted").unwrap();

        for i in 0..10 {
            col.insert(json!({"num": i})).unwrap();
        }

        assert_eq!(col.count(), 10);
        engine.flush_all_stats();
    }

    // Second session: verify count
    {
        let engine = StorageEngine::new(path).unwrap();
        let col = engine.get_collection("counted").unwrap();
        assert_eq!(col.count(), 10);
    }
}

// ============================================================================
// Edge Cases Tests
// ============================================================================

#[test]
fn test_collection_name_with_database_prefix() {
    let (engine, _tmp) = create_test_engine();

    // Create via database
    engine.create_database("mydb".to_string()).unwrap();
    let db = engine.get_database("mydb").unwrap();
    db.create_collection("items".to_string(), None).unwrap();

    // Should be accessible via get_collection
    let col = engine.get_collection("mydb:items").unwrap();
    col.insert(json!({"test": true})).unwrap();

    assert_eq!(col.count(), 1);
}

#[test]
fn test_multiple_databases_isolation() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("app1".to_string()).unwrap();
    engine.create_database("app2".to_string()).unwrap();

    let db1 = engine.get_database("app1").unwrap();
    let db2 = engine.get_database("app2").unwrap();

    db1.create_collection("users".to_string(), None).unwrap();
    db2.create_collection("users".to_string(), None).unwrap();

    // Insert to app1
    let col1 = db1.get_collection("users").unwrap();
    col1.insert(json!({"name": "Alice"})).unwrap();

    // app2 should be empty
    let col2 = db2.get_collection("users").unwrap();
    assert_eq!(col2.count(), 0);
    assert_eq!(col1.count(), 1);
}

// ===========================================================================
// create_checkpoint — physical backup
// ===========================================================================

/// A checkpoint must be openable as a standalone database containing the data
/// that existed when it was taken. This is the whole point of the mechanism:
/// `solidb-dump` is a logical export, and nothing else produces a physical,
/// point-in-time-consistent copy.
#[test]
fn test_checkpoint_is_a_usable_standalone_copy() {
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();

    engine.create_database("shop".to_string()).unwrap();
    let db = engine.get_database("shop").unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    let orders = db.get_collection("orders").unwrap();
    orders.insert(json!({"_key": "o1", "total": 42})).unwrap();
    orders.insert(json!({"_key": "o2", "total": 7})).unwrap();

    let backup_dir = TempDir::new().unwrap();
    let target = backup_dir.path().join("snapshot");
    engine.create_checkpoint(&target).unwrap();
    assert!(target.exists(), "checkpoint directory should be created");

    // Open the checkpoint as its own engine — a backup you cannot open is not
    // a backup.
    let restored = StorageEngine::new(target.to_str().unwrap()).expect("open checkpoint");
    let rdb = restored
        .get_database("shop")
        .expect("database in checkpoint");
    let rorders = rdb
        .get_collection("orders")
        .expect("collection in checkpoint");
    assert_eq!(rorders.count(), 2);
    assert_eq!(rorders.get("o1").unwrap().data["total"], 42);
    assert_eq!(rorders.get("o2").unwrap().data["total"], 7);
}

/// Writes made after the checkpoint must not appear in it, or it is not a
/// point-in-time snapshot.
#[test]
fn test_checkpoint_does_not_capture_later_writes() {
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();
    engine.create_database("shop".to_string()).unwrap();
    let db = engine.get_database("shop").unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    let orders = db.get_collection("orders").unwrap();
    orders.insert(json!({"_key": "before", "n": 1})).unwrap();

    let backup_dir = TempDir::new().unwrap();
    let target = backup_dir.path().join("snapshot");
    engine.create_checkpoint(&target).unwrap();

    orders.insert(json!({"_key": "after", "n": 2})).unwrap();

    let restored = StorageEngine::new(target.to_str().unwrap()).unwrap();
    let rorders = restored
        .get_database("shop")
        .unwrap()
        .get_collection("orders")
        .unwrap();
    assert!(
        rorders.get("before").is_ok(),
        "pre-checkpoint write present"
    );
    assert!(
        rorders.get("after").is_err(),
        "post-checkpoint write must not be in the snapshot"
    );
    // The live database still has both.
    assert_eq!(orders.count(), 2);
}

/// RocksDB refuses to checkpoint into an existing directory; fail with a clear
/// message rather than a raw engine error.
#[test]
fn test_checkpoint_refuses_existing_target() {
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();

    let backup_dir = TempDir::new().unwrap();
    let target = backup_dir.path().join("already-here");
    std::fs::create_dir_all(&target).unwrap();

    let err = engine.create_checkpoint(&target).unwrap_err();
    assert!(
        err.to_string().contains("already exists"),
        "expected a clear 'already exists' error, got: {err}"
    );
}

// ============================================================================
// Startup cost: the clean-shutdown marker and the lazy blob-chunk count
// ============================================================================

/// A graceful shutdown lets the next startup trust the persisted counts
/// instead of walking every `doc:` key of every collection.
#[test]
fn test_clean_shutdown_lets_startup_trust_persisted_counts() {
    let tmp_dir = TempDir::new().unwrap();
    let path = tmp_dir.path().to_str().unwrap();

    {
        let engine = StorageEngine::new(path).unwrap();
        engine.initialize().unwrap();
        engine.create_collection("docs".to_string(), None).unwrap();
        let col = engine.get_collection("docs").unwrap();
        for i in 0..7 {
            col.insert(json!({ "num": i })).unwrap();
        }
        // The graceful path: this is what records the marker.
        engine.flush_all_stats();
    }

    {
        let engine = StorageEngine::new(path).unwrap();
        engine.initialize().unwrap();
        assert_eq!(engine.get_collection("docs").unwrap().count(), 7);
    }

    // The marker is consumed by the startup that observed it, so a second
    // reopen with no intervening flush must fall back to the recount — and
    // still land on the same number.
    {
        let engine = StorageEngine::new(path).unwrap();
        engine.initialize().unwrap();
        assert_eq!(engine.get_collection("docs").unwrap().count(), 7);
    }
}

/// Counts survive a startup with no clean-shutdown marker, which is the
/// crash path: `initialize` recounts from the documents themselves.
#[test]
fn test_counts_are_recovered_without_a_clean_shutdown() {
    let tmp_dir = TempDir::new().unwrap();
    let path = tmp_dir.path().to_str().unwrap();

    {
        let engine = StorageEngine::new(path).unwrap();
        engine.initialize().unwrap();
        engine.create_collection("docs".to_string(), None).unwrap();
        let col = engine.get_collection("docs").unwrap();
        for i in 0..5 {
            col.insert(json!({ "num": i })).unwrap();
        }
        // Make the documents durable, but never call flush_all_stats — so no
        // marker is written and the next start must not trust the cache.
        engine.flush().unwrap();
    }

    let engine = StorageEngine::new(path).unwrap();
    engine.initialize().unwrap();
    assert_eq!(engine.get_collection("docs").unwrap().count(), 5);
}

/// `Collection::new` no longer walks `blo:` for every collection; the count
/// is resolved on first use and must agree with an eager walk.
#[test]
fn test_blob_chunk_count_resolves_lazily_after_reopen() {
    let tmp_dir = TempDir::new().unwrap();
    let path = tmp_dir.path().to_str().unwrap();

    {
        let engine = StorageEngine::new(path).unwrap();
        engine.initialize().unwrap();
        engine
            .create_collection("files".to_string(), Some("blob".to_string()))
            .unwrap();
        let files = engine.get_collection("files").unwrap();
        files
            .insert(json!({ "_key": "a", "filename": "a.bin" }))
            .unwrap();
        files.put_blob_chunk("a", 0, b"zero").unwrap();
        files.put_blob_chunk("a", 1, b"one").unwrap();
        files.put_blob_chunk("a", 2, b"two").unwrap();
        assert_eq!(files.chunk_count(), 3);
        engine.flush_all_stats();
    }

    // Fresh handle, never scanned at construction.
    let engine = StorageEngine::new(path).unwrap();
    engine.initialize().unwrap();
    let files = engine.get_collection("files").unwrap();
    assert_eq!(files.chunk_count(), 3);
    assert_eq!(files.blob_stats().unwrap().0, 3);

    // Writing through a handle that has never resolved its count must not
    // double-count: the resolve reads an absolute value from disk.
    files.put_blob_chunk("a", 3, b"three").unwrap();
    assert_eq!(files.chunk_count(), 4);
    assert_eq!(files.blob_stats().unwrap().0, 4);
}

/// The same, for the delete path: a handle that never resolved must not
/// subtract chunks the walk had already excluded.
#[test]
fn test_blob_chunk_count_is_exact_when_deleting_before_first_read() {
    let tmp_dir = TempDir::new().unwrap();
    let path = tmp_dir.path().to_str().unwrap();

    {
        let engine = StorageEngine::new(path).unwrap();
        engine.initialize().unwrap();
        engine
            .create_collection("files".to_string(), Some("blob".to_string()))
            .unwrap();
        let files = engine.get_collection("files").unwrap();
        files
            .insert(json!({ "_key": "a", "filename": "a.bin" }))
            .unwrap();
        files
            .insert(json!({ "_key": "b", "filename": "b.bin" }))
            .unwrap();
        files.put_blob_chunk("a", 0, b"zero").unwrap();
        files.put_blob_chunk("a", 1, b"one").unwrap();
        files.put_blob_chunk("b", 0, b"other").unwrap();
        engine.flush_all_stats();
    }

    let engine = StorageEngine::new(path).unwrap();
    engine.initialize().unwrap();
    let files = engine.get_collection("files").unwrap();

    // Delete is the first operation to touch the count on this handle.
    files.delete_blob_data("a").unwrap();
    assert_eq!(files.chunk_count(), 1);
    assert_eq!(files.blob_stats().unwrap().0, 1);
}

// ============================================================================
// Reusing a doomed column family instead of dropping and recreating it
// ============================================================================

/// Deleting a collection and recreating it under the same name must reuse the
/// column family — the pair of OPTIONS rewrites this avoids is the whole
/// point — and the new incarnation must start empty.
#[test]
fn test_recreating_a_deleted_collection_reuses_its_column_family() {
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();
    let db = engine.get_database("_system").unwrap();

    db.create_collection("churn".to_string(), None).unwrap();
    let coll = db.get_collection("churn").unwrap();
    coll.insert(json!({ "_key": "before", "v": 1 })).unwrap();
    assert_eq!(coll.count(), 1);

    let reuses_before = solidb::storage::cf_ops::reuses();
    db.delete_collection("churn").unwrap();
    db.create_collection("churn".to_string(), None).unwrap();
    assert_eq!(
        solidb::storage::cf_ops::reuses(),
        reuses_before + 1,
        "the recreate should have reused the column family, not rebuilt it"
    );

    // Nothing of the previous incarnation survives.
    let coll = db.get_collection("churn").unwrap();
    assert_eq!(coll.count(), 0);
    assert!(coll.get("before").is_err());
    assert!(coll.all().is_empty());
}

/// A deleted collection is invisible the moment `delete_collection` returns,
/// even though its column family is still on disk awaiting the reaper.
#[test]
fn test_deleted_collection_is_invisible_before_its_drop() {
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();
    let db = engine.get_database("_system").unwrap();

    db.create_collection("gone".to_string(), None).unwrap();
    assert!(db.list_collections().contains(&"gone".to_string()));

    db.delete_collection("gone").unwrap();

    assert!(!db.list_collections().contains(&"gone".to_string()));
    assert!(db.get_collection("gone").is_err());
    // A second delete must report it as already gone, not succeed twice.
    assert!(db.delete_collection("gone").is_err());
}

/// Index definitions must not leak from one incarnation to the next: the
/// column family object is the same, so only the wipe and the cache
/// invalidation keep them apart.
#[test]
fn test_reused_column_family_carries_no_index_definitions() {
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();
    let db = engine.get_database("_system").unwrap();

    db.create_collection("indexed".to_string(), None).unwrap();
    let coll = db.get_collection("indexed").unwrap();
    coll.create_index(
        "by_email".to_string(),
        vec!["email".to_string()],
        solidb::storage::IndexType::Persistent,
        false,
    )
    .unwrap();
    coll.insert(json!({ "_key": "a", "email": "a@example.com" }))
        .unwrap();
    assert!(coll.list_indexes().iter().any(|i| i.name == "by_email"));

    db.delete_collection("indexed").unwrap();
    db.create_collection("indexed".to_string(), None).unwrap();

    let coll = db.get_collection("indexed").unwrap();
    assert!(
        coll.list_indexes().is_empty(),
        "a recreated collection must not inherit the previous index definitions"
    );
    assert_eq!(coll.count(), 0);
}

/// The collection type is re-established on a reused column family rather
/// than inherited from whatever was there before.
#[test]
fn test_reused_column_family_takes_the_new_collection_type() {
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();
    let db = engine.get_database("_system").unwrap();

    db.create_collection("shape".to_string(), Some("edge".to_string()))
        .unwrap();
    assert_eq!(db.get_collection("shape").unwrap().get_type(), "edge");

    db.delete_collection("shape").unwrap();
    db.create_collection("shape".to_string(), Some("document".to_string()))
        .unwrap();
    assert_eq!(db.get_collection("shape").unwrap().get_type(), "document");
}

// ============================================================================
// Collection registry in _meta
// ============================================================================

/// Dropping a database removes its collections' registry entries in the same
/// batch that removes the database itself.
#[test]
fn test_dropping_a_database_deregisters_its_collections() {
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();

    engine.create_database("doomed".to_string()).unwrap();
    let db = engine.get_database("doomed").unwrap();
    db.create_collection("a".to_string(), None).unwrap();
    db.create_collection("b".to_string(), None).unwrap();
    assert_eq!(
        engine.collections_grouped().get("doomed").map(|v| v.len()),
        Some(2)
    );

    engine.delete_database("doomed").unwrap();
    assert!(engine.collections_grouped().get("doomed").is_none());
}

/// A collection deleted through the engine-level path is deregistered too,
/// not only the one on `Database`.
#[test]
fn test_engine_level_delete_deregisters() {
    let (engine, _tmp) = create_test_engine();
    engine.initialize().unwrap();

    engine
        .create_collection("_system:direct".to_string(), None)
        .unwrap();
    let db = engine.get_database("_system").unwrap();
    assert!(db.list_collections().contains(&"direct".to_string()));

    engine.delete_collection("_system:direct").unwrap();
    assert!(!db.list_collections().contains(&"direct".to_string()));
}
