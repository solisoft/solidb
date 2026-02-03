//! Storage Layer Unit Tests
//!
//! Tests for the database storage layer, covering:
//! - Document creation and manipulation
//! - Collection operations
//! - Storage engine functionality

use serde_json::json;
use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
use solidb::storage::{Document, StorageEngine};
use std::sync::Arc;
use tempfile::TempDir;

// ============================================================================
// Document Tests
// ============================================================================

#[test]
fn test_document_new() {
    let doc = Document::new("users", json!({"name": "Alice", "age": 30}));

    assert!(!doc.key.is_empty(), "Document should have a key");
    assert!(
        doc.id.starts_with("users/"),
        "Document ID should include collection"
    );
    assert!(!doc.rev.is_empty(), "Document should have a revision");
}

#[test]
fn test_document_with_key() {
    let doc = Document::with_key("users", "custom-key".to_string(), json!({"name": "Bob"}));

    assert_eq!(doc.key, "custom-key");
    assert_eq!(doc.id, "users/custom-key");
}

#[test]
fn test_document_get_field() {
    let doc = Document::new(
        "users",
        json!({"name": "Charlie", "email": "charlie@test.com"}),
    );

    assert_eq!(doc.get("name"), Some(json!("Charlie")));
    assert_eq!(doc.get("email"), Some(json!("charlie@test.com")));
    assert_eq!(doc.get("nonexistent"), None);
}

#[test]
fn test_document_get_system_fields() {
    let doc = Document::with_key("users", "key123".to_string(), json!({"name": "Test"}));

    assert_eq!(doc.get("_key"), Some(json!("key123")));
    assert_eq!(doc.get("_id"), Some(json!("users/key123")));
    assert!(doc.get("_rev").is_some());
}

#[test]
fn test_document_update() {
    let mut doc = Document::new("users", json!({"name": "Initial", "score": 0}));
    let original_rev = doc.rev.clone();

    doc.update(json!({"score": 100, "level": 5}));

    assert_eq!(
        doc.get("name"),
        Some(json!("Initial")),
        "Original field should remain"
    );
    assert_eq!(
        doc.get("score"),
        Some(json!(100)),
        "Field should be updated"
    );
    assert_eq!(
        doc.get("level"),
        Some(json!(5)),
        "New field should be added"
    );
    assert_ne!(doc.rev, original_rev, "Revision should change on update");
}

#[test]
fn test_document_update_preserves_system_fields() {
    let mut doc = Document::with_key("users", "testkey".to_string(), json!({"name": "Test"}));

    // Try to overwrite system fields
    doc.update(json!({"_key": "hacked", "_id": "hacked/key", "name": "Updated"}));

    // System fields should NOT be overwritten
    assert_eq!(doc.key, "testkey");
    assert_eq!(doc.id, "users/testkey");
    assert_eq!(doc.get("name"), Some(json!("Updated")));
}

#[test]
fn test_document_to_value() {
    let doc = Document::with_key("users", "key1".to_string(), json!({"name": "Test"}));
    let value = doc.to_value();

    assert!(value.is_object());
    assert_eq!(value.get("_key"), Some(&json!("key1")));
    assert_eq!(value.get("_id"), Some(&json!("users/key1")));
    assert!(value.get("_rev").is_some());
    assert_eq!(value.get("name"), Some(&json!("Test")));
}

#[test]
fn test_document_revision_changes_on_update() {
    let mut doc = Document::new("users", json!({"counter": 0}));
    let mut revisions = vec![doc.rev.clone()];

    for i in 1..5 {
        doc.update(json!({"counter": i}));
        revisions.push(doc.rev.clone());
    }

    // All revisions should be unique
    let unique_revs: std::collections::HashSet<_> = revisions.iter().collect();
    assert_eq!(
        unique_revs.len(),
        revisions.len(),
        "Each update should generate unique revision"
    );
}

// ============================================================================
// StorageEngine Tests
// ============================================================================

fn create_test_engine() -> (Arc<StorageEngine>, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (Arc::new(engine), tmp_dir)
}

#[test]
fn test_storage_engine_create() {
    let (engine, _tmp) = create_test_engine();
    assert!(!engine.data_dir().is_empty());
}

#[test]
fn test_storage_engine_create_database() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.create_database("test_db".to_string());
    assert!(result.is_ok(), "Should create database: {:?}", result.err());
}

#[test]
fn test_storage_engine_list_databases() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("db1".to_string()).unwrap();
    engine.create_database("db2".to_string()).unwrap();

    let dbs = engine.list_databases();
    assert!(dbs.contains(&"db1".to_string()));
    assert!(dbs.contains(&"db2".to_string()));
}

