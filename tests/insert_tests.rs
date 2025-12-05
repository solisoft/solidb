// ==================== INSERT Statement Tests ====================

use serde_json::json;
use solidb::{parse, QueryExecutor, StorageEngine};
use tempfile::TempDir;

/// Helper to create a test storage engine
fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

#[test]
fn test_insert_simple() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("numbers".to_string()).unwrap();

    // Insert a single document (without RETURN NEW - that feature requires executor support)
    let query = parse("FOR i IN 1..1 INSERT { value: 42 } INTO numbers RETURN i").unwrap();
    let executor = QueryExecutor::new(&storage);
    let _results = executor.execute(&query).unwrap();

    // Verify the document was inserted
    let collection = storage.get_collection("numbers").unwrap();
    let all_docs = collection.scan(None);
    assert_eq!(all_docs.len(), 1);
    assert_eq!(all_docs[0].data["value"], json!(42.0));
}

#[test]
fn test_insert_in_loop() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("numbers".to_string()).unwrap();

    // Insert multiple documents using FOR loop
    let query = parse(
        r#"
        FOR i IN 1..5
          INSERT { value: i } INTO numbers
          RETURN i
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 5);

    // Verify all documents were inserted
    let collection = storage.get_collection("numbers").unwrap();
    let all_docs = collection.scan(None);
    assert_eq!(all_docs.len(), 5);
}

#[test]
fn test_insert_with_object_construction() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();

    // Insert with object construction
    let query = parse(
        r#"
        FOR i IN 1..3
          INSERT { name: CONCAT("User", i), index: i } INTO users
          RETURN i
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);

    // Verify documents
    let collection = storage.get_collection("users").unwrap();
    let all_docs = collection.scan(None);
    assert_eq!(all_docs.len(), 3);
}

#[test]
fn test_insert_from_existing_collection() {
    let (storage, _dir) = create_test_storage();

    // Create source collection with data
    storage.create_collection("source".to_string()).unwrap();
    let source = storage.get_collection("source").unwrap();
    source.insert(json!({"name": "Alice", "age": 30})).unwrap();
    source.insert(json!({"name": "Bob", "age": 25})).unwrap();

    // Create target collection
    storage.create_collection("target".to_string()).unwrap();

    // Copy documents from source to target
    let query = parse(
        r#"
        FOR doc IN source
          INSERT { name: doc.name, age: doc.age, copied: true } INTO target
          RETURN doc.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);

    // Verify target has the documents
    let target = storage.get_collection("target").unwrap();
    let all_docs = target.scan(None);
    assert_eq!(all_docs.len(), 2);
    assert!(all_docs.iter().all(|d| d.data["copied"] == json!(true)));
}
