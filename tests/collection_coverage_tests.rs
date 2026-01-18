//! Collection Coverage Tests
//!
//! Additional comprehensive tests for storage/collection.rs including:
//! - Index operations
//! - Blob operations
//! - Scan and query operations
//! - Statistics and disk usage
//! - Edge cases

use serde_json::json;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_engine() -> (Arc<StorageEngine>, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (Arc::new(engine), tmp_dir)
}

// ============================================================================
// Index Operations Tests
// ============================================================================

#[test]
fn test_create_index() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("indexed".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("indexed").unwrap();

    // Create index
    let stats = collection
        .create_index(
            "email_idx".to_string(),
            vec!["email".to_string()],
            solidb::storage::IndexType::Persistent,
            false, // not unique
        )
        .expect("Index creation should succeed");

    assert_eq!(stats.name, "email_idx");
}

#[test]
fn test_create_unique_index() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("unique_indexed".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("unique_indexed").unwrap();

    // Create unique index
    let stats = collection
        .create_index(
            "id_idx".to_string(),
            vec!["user_id".to_string()],
            solidb::storage::IndexType::Persistent,
            true, // unique
        )
        .expect("Unique index creation should succeed");

    assert!(stats.unique);
}

#[test]
fn test_list_indexes() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("multi_indexed".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("multi_indexed").unwrap();

    // Create multiple indexes
    collection
        .create_index(
            "idx1".to_string(),
            vec!["field1".to_string()],
            solidb::storage::IndexType::Persistent,
            false,
        )
        .unwrap();

    collection
        .create_index(
            "idx2".to_string(),
            vec!["field2".to_string()],
            solidb::storage::IndexType::Persistent,
            false,
        )
        .unwrap();

    let indexes = collection.list_indexes();
    assert!(indexes.len() >= 2);

    let names: Vec<_> = indexes.iter().map(|i| i.name.as_str()).collect();
    assert!(names.contains(&"idx1"));
    assert!(names.contains(&"idx2"));
}

#[test]
fn test_drop_index() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("drop_indexed".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("drop_indexed").unwrap();

    // Create index
    collection
        .create_index(
            "to_drop".to_string(),
            vec!["temp".to_string()],
            solidb::storage::IndexType::Persistent,
            false,
        )
        .unwrap();

    // Verify it exists
    let before = collection.list_indexes();
    assert!(before.iter().any(|i| i.name == "to_drop"));

    // Drop index
    collection.drop_index("to_drop").unwrap();

    // Verify it's gone
    let after = collection.list_indexes();
    assert!(!after.iter().any(|i| i.name == "to_drop"));
}

#[test]
fn test_index_lookup() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("lookup_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("lookup_test").unwrap();

    // Create index
    collection
        .create_index(
            "status_idx".to_string(),
            vec!["status".to_string()],
            solidb::storage::IndexType::Persistent,
            false,
        )
        .unwrap();

    // Insert documents
    collection
        .insert(json!({"_key": "doc1", "status": "active", "name": "Item 1"}))
        .unwrap();
    collection
        .insert(json!({"_key": "doc2", "status": "inactive", "name": "Item 2"}))
        .unwrap();
    collection
        .insert(json!({"_key": "doc3", "status": "active", "name": "Item 3"}))
        .unwrap();

    // Lookup by index
    let results = collection.index_lookup_eq("status", &json!("active"));
    assert!(results.is_some());
    let docs = results.unwrap();
    assert_eq!(docs.len(), 2);
}

#[test]
fn test_compound_index() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("compound".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("compound").unwrap();

    // Create compound index
    let stats = collection
        .create_index(
            "compound_idx".to_string(),
            vec!["first_name".to_string(), "last_name".to_string()],
            solidb::storage::IndexType::Persistent,
            false,
        )
        .expect("Compound index creation should succeed");

    assert_eq!(stats.fields.len(), 2);
}

#[test]
fn test_rebuild_indexes() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("rebuild_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("rebuild_test").unwrap();

    // Insert documents without indexing
    for i in 0..10 {
        collection
            .insert_no_index(json!({"name": format!("item{}", i), "value": i}))
            .unwrap();
    }

    // Create index
    collection
        .create_index(
            "name_idx".to_string(),
            vec!["name".to_string()],
            solidb::storage::IndexType::Persistent,
            false,
        )
        .unwrap();

    // Rebuild indexes
    let count = collection
        .rebuild_all_indexes()
        .expect("Rebuild should succeed");
    assert!(count > 0);
}

// ============================================================================
// Scan and Query Operations Tests
// ============================================================================

#[test]
fn test_scan_with_limit() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("scan_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("scan_test").unwrap();

    // Insert 100 documents
    for i in 0..100 {
        collection.insert(json!({"index": i})).unwrap();
    }

    // Scan with limit
    let docs = collection.scan(Some(10));
    assert_eq!(docs.len(), 10);
}

#[test]
fn test_scan_all() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("scan_all".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("scan_all").unwrap();

    // Insert documents
    for i in 0..50 {
        collection.insert(json!({"num": i})).unwrap();
    }

    // Scan all (no limit)
    let docs = collection.scan(None);
    assert_eq!(docs.len(), 50);
}