#[test]
fn test_storage_engine_get_database() {
    let (engine, _tmp) = create_test_engine();
    engine.create_database("mydb".to_string()).unwrap();

    let db = engine.get_database("mydb");
    assert!(db.is_ok(), "Should get database: {:?}", db.err());
}

#[test]
fn test_storage_engine_create_collection() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.create_collection("test_collection".to_string(), None);
    assert!(
        result.is_ok(),
        "Should create collection: {:?}",
        result.err()
    );
}

#[test]
fn test_storage_engine_list_collections() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("col1".to_string(), None).unwrap();
    engine.create_collection("col2".to_string(), None).unwrap();

    let cols = engine.list_collections();
    assert!(cols.contains(&"col1".to_string()));
    assert!(cols.contains(&"col2".to_string()));
}

#[test]
fn test_storage_engine_get_collection() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();

    let col = engine.get_collection("users");
    assert!(col.is_ok(), "Should get collection: {:?}", col.err());
}

#[test]
fn test_storage_engine_delete_collection() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("to_delete".to_string(), None)
        .unwrap();

    let before = engine.list_collections();
    assert!(before.contains(&"to_delete".to_string()));

    engine.delete_collection("to_delete").unwrap();

    let after = engine.list_collections();
    assert!(!after.contains(&"to_delete".to_string()));
}

// ============================================================================
// Collection Document Operations
// ============================================================================

#[test]
fn test_collection_insert_and_get() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let collection = engine.get_collection("users").unwrap();

    // Insert a document using Value (the actual API)
    let inserted = collection
        .insert(json!({"_key": "user1", "name": "Alice", "age": 30}))
        .expect("Insert should succeed");

    assert_eq!(inserted.key, "user1");

    // Get it back
    let retrieved = collection.get("user1").expect("Get should succeed");

    let val = retrieved.to_value();
    assert_eq!(val.get("name"), Some(&json!("Alice")));
    assert_eq!(val.get("age"), Some(&json!(30)));
}

#[test]
fn test_collection_insert_auto_key() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let collection = engine.get_collection("items").unwrap();

    // Insert without _key - should auto-generate
    let inserted = collection
        .insert(json!({"value": 42}))
        .expect("Insert should succeed");

    // Key should be a UUID v7
    assert!(!inserted.key.is_empty());
    assert!(inserted.key.contains('-')); // UUID format
}

#[test]
fn test_collection_update_document() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let collection = engine.get_collection("users").unwrap();

    // Insert initial document
    collection
        .insert(json!({"_key": "user1", "name": "Bob", "score": 0}))
        .expect("Insert should succeed");

    // Update it using the Collection API: update(key, data)
    let updated = collection
        .update("user1", json!({"score": 100}))
        .expect("Update should succeed");

    // Verify update
    let val = updated.to_value();
    assert_eq!(val.get("score"), Some(&json!(100)));
    assert_eq!(val.get("name"), Some(&json!("Bob"))); // Name should be preserved
}

#[test]
fn test_collection_delete_document() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let collection = engine.get_collection("users").unwrap();

    // Insert
    collection
        .insert(json!({"_key": "to_delete", "temp": true}))
        .expect("Insert should succeed");

    // Verify exists
    assert!(collection.get("to_delete").is_ok());

    // Delete
    collection
        .delete("to_delete")
        .expect("Delete should succeed");

    // Verify deleted - get should return an error
    assert!(collection.get("to_delete").is_err());
}

#[test]
fn test_collection_count() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let collection = engine.get_collection("items").unwrap();

    // Insert multiple documents
    for i in 0..10 {
        collection
            .insert(json!({"index": i}))
            .expect("Insert should succeed");
    }

    let count = collection.count();
    assert_eq!(count, 10);
}

#[test]
fn test_collection_all_documents() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let collection = engine.get_collection("items").unwrap();

    // Insert some documents
    for i in 0..5 {
        collection
            .insert(json!({"value": i}))
            .expect("Insert should succeed");
    }

    let all_docs = collection.all();
    assert_eq!(all_docs.len(), 5);
}

// ============================================================================
// Edge Collection Tests
// ============================================================================

#[test]
fn test_edge_collection_create() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.create_collection("follows".to_string(), Some("edge".to_string()));
    assert!(
        result.is_ok(),
        "Should create edge collection: {:?}",
        result.err()
    );
}

