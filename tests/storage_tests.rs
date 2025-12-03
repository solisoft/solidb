//! Storage Engine Tests
//! Tests for RocksDB-backed storage, collections, and documents

use rust_db::{StorageEngine, DbError};
use serde_json::json;
use tempfile::TempDir;

/// Helper to create a test storage engine with a temporary directory
fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

// ==================== Collection Tests ====================

#[test]
fn test_create_collection() {
    let (storage, _dir) = create_test_storage();

    let result = storage.create_collection("users".to_string());
    assert!(result.is_ok());

    let collections = storage.list_collections();
    assert!(collections.contains(&"users".to_string()));
}

#[test]
fn test_create_duplicate_collection_fails() {
    let (storage, _dir) = create_test_storage();

    storage.create_collection("users".to_string()).unwrap();
    let result = storage.create_collection("users".to_string());

    assert!(matches!(result, Err(DbError::CollectionAlreadyExists(_))));
}

#[test]
fn test_get_collection() {
    let (storage, _dir) = create_test_storage();

    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users");

    assert!(collection.is_ok());
    assert_eq!(collection.unwrap().name, "users");
}

#[test]
fn test_get_nonexistent_collection_fails() {
    let (storage, _dir) = create_test_storage();

    let result = storage.get_collection("nonexistent");
    assert!(matches!(result, Err(DbError::CollectionNotFound(_))));
}

#[test]
fn test_delete_collection() {
    let (storage, _dir) = create_test_storage();

    storage.create_collection("users".to_string()).unwrap();
    let result = storage.delete_collection("users");

    assert!(result.is_ok());
    assert!(!storage.list_collections().contains(&"users".to_string()));
}

#[test]
fn test_list_multiple_collections() {
    let (storage, _dir) = create_test_storage();

    storage.create_collection("users".to_string()).unwrap();
    storage.create_collection("products".to_string()).unwrap();
    storage.create_collection("orders".to_string()).unwrap();

    let collections = storage.list_collections();
    assert_eq!(collections.len(), 3);
    assert!(collections.contains(&"users".to_string()));
    assert!(collections.contains(&"products".to_string()));
    assert!(collections.contains(&"orders".to_string()));
}

// ==================== Document Tests ====================

#[test]
fn test_insert_document() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    let doc = collection.insert(json!({
        "name": "Alice",
        "age": 30
    })).unwrap();

    assert!(!doc.key.is_empty());
    assert!(doc.id.starts_with("users/"));
}

#[test]
fn test_insert_document_with_custom_key() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    let doc = collection.insert(json!({
        "_key": "alice",
        "name": "Alice",
        "age": 30
    })).unwrap();

    assert_eq!(doc.key, "alice");
}

#[test]
fn test_get_document() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    let inserted = collection.insert(json!({
        "_key": "alice",
        "name": "Alice",
        "age": 30
    })).unwrap();

    let retrieved = collection.get("alice").unwrap();
    assert_eq!(retrieved.key, inserted.key);
}

#[test]
fn test_get_nonexistent_document_fails() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    let result = collection.get("nonexistent");
    assert!(matches!(result, Err(DbError::DocumentNotFound(_))));
}

#[test]
fn test_update_document() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    collection.insert(json!({
        "_key": "alice",
        "name": "Alice",
        "age": 30
    })).unwrap();

    let updated = collection.update("alice", json!({
        "age": 31,
        "city": "Paris"
    })).unwrap();

    let doc_value = updated.to_value();
    assert_eq!(doc_value["age"], 31);
    assert_eq!(doc_value["city"], "Paris");
    assert_eq!(doc_value["name"], "Alice"); // Original field preserved
}

#[test]
fn test_delete_document() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    collection.insert(json!({
        "_key": "alice",
        "name": "Alice"
    })).unwrap();

    let result = collection.delete("alice");
    assert!(result.is_ok());

    let get_result = collection.get("alice");
    assert!(matches!(get_result, Err(DbError::DocumentNotFound(_))));
}

#[test]
fn test_all_documents() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    collection.insert(json!({"name": "Alice"})).unwrap();
    collection.insert(json!({"name": "Bob"})).unwrap();
    collection.insert(json!({"name": "Charlie"})).unwrap();

    let all = collection.all();
    assert_eq!(all.len(), 3);
}

#[test]
fn test_document_count() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    assert_eq!(collection.count(), 0);

    collection.insert(json!({"name": "Alice"})).unwrap();
    collection.insert(json!({"name": "Bob"})).unwrap();

    assert_eq!(collection.count(), 2);
}

// ==================== Index Tests ====================

#[test]
fn test_create_index() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    let stats = collection.create_index(
        "idx_age".to_string(),
        "age".to_string(),
        rust_db::IndexType::Persistent,
        false
    ).unwrap();

    assert_eq!(stats.name, "idx_age");
    assert_eq!(stats.field, "age");
}

#[test]
fn test_index_lookup_eq() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    // Insert documents first
    collection.insert(json!({"_key": "alice", "name": "Alice", "age": 30})).unwrap();
    collection.insert(json!({"_key": "bob", "name": "Bob", "age": 25})).unwrap();
    collection.insert(json!({"_key": "charlie", "name": "Charlie", "age": 30})).unwrap();

    // Create index
    collection.create_index(
        "idx_age".to_string(),
        "age".to_string(),
        rust_db::IndexType::Hash,
        false
    ).unwrap();

    // Lookup
    let results = collection.index_lookup_eq("age", &json!(30));
    assert!(results.is_some());
    let docs = results.unwrap();
    assert_eq!(docs.len(), 2);
}