#[test]
fn test_get_many() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("getmany".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("getmany").unwrap();

    // Insert documents
    collection.insert(json!({"_key": "a", "value": 1})).unwrap();
    collection.insert(json!({"_key": "b", "value": 2})).unwrap();
    collection.insert(json!({"_key": "c", "value": 3})).unwrap();

    // Get many
    let keys = vec!["a".to_string(), "c".to_string()];
    let docs = collection.get_many(&keys);

    assert_eq!(docs.len(), 2);
}

#[test]
fn test_get_many_with_missing() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("getmany_missing".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("getmany_missing").unwrap();

    collection
        .insert(json!({"_key": "exists", "value": 1}))
        .unwrap();

    // Get many including missing key
    let keys = vec!["exists".to_string(), "missing".to_string()];
    let docs = collection.get_many(&keys);

    // Should only return existing document
    assert_eq!(docs.len(), 1);
}

// ============================================================================
// Statistics Tests
// ============================================================================

#[test]
fn test_collection_stats() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("stats_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("stats_test").unwrap();

    // Insert documents
    for i in 0..20 {
        collection
            .insert(json!({"data": format!("item_{}", i)}))
            .unwrap();
    }

    let stats = collection.stats();
    assert_eq!(stats.document_count, 20);
}

#[test]
fn test_recount_documents() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("recount_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("recount_test").unwrap();

    // Insert documents
    for i in 0..15 {
        collection.insert(json!({"num": i})).unwrap();
    }

    let count = collection.recount_documents();
    assert_eq!(count, 15);
    assert_eq!(collection.count(), 15);
}

#[test]
fn test_disk_usage() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("disk_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("disk_test").unwrap();

    // Insert some data
    for i in 0..10 {
        collection
            .insert(json!({"data": "x".repeat(1000), "index": i}))
            .unwrap();
    }

    let _usage = collection.disk_usage();
    // DiskUsage has sst_files_size, live_data_size, etc.
    // Note: usize values are always >= 0, so this assertion is redundant
}

// ============================================================================
// Truncate Tests
// ============================================================================

#[test]
fn test_truncate_collection() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("truncate_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("truncate_test").unwrap();

    // Insert documents
    for i in 0..50 {
        collection.insert(json!({"num": i})).unwrap();
    }
    assert_eq!(collection.count(), 50);

    // Truncate
    let deleted = collection.truncate().expect("Truncate should succeed");
    assert_eq!(deleted, 50);
    assert_eq!(collection.count(), 0);
}

#[test]
fn test_truncate_empty_collection() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("truncate_empty".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("truncate_empty").unwrap();

    let deleted = collection
        .truncate()
        .expect("Truncate empty should succeed");
    assert_eq!(deleted, 0);
}

// ============================================================================
// Batch Delete Tests
// ============================================================================

#[test]
fn test_batch_delete() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("batch_del".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("batch_del").unwrap();

    // Insert documents
    for i in 0..10 {
        collection
            .insert(json!({"_key": format!("doc{}", i), "value": i}))
            .unwrap();
    }

    // Delete half
    let keys: Vec<String> = (0..5).map(|i| format!("doc{}", i)).collect();
    let deleted = collection
        .delete_batch(keys)
        .expect("Batch delete should succeed");

    assert_eq!(deleted, 5);
    assert_eq!(collection.count(), 5);
}

// ============================================================================
// Update with Revision Tests
// ============================================================================

#[test]
fn test_update_with_correct_revision() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("rev_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("rev_test").unwrap();

    // Insert document
    let doc = collection
        .insert(json!({"_key": "doc1", "value": 1}))
        .unwrap();
    let rev = doc.rev.clone();

    // Update with correct revision
    let updated = collection.update_with_rev("doc1", &rev, json!({"value": 2}));
    assert!(updated.is_ok());
}

#[test]
fn test_update_with_wrong_revision() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("rev_wrong".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("rev_wrong").unwrap();

    // Insert document
    collection
        .insert(json!({"_key": "doc1", "value": 1}))
        .unwrap();

    // Update with wrong revision
    let result = collection.update_with_rev("doc1", "wrong-revision", json!({"value": 2}));
    assert!(result.is_err());
}

// ============================================================================
// Edge Collection Tests
// ============================================================================

#[test]
fn test_edge_collection_validation() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = engine.get_collection("edges").unwrap();

    // Valid edge
    let result = collection.insert(json!({
        "_from": "users/alice",
        "_to": "users/bob",
        "relationship": "follows"
    }));
    assert!(result.is_ok());

    // Invalid edge (missing _from)
    let result = collection.insert(json!({
        "_to": "users/bob",
        "data": "test"
    }));
    assert!(result.is_err());

    // Invalid edge (missing _to)
    let result = collection.insert(json!({
        "_from": "users/alice",
        "data": "test"
    }));
    assert!(result.is_err());
}

#[test]
fn test_edge_collection_type() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("typed_edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = engine.get_collection("typed_edges").unwrap();

    assert_eq!(collection.get_type(), "edge");
}

