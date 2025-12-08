use serde_json::json;
use solidb::{parse, QueryExecutor, StorageEngine};
use tempfile::TempDir;

fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

#[test]
fn test_keep_function_varargs() {
    let (storage, _dir) = create_test_storage();
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

    // Keep name and age
    let query = parse("FOR doc IN users RETURN KEEP(doc, \"name\", \"age\")").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    let doc = &results[0];
    assert_eq!(doc.as_object().unwrap().len(), 2);
    assert_eq!(doc["name"], json!("Alice"));
    assert_eq!(doc["age"], json!(30));
    assert!(doc.get("city").is_none());
    assert!(doc.get("active").is_none());
}

#[test]
fn test_keep_function_array() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string(), None).unwrap();
    let collection = storage.get_collection("users").unwrap();

    collection
        .insert(json!({
            "_key": "bob",
            "name": "Bob",
            "age": 25,
            "city": "London"
        }))
        .unwrap();

    // Keep name and city using array
    let query = parse("FOR doc IN users RETURN KEEP(doc, [\"name\", \"city\"])").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    let doc = &results[0];
    assert_eq!(doc.as_object().unwrap().len(), 2);
    assert_eq!(doc["name"], json!("Bob"));
    assert_eq!(doc["city"], json!("London"));
    assert!(doc.get("age").is_none());
}

#[test]
fn test_keep_non_existent_attribute() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string(), None).unwrap();
    let collection = storage.get_collection("users").unwrap();

    collection
        .insert(json!({
            "name": "Charlie"
        }))
        .unwrap();

    // Keep name and non-existent attribute
    let query = parse("FOR doc IN users RETURN KEEP(doc, \"name\", \"missing\")").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    let doc = &results[0];
    assert_eq!(doc.as_object().unwrap().len(), 1);
    assert_eq!(doc["name"], json!("Charlie"));
}

#[test]
fn test_keep_argument_validation() {
    let (storage, _dir) = create_test_storage();
    
    // Not enough arguments
    let query = parse("RETURN KEEP({a:1})").unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("requires at least 2 arguments"));

    // First arg not object
    let query = parse("RETURN KEEP(\"not object\", \"attr\")").unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("first argument must be a document"));

    // Invalid attribute type (number instead of string)
    let query = parse("RETURN KEEP({a:1}, 123)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("attribute names must be strings"));
}