#[test]
fn test_list_indexes() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    collection.create_index("idx_age".to_string(), "age".to_string(), rust_db::IndexType::Persistent, false).unwrap();
    collection.create_index("idx_name".to_string(), "name".to_string(), rust_db::IndexType::Hash, false).unwrap();

    let indexes = collection.list_indexes();
    assert_eq!(indexes.len(), 2);
}

#[test]
fn test_drop_index() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();

    collection.create_index("idx_age".to_string(), "age".to_string(), rust_db::IndexType::Persistent, false).unwrap();

    let result = collection.drop_index("idx_age");
    assert!(result.is_ok());

    let indexes = collection.list_indexes();
    assert_eq!(indexes.len(), 0);
}

// ==================== Persistence Tests ====================

#[test]
fn test_data_persists_after_reopen() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path().to_path_buf();

    // Create storage, insert data, drop storage
    {
        let storage = StorageEngine::new(&path).unwrap();
        storage.create_collection("users".to_string()).unwrap();
        let collection = storage.get_collection("users").unwrap();
        collection.insert(json!({
            "_key": "alice",
            "name": "Alice",
            "age": 30
        })).unwrap();
    }

    // Reopen storage and verify data
    {
        let storage = StorageEngine::new(&path).unwrap();
        let collections = storage.list_collections();
        assert!(collections.contains(&"users".to_string()));

        let collection = storage.get_collection("users").unwrap();
        let doc = collection.get("alice").unwrap();
        let value = doc.to_value();
        assert_eq!(value["name"], "Alice");
        assert_eq!(value["age"], 30);
    }
}

#[test]
fn test_index_persists_after_reopen() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path().to_path_buf();

    // Create storage with index
    {
        let storage = StorageEngine::new(&path).unwrap();
        storage.create_collection("users".to_string()).unwrap();
        let collection = storage.get_collection("users").unwrap();
        collection.insert(json!({"_key": "alice", "age": 30})).unwrap();
        collection.create_index("idx_age".to_string(), "age".to_string(), rust_db::IndexType::Persistent, false).unwrap();
    }

    // Reopen and verify index
    {
        let storage = StorageEngine::new(&path).unwrap();
        let collection = storage.get_collection("users").unwrap();
        let indexes = collection.list_indexes();
        assert_eq!(indexes.len(), 1);
        assert_eq!(indexes[0].name, "idx_age");
    }
}

// ==================== Geo Index Tests ====================

#[test]
fn test_create_geo_index() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("places".to_string()).unwrap();
    let collection = storage.get_collection("places").unwrap();

    let stats = collection.create_geo_index(
        "idx_location".to_string(),
        "location".to_string()
    ).unwrap();

    assert_eq!(stats.name, "idx_location");
    assert_eq!(stats.field, "location");
}

#[test]
fn test_geo_near() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("places".to_string()).unwrap();
    let collection = storage.get_collection("places").unwrap();

    // Insert places
    collection.insert(json!({
        "_key": "eiffel",
        "name": "Eiffel Tower",
        "location": {"lat": 48.8584, "lon": 2.2945}
    })).unwrap();
    collection.insert(json!({
        "_key": "louvre",
        "name": "Louvre Museum",
        "location": {"lat": 48.8606, "lon": 2.3376}
    })).unwrap();
    collection.insert(json!({
        "_key": "notre_dame",
        "name": "Notre Dame",
        "location": {"lat": 48.8530, "lon": 2.3499}
    })).unwrap();

    // Create geo index
    collection.create_geo_index("idx_location".to_string(), "location".to_string()).unwrap();

    // Query near Eiffel Tower
    let results = collection.geo_near("location", 48.8584, 2.2945, 3);
    assert!(results.is_some());
    let places = results.unwrap();
    assert!(!places.is_empty());

    // First result should be Eiffel Tower (closest to itself)
    let (first_doc, first_dist) = &places[0];
    assert_eq!(first_doc.key, "eiffel");
    assert!(first_dist < &100.0); // Very close to itself
}

#[test]
fn test_geo_within() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("places".to_string()).unwrap();
    let collection = storage.get_collection("places").unwrap();

    // Insert places
    collection.insert(json!({
        "_key": "eiffel",
        "name": "Eiffel Tower",
        "location": {"lat": 48.8584, "lon": 2.2945}
    })).unwrap();
    collection.insert(json!({
        "_key": "london_eye",
        "name": "London Eye",
        "location": {"lat": 51.5033, "lon": -0.1196}
    })).unwrap();

    // Create geo index
    collection.create_geo_index("idx_location".to_string(), "location".to_string()).unwrap();

    // Query within 10km of Eiffel Tower (should not include London Eye)
    let results = collection.geo_within("location", 48.8584, 2.2945, 10_000.0);
    assert!(results.is_some());
    let places = results.unwrap();
    assert_eq!(places.len(), 1);
    assert_eq!(places[0].0.key, "eiffel");
}

