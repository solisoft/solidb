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
fn test_has_function_basic() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string(), None).unwrap();
    let collection = storage.get_collection("users").unwrap();

    collection
        .insert(json!({
            "_key": "alice",
            "name": "Alice",
            "age": 30
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "bob",
            "name": "Bob",
            "email": "bob@example.com"
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "charlie",
            "name": "Charlie"
        }))
        .unwrap();

    // Query for documents that have "age"
    let query = parse("FOR doc IN users FILTER HAS(doc, \"age\") RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));

    // Query for documents that have "email"
    let query = parse("FOR doc IN users FILTER HAS(doc, \"email\") RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Bob"));

    // Query for documents that do NOT have "age"
    // Note: HAS returns bool, so we can check == false or using ! operator if supported (but let's use == false for safety first if ! is not guaranteed yet, actually ! should be supported but let's stick to simple implementation first or check what parser supports. The parser supports standard booleans, so HAS(...) == false should work)
    let query = parse("FOR doc IN users FILTER HAS(doc, \"age\") == false RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Bob")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_has_function_argument_validation() {
    let (storage, _dir) = create_test_storage();
    // Test invalid argument count
    let query = parse("RETURN HAS({})").unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("requires 2 arguments"));

    // Test invalid first argument type
    let query = parse("RETURN HAS(\"not an object\", \"field\")").unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("first argument must be a document"));

    // Test invalid second argument type
    let query = parse("RETURN HAS({}, 123)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("second argument must be a string"));
}

#[test]
fn test_has_nested_object() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("data".to_string(), None).unwrap();
    let collection = storage.get_collection("data").unwrap();

    collection
        .insert(json!({
            "meta": {
                "created_at": 12345
            }
        }))
        .unwrap();

    // Check existence of field in nested object
    let query = parse("FOR doc IN data RETURN HAS(doc.meta, \"created_at\")").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(true));

     // Check non-existence of field in nested object
    let query = parse("FOR doc IN data RETURN HAS(doc.meta, \"updated_at\")").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(false));
}