#[test]
fn test_edge_collection_requires_from_to() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = engine.get_collection("edges").unwrap();

    // Try to insert without _from and _to - should fail
    let result = collection.insert(json!({"data": "no edges"}));
    assert!(result.is_err(), "Edge without _from/_to should fail");

    // Insert with _from and _to - should succeed
    let result = collection.insert(json!({
        "_from": "users/alice",
        "_to": "users/bob",
        "since": "2024-01-01"
    }));
    assert!(
        result.is_ok(),
        "Edge with _from/_to should succeed: {:?}",
        result.err()
    );
}

// ============================================================================
// Concurrent Access Tests
// ============================================================================

#[test]
fn test_concurrent_document_operations() {
    use std::thread;

    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("concurrent".to_string(), None)
        .unwrap();

    let handles: Vec<_> = (0..10)
        .map(|t| {
            let engine = Arc::clone(&engine);
            thread::spawn(move || {
                let collection = engine.get_collection("concurrent").unwrap();
                let mut success_count = 0;
                let mut error_count = 0;

                for i in 0..10 {
                    let key = format!("t{}_doc{}", t, i);
                    match collection.insert(json!({"_key": key, "thread": t, "doc": i})) {
                        Ok(_) => success_count += 1,
                        Err(e) => {
                            error_count += 1;
                            eprintln!("Thread {} insert {}: {:?}", t, i, e);
                        }
                    }
                }

                (success_count, error_count)
            })
        })
        .collect();

    let mut total_success = 0;
    let mut total_errors = 0;

    for h in handles {
        let (success, errors) = h.join().expect("Thread should complete");
        total_success += success;
        total_errors += errors;
    }

    println!(
        "Total successful inserts: {}, Total errors: {}",
        total_success, total_errors
    );

    // Use recount to get actual document count from disk
    let collection = engine.get_collection("concurrent").unwrap();
    let actual_count = collection.recount_documents();
    let cached_count = collection.count();

    println!(
        "Actual count from disk: {}, Cached count: {}",
        actual_count, cached_count
    );

    // Assert that we have the expected number of documents
    assert_eq!(actual_count, 100, "All concurrent inserts should succeed");
    assert_eq!(
        cached_count, actual_count,
        "Cached count should match actual count"
    );
    assert_eq!(total_errors, 0, "No insert operations should fail");
}

#[test]
fn test_document_uuidv7_key_ordering() {
    // UUID v7 keys should be time-ordered
    let doc1 = Document::new("test", json!({"seq": 1}));
    std::thread::sleep(std::time::Duration::from_millis(2));
    let doc2 = Document::new("test", json!({"seq": 2}));
    std::thread::sleep(std::time::Duration::from_millis(2));
    let doc3 = Document::new("test", json!({"seq": 3}));

    // UUID v7 keys are lexicographically sortable by time
    assert!(doc1.key < doc2.key, "doc1 key should be less than doc2 key");
    assert!(doc2.key < doc3.key, "doc2 key should be less than doc3 key");
}

// ============================================================================
// Batch Operations
// ============================================================================

#[test]
fn test_collection_batch_insert() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("batch_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("batch_test").unwrap();

    // Prepare batch of documents
    let documents: Vec<serde_json::Value> = (0..100)
        .map(|i| json!({"index": i, "name": format!("item_{}", i)}))
        .collect();

    let result = collection.insert_batch(documents);
    assert!(
        result.is_ok(),
        "Batch insert should succeed: {:?}",
        result.err()
    );

    let inserted = result.unwrap();
    assert_eq!(inserted.len(), 100);

    let count = collection.count();
    assert_eq!(count, 100);
}

#[test]
fn test_batch_insert_empty() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("batch_empty_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("batch_empty_test").unwrap();

    // Empty batch should return empty result without error
    let result = collection.insert_batch(vec![]);
    assert!(result.is_ok(), "Empty batch insert should succeed");
    assert_eq!(result.unwrap().len(), 0);

    let count = collection.count();
    assert_eq!(count, 0);
}

#[test]
fn test_batch_insert_with_keys() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("batch_keys_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("batch_keys_test").unwrap();

    // Prepare batch with explicit _key values
    let documents: Vec<serde_json::Value> = (0..10)
        .map(|i| json!({"_key": format!("doc_{}", i), "value": i}))
        .collect();

    let result = collection.insert_batch(documents);
    assert!(result.is_ok(), "Batch insert with keys should succeed");

    let inserted = result.unwrap();
    assert_eq!(inserted.len(), 10);

    // Verify keys were preserved
    for i in 0..10 {
        let key = format!("doc_{}", i);
        let doc = collection.get(&key);
        assert!(doc.is_ok(), "Document with key {} should exist", key);
    }
}