// ============================================================================
// Blob Operations Tests
// ============================================================================

#[test]
fn test_blob_put_and_get() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("blobs".to_string(), Some("blob".to_string()))
        .unwrap();
    let collection = engine.get_collection("blobs").unwrap();

    // Store blob chunks
    let data = b"Hello, World! This is some binary data.";
    collection
        .put_blob_chunk("myblob", 0, data)
        .expect("Put blob should succeed");

    // Retrieve
    let retrieved = collection
        .get_blob_chunk("myblob", 0)
        .expect("Get blob should succeed");

    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap(), data.to_vec());
}

#[test]
fn test_blob_multiple_chunks() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("multi_blobs".to_string(), Some("blob".to_string()))
        .unwrap();
    let collection = engine.get_collection("multi_blobs").unwrap();

    // Store multiple chunks
    for i in 0..5 {
        let data = format!("Chunk {} data", i);
        collection
            .put_blob_chunk("bigfile", i, data.as_bytes())
            .unwrap();
    }

    // Retrieve all chunks
    for i in 0..5 {
        let chunk = collection.get_blob_chunk("bigfile", i).unwrap();
        assert!(chunk.is_some());
    }
}

#[test]
fn test_blob_delete() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("del_blobs".to_string(), Some("blob".to_string()))
        .unwrap();
    let collection = engine.get_collection("del_blobs").unwrap();

    // Store blob
    collection.put_blob_chunk("todelete", 0, b"data").unwrap();
    collection
        .put_blob_chunk("todelete", 1, b"more data")
        .unwrap();

    // Delete all blob data
    collection
        .delete_blob_data("todelete")
        .expect("Delete blob should succeed");

    // Verify deleted
    let chunk = collection.get_blob_chunk("todelete", 0).unwrap();
    assert!(chunk.is_none());
}

// ============================================================================
// Upsert Batch Tests
// ============================================================================

#[test]
fn test_upsert_batch_insert() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("upsert_ins".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("upsert_ins").unwrap();

    // Upsert (should insert)
    let docs: Vec<(String, serde_json::Value)> = (0..10)
        .map(|i| (format!("doc{}", i), json!({"value": i})))
        .collect();

    let count = collection
        .upsert_batch(docs)
        .expect("Upsert should succeed");
    assert_eq!(count, 10);
    assert_eq!(collection.count(), 10);
}

#[test]
fn test_upsert_batch_update() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("upsert_upd".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("upsert_upd").unwrap();

    // Insert first
    collection
        .insert(json!({"_key": "doc1", "value": 1}))
        .unwrap();

    // Upsert (should update)
    let docs = vec![
        ("doc1".to_string(), json!({"value": 100})),
        ("doc2".to_string(), json!({"value": 2})),
    ];

    let count = collection
        .upsert_batch(docs)
        .expect("Upsert should succeed");
    assert_eq!(count, 2);

    // Verify update
    let doc = collection.get("doc1").unwrap();
    assert_eq!(doc.get("value"), Some(json!(100)));
}

// ============================================================================
// Edge Cases Tests
// ============================================================================

#[test]
fn test_special_characters_in_key() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("special".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("special").unwrap();

    // Keys with special characters
    let keys = vec!["user:123", "item-456", "doc_789", "a.b.c"];

    for key in keys {
        collection
            .insert(json!({"_key": key, "data": "test"}))
            .unwrap();
        let doc = collection.get(key);
        assert!(doc.is_ok(), "Key '{}' should work", key);
    }
}

#[test]
fn test_unicode_in_document() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("unicode".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("unicode").unwrap();

    let doc = collection
        .insert(json!({
            "_key": "unicode_test",
            "japanese": "日本語",
            "emoji": "🎉🚀",
            "arabic": "مرحبا"
        }))
        .unwrap();

    let retrieved = collection.get(&doc.key).unwrap();
    assert_eq!(retrieved.get("japanese"), Some(json!("日本語")));
    assert_eq!(retrieved.get("emoji"), Some(json!("🎉🚀")));
}

#[test]
fn test_deeply_nested_document() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("nested".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("nested").unwrap();

    let doc = collection
        .insert(json!({
            "_key": "nested",
            "level1": {
                "level2": {
                    "level3": {
                        "level4": {
                            "value": "deep"
                        }
                    }
                }
            }
        }))
        .unwrap();

    let retrieved = collection.get(&doc.key).unwrap();
    let val = retrieved.to_value();
    let deep = val["level1"]["level2"]["level3"]["level4"]["value"].as_str();
    assert_eq!(deep, Some("deep"));
}

#[test]
fn test_array_in_document() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("arrays".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("arrays").unwrap();

    let doc = collection
        .insert(json!({
            "_key": "arrays",
            "numbers": [1, 2, 3, 4, 5],
            "nested": [{"a": 1}, {"b": 2}]
        }))
        .unwrap();

    let retrieved = collection.get(&doc.key).unwrap();
    let val = retrieved.to_value();
    assert!(val["numbers"].is_array());
    assert_eq!(val["numbers"].as_array().unwrap().len(), 5);
}
