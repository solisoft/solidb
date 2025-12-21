use serde_json::json;
use solidb::StorageEngine;
use tempfile::TempDir;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn test_ttl_index_creation() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");

    storage.create_collection("sessions".to_string(), None).unwrap();
    let collection = storage.get_collection("sessions").unwrap();

    // Create TTL index
    let stats = collection
        .create_ttl_index(
            "session_expiry".to_string(),
            "expires_at".to_string(),
            3600, // 1 hour
        )
        .unwrap();

    assert_eq!(stats.name, "session_expiry");
    assert_eq!(stats.field, "expires_at");
    assert_eq!(stats.expire_after_seconds, 3600);

    // Verify index is listed
    let indexes = collection.list_ttl_indexes();
    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0].name, "session_expiry");
}

#[test]
fn test_ttl_index_duplicate_prevention() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");

    storage.create_collection("cache".to_string(), None).unwrap();
    let collection = storage.get_collection("cache").unwrap();

    // Create first TTL index
    collection
        .create_ttl_index("ttl_idx".to_string(), "created_at".to_string(), 60)
        .unwrap();

    // Try to create duplicate - should fail
    let result = collection.create_ttl_index("ttl_idx".to_string(), "other_field".to_string(), 120);
    assert!(result.is_err());
}

#[test]
fn test_ttl_index_drop() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");

    storage.create_collection("temp".to_string(), None).unwrap();
    let collection = storage.get_collection("temp").unwrap();

    // Create and drop TTL index
    collection
        .create_ttl_index("temp_ttl".to_string(), "timestamp".to_string(), 300)
        .unwrap();

    assert_eq!(collection.list_ttl_indexes().len(), 1);

    collection.drop_ttl_index("temp_ttl").unwrap();
    assert_eq!(collection.list_ttl_indexes().len(), 0);

    // Dropping non-existent should fail
    let result = collection.drop_ttl_index("temp_ttl");
    assert!(result.is_err());
}

#[test]
fn test_ttl_cleanup_expired_documents() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");

    storage.create_collection("tokens".to_string(), None).unwrap();
    let collection = storage.get_collection("tokens").unwrap();

    // Create TTL index with 0 second expiry (immediate)
    collection
        .create_ttl_index("token_expiry".to_string(), "created_at".to_string(), 0)
        .unwrap();

    // Get current timestamp and create an expired document
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Insert an expired document (timestamp in the past)
    collection.insert(json!({
        "token": "abc123",
        "created_at": now - 10 // 10 seconds ago, with 0 expiry = expired
    })).unwrap();

    // Insert a non-expired document (timestamp in the future)
    collection.insert(json!({
        "token": "def456",
        "created_at": now + 3600 // 1 hour from now
    })).unwrap();

    // Verify we have 2 documents
    assert_eq!(collection.count(), 2);

    // Run cleanup
    let deleted = collection.cleanup_all_expired_documents().unwrap();
    assert_eq!(deleted, 1, "Should have deleted 1 expired document");

    // Verify only 1 document remains
    assert_eq!(collection.count(), 1);

    // The remaining document should be the non-expired one
    let docs = collection.all();
    assert_eq!(docs[0].to_value()["token"], "def456");
}

#[test]
fn test_ttl_cleanup_with_expire_after_seconds() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");

    storage.create_collection("sessions".to_string(), None).unwrap();
    let collection = storage.get_collection("sessions").unwrap();

    // Create TTL index with 60 second expiry
    collection
        .create_ttl_index("session_ttl".to_string(), "last_access".to_string(), 60)
        .unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Insert document accessed 100 seconds ago (expired: 100 > 60)
    collection.insert(json!({
        "user": "alice",
        "last_access": now - 100
    })).unwrap();

    // Insert document accessed 30 seconds ago (not expired: 30 < 60)
    collection.insert(json!({
        "user": "bob",
        "last_access": now - 30
    })).unwrap();

    // Insert document accessed now (not expired: 0 < 60)
    collection.insert(json!({
        "user": "charlie",
        "last_access": now
    })).unwrap();

    assert_eq!(collection.count(), 3);

    // Cleanup should remove alice's session
    let deleted = collection.cleanup_all_expired_documents().unwrap();
    assert_eq!(deleted, 1);

    assert_eq!(collection.count(), 2);

    // Verify remaining users
    let docs = collection.all();
    let users: Vec<String> = docs.iter()
        .filter_map(|d| d.to_value()["user"].as_str().map(|s| s.to_string()))
        .collect();
    assert!(users.contains(&"bob".to_string()));
    assert!(users.contains(&"charlie".to_string()));
    assert!(!users.contains(&"alice".to_string()));
}

#[test]
fn test_ttl_no_cleanup_without_timestamp_field() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");

    storage.create_collection("data".to_string(), None).unwrap();
    let collection = storage.get_collection("data").unwrap();

    // Create TTL index on a field
    collection
        .create_ttl_index("data_ttl".to_string(), "expires_at".to_string(), 0)
        .unwrap();

    // Insert document WITHOUT the expires_at field
    collection.insert(json!({
        "name": "test",
        "value": 123
    })).unwrap();

    // Cleanup should not delete documents without the timestamp field
    let deleted = collection.cleanup_all_expired_documents().unwrap();
    assert_eq!(deleted, 0);
    assert_eq!(collection.count(), 1);
}
