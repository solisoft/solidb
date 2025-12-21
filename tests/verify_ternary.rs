//! Verify ternary operator (condition ? true_expr : false_expr) in SDBQL
//!
//! Run with: cargo test --test verify_ternary

use serde_json::json;
use solidb::{parse, QueryExecutor, StorageEngine};
use tempfile::TempDir;

fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

fn setup_users(storage: &StorageEngine) {
    storage.create_collection("users".to_string(), None).unwrap();
    let collection = storage.get_collection("users").unwrap();

    collection
        .insert(json!({
            "_key": "alice",
            "name": "Alice",
            "age": 30,
            "active": true
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "bob",
            "name": "Bob",
            "age": 17,
            "active": false
        }))
        .unwrap();
}

#[test]
fn test_ternary_basic_true() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN true ? "yes" : "no""#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("yes"));
}

#[test]
fn test_ternary_basic_false() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN false ? "yes" : "no""#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("no"));
}

#[test]
fn test_ternary_with_comparison() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN 5 > 3 ? "greater" : "less""#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("greater"));
}

#[test]
fn test_ternary_with_numeric_result() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN true ? 100 : 0").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(100));
}

#[test]
fn test_ternary_in_for_loop() {
    let (storage, _dir) = create_test_storage();
    setup_users(&storage);

    let query = parse(r#"FOR doc IN users RETURN doc.active ? "Active" : "Inactive""#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Active")));
    assert!(results.contains(&json!("Inactive")));
}

#[test]
fn test_ternary_with_field_access() {
    let (storage, _dir) = create_test_storage();
    setup_users(&storage);

    let query = parse(r#"FOR doc IN users RETURN doc.age >= 18 ? "Adult" : "Minor""#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Adult")));
    assert!(results.contains(&json!("Minor")));
}

#[test]
fn test_ternary_in_object() {
    let (storage, _dir) = create_test_storage();
    setup_users(&storage);

    let query = parse(r#"FOR doc IN users RETURN { name: doc.name, status: doc.active ? "active" : "inactive" }"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    
    // Find Alice's result
    let alice = results.iter().find(|r| r["name"] == "Alice").unwrap();
    assert_eq!(alice["status"], "active");
    
    // Find Bob's result
    let bob = results.iter().find(|r| r["name"] == "Bob").unwrap();
    assert_eq!(bob["status"], "inactive");
}

#[test]
fn test_nested_ternary() {
    let (storage, _dir) = create_test_storage();

    // Test: a ? (b ? "ab" : "a") : "none"
    let query = parse(r#"RETURN true ? (false ? "ab" : "a") : "none""#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("a"));
}

#[test]
fn test_ternary_equivalent_to_if() {
    let (storage, _dir) = create_test_storage();

    // Test that ternary gives same result as IF function
    let ternary_query = parse(r#"RETURN 10 > 5 ? "yes" : "no""#).unwrap();
    let if_query = parse(r#"RETURN IF(10 > 5, "yes", "no")"#).unwrap();
    
    let executor = QueryExecutor::new(&storage);
    
    let ternary_results = executor.execute(&ternary_query).unwrap();
    let if_results = executor.execute(&if_query).unwrap();

    assert_eq!(ternary_results, if_results);
}
