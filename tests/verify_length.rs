use serde_json::json;
use solidb::{parse, QueryExecutor, StorageEngine};
use tempfile::TempDir;

/// Helper to create a test storage engine
fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

/// Setup test data in users collection
fn setup_users_collection(storage: &StorageEngine) {
    storage.create_collection("users".to_string(), None).unwrap();
    let collection = storage.get_collection("users").unwrap();

    collection
        .insert(json!({
            "_key": "alice",
            "name": "Alice",
            "age": 30,
            "city": "Paris",
            "active": true
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "bob",
            "name": "Bob",
            "age": 25,
            "city": "London",
            "active": true
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "charlie",
            "name": "Charlie",
            "age": 35,
            "city": "Paris",
            "active": false
        }))
        .unwrap();
}

#[test]
fn test_execute_length_with_collection() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Test LENGTH with existing collection argument
    let query = parse("RETURN LENGTH(\"users\")").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // "users" collection has 3 documents (setup_users_collection inserts 3)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(3));

    // Test LENGTH with non-existing collection (should return string length)
    let query_str = "RETURN LENGTH(\"non_existent_collection\")";
    let query = parse(query_str).unwrap();
    let results = executor.execute(&query).unwrap();

    // "non_existent_collection" has length 23
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(23));
}