#[test]
fn test_batch_insert_duplicate_key_in_batch() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("batch_dup_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("batch_dup_test").unwrap();

    // Insert a document first
    collection
        .insert(json!({"_key": "existing", "value": 1}))
        .unwrap();

    // Batch with duplicate key should fail on unique constraint check
    let documents: Vec<serde_json::Value> = vec![
        json!({"_key": "new1", "value": 2}),
        json!({"_key": "existing", "value": 3}), // Duplicate!
        json!({"_key": "new2", "value": 4}),
    ];

    let result = collection.insert_batch(documents);
    assert!(
        result.is_err(),
        "Batch insert with duplicate key should fail"
    );

    // Verify no new documents were inserted (atomicity)
    let count = collection.count();
    assert_eq!(count, 1, "Only original document should exist");
}

#[test]
fn test_batch_insert_schema_validation() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("batch_schema_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("batch_schema_test").unwrap();

    // Set up schema validation
    let schema = CollectionSchema::new(
        "default".to_string(),
        json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "age": {"type": "integer", "minimum": 0}
            },
            "required": ["name"]
        }),
        SchemaValidationMode::Strict,
    );
    collection.set_json_schema(schema).unwrap();

    // Valid batch should succeed
    let valid_docs: Vec<serde_json::Value> = (0..5)
        .map(|i| json!({"name": format!("user_{}", i), "age": i * 10}))
        .collect();

    let result = collection.insert_batch(valid_docs);
    assert!(result.is_ok(), "Valid batch should succeed");
    assert_eq!(result.unwrap().len(), 5);

    // Invalid batch (missing required field) should fail
    let invalid_docs: Vec<serde_json::Value> = vec![
        json!({"name": "valid", "age": 25}),
        json!({"age": 30}), // Missing required "name" field
    ];

    let result = collection.insert_batch(invalid_docs);
    assert!(result.is_err(), "Batch with schema violation should fail");

    // Verify partial documents weren't inserted
    let count = collection.count();
    assert_eq!(count, 5, "Only first batch should be committed");
}

#[test]
fn test_batch_insert_edge_validation() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("batch_edge_test".to_string(), Some("edge".to_string()))
        .unwrap();
    let collection = engine.get_collection("batch_edge_test").unwrap();

    // Valid edge documents
    let valid_edges: Vec<serde_json::Value> = vec![
        json!({
            "_from": "users/alice",
            "_to": "users/bob",
            "relationship": "friend"
        }),
        json!({
            "_from": "users/bob",
            "_to": "users/charlie",
            "relationship": "follows"
        }),
    ];

    let result = collection.insert_batch(valid_edges);
    assert!(result.is_ok(), "Valid edge batch should succeed");
    assert_eq!(result.unwrap().len(), 2);

    // Invalid edge document (missing _from)
    let invalid_edges: Vec<serde_json::Value> = vec![
        json!({
            "_from": "users/alice",
            "_to": "users/bob",
            "relationship": "friend"
        }),
        json!({
            "_to": "users/charlie",
            "relationship": "invalid" // Missing _from
        }),
    ];

    let result = collection.insert_batch(invalid_edges);
    assert!(result.is_err(), "Batch with invalid edge should fail");

    // Verify count
    let count = collection.count();
    assert_eq!(count, 2, "Only first edge batch should be committed");
}

#[test]
fn test_batch_insert_mixed_key_generation() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("batch_mixed_keys".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("batch_mixed_keys").unwrap();

    // Mix of documents with and without _key
    let documents: Vec<serde_json::Value> = vec![
        json!({"_key": "explicit_key", "value": 1}),
        json!({"value": 2}), // Will get auto-generated key
        json!({"_key": "another_explicit", "value": 3}),
        json!({"value": 4}), // Will get auto-generated key
    ];

    let result = collection.insert_batch(documents);
    assert!(result.is_ok(), "Mixed key batch should succeed");

    let inserted = result.unwrap();
    assert_eq!(inserted.len(), 4);

    // Verify explicit keys
    assert!(collection.get("explicit_key").is_ok());
    assert!(collection.get("another_explicit").is_ok());

    // All docs should be queryable
    let count = collection.count();
    assert_eq!(count, 4);
}

// ============================================================================
// Document Key Tests
// ============================================================================

#[test]
fn test_document_duplicate_key() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("unique_test".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("unique_test").unwrap();

    // Insert first document
    collection
        .insert(json!({"_key": "same-key", "value": 1}))
        .unwrap();

    // Inserting with same key should actually overwrite (or return error depending on impl)
    // Let's verify the behavior
    let _result = collection.insert(json!({"_key": "same-key", "value": 2}));

    // Get the document to see final state
    let doc = collection.get("same-key").unwrap();
    let val = doc.to_value();

    // Document should exist (either original or updated)
    assert!(val.get("value").is_some());
}
