//! Optional Chaining Tests for SDBQL
//!
//! Tests for the ?. operator that safely accesses nested properties,
//! returning null if the base is null or not an object.

use serde_json::json;
use solidb::parse;
use solidb::sdbql::QueryExecutor;
use solidb::storage::StorageEngine;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine =
        StorageEngine::new(tmp_dir.path().to_str().unwrap()).expect("Failed to create storage");
    (engine, tmp_dir)
}

fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect("Failed to parse query");
    let executor = QueryExecutor::new(engine);
    executor.execute(&query).expect("Failed to execute query")
}

fn setup_test_data(engine: &StorageEngine) {
    engine
        .create_collection("users".to_string(), None)
        .unwrap();
    let users = engine.get_collection("users").unwrap();

    // User with full nested data
    users
        .insert(json!({
            "_key": "u1",
            "name": "Alice",
            "address": {
                "city": "New York",
                "zip": "10001",
                "location": {
                    "lat": 40.7128,
                    "lng": -74.0060
                }
            },
            "profile": {
                "bio": "Developer"
            }
        }))
        .unwrap();

    // User with partial data (no address)
    users
        .insert(json!({
            "_key": "u2",
            "name": "Bob",
            "profile": null
        }))
        .unwrap();

    // User with address but no location
    users
        .insert(json!({
            "_key": "u3",
            "name": "Charlie",
            "address": {
                "city": "Boston"
            }
        }))
        .unwrap();
}

// ============================================================================
// Basic Optional Chaining Tests
// ============================================================================

#[test]
fn test_optional_chaining_on_existing_field() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        FILTER doc._key == "u1"
        RETURN doc?.name
    "#,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

#[test]
fn test_optional_chaining_on_null_field() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        FILTER doc._key == "u2"
        RETURN doc.profile?.bio
    "#,
    );

    assert_eq!(results.len(), 1);
    assert!(results[0].is_null());
}

#[test]
fn test_optional_chaining_on_missing_field() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        FILTER doc._key == "u2"
        RETURN doc.address?.city
    "#,
    );

    assert_eq!(results.len(), 1);
    assert!(results[0].is_null());
}

// ============================================================================
// Deep Optional Chaining Tests
// ============================================================================

#[test]
fn test_deep_optional_chaining_success() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        FILTER doc._key == "u1"
        RETURN doc.address?.location?.lat
    "#,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(40.7128));
}

#[test]
fn test_deep_optional_chaining_null_intermediate() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        FILTER doc._key == "u3"
        RETURN doc.address?.location?.lat
    "#,
    );

    assert_eq!(results.len(), 1);
    assert!(results[0].is_null());
}

#[test]
fn test_deep_optional_chaining_missing_root() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        FILTER doc._key == "u2"
        RETURN doc.address?.location?.lat
    "#,
    );

    assert_eq!(results.len(), 1);
    assert!(results[0].is_null());
}

// ============================================================================
// Mixed Access Tests (regular . and ?.)
// ============================================================================

#[test]
fn test_mixed_regular_and_optional_access() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        FILTER doc._key == "u1"
        RETURN doc.address.location?.lat
    "#,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(40.7128));
}

#[test]
fn test_optional_then_regular_access() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    // doc.address?.city uses optional then tries to access city
    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        FILTER doc._key == "u1"
        RETURN doc.address?.city
    "#,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("New York"));
}

// ============================================================================
// With Null Coalescing Operator
// ============================================================================

#[test]
fn test_optional_chaining_with_null_coalescing() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        RETURN {
            name: doc.name,
            city: doc.address?.city ?? "Unknown"
        }
    "#,
    );

    assert_eq!(results.len(), 3);

    let alice = results.iter().find(|r| r["name"] == json!("Alice")).unwrap();
    assert_eq!(alice["city"], json!("New York"));

    let bob = results.iter().find(|r| r["name"] == json!("Bob")).unwrap();
    assert_eq!(bob["city"], json!("Unknown"));

    let charlie = results.iter().find(|r| r["name"] == json!("Charlie")).unwrap();
    assert_eq!(charlie["city"], json!("Boston"));
}

#[test]
fn test_deep_optional_with_null_coalescing() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        RETURN {
            name: doc.name,
            lat: doc.address?.location?.lat ?? 0
        }
    "#,
    );

    assert_eq!(results.len(), 3);

    let alice = results.iter().find(|r| r["name"] == json!("Alice")).unwrap();
    assert_eq!(alice["lat"], json!(40.7128));

    let bob = results.iter().find(|r| r["name"] == json!("Bob")).unwrap();
    assert_eq!(bob["lat"], json!(0));

    let charlie = results.iter().find(|r| r["name"] == json!("Charlie")).unwrap();
    assert_eq!(charlie["lat"], json!(0));
}

// ============================================================================
// In Object Construction
// ============================================================================

#[test]
fn test_optional_chaining_in_return_object() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        RETURN {
            name: doc.name,
            city: doc.address?.city,
            bio: doc.profile?.bio,
            lat: doc.address?.location?.lat
        }
    "#,
    );

    assert_eq!(results.len(), 3);

    let alice = results.iter().find(|r| r["name"] == json!("Alice")).unwrap();
    assert_eq!(alice["city"], json!("New York"));
    assert_eq!(alice["bio"], json!("Developer"));
    assert_eq!(alice["lat"], json!(40.7128));

    let bob = results.iter().find(|r| r["name"] == json!("Bob")).unwrap();
    assert!(bob["city"].is_null());
    assert!(bob["bio"].is_null());
    assert!(bob["lat"].is_null());
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_optional_chaining_on_array() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("data".to_string(), None)
        .unwrap();
    let data = engine.get_collection("data").unwrap();

    data.insert(json!({
        "_key": "d1",
        "items": [1, 2, 3]
    }))
    .unwrap();

    // Optional chaining on an array should return null (not an object)
    let results = execute_query(
        &engine,
        r#"
        FOR doc IN data
        RETURN doc.items?.first
    "#,
    );

    assert_eq!(results.len(), 1);
    assert!(results[0].is_null());
}

#[test]
fn test_optional_chaining_on_number() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("data".to_string(), None)
        .unwrap();
    let data = engine.get_collection("data").unwrap();

    data.insert(json!({
        "_key": "d1",
        "num": 42
    }))
    .unwrap();

    // Optional chaining on a number should return null
    let results = execute_query(
        &engine,
        r#"
        FOR doc IN data
        RETURN doc.num?.value
    "#,
    );

    assert_eq!(results.len(), 1);
    assert!(results[0].is_null());
}

#[test]
fn test_optional_chaining_on_string() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("data".to_string(), None)
        .unwrap();
    let data = engine.get_collection("data").unwrap();

    data.insert(json!({
        "_key": "d1",
        "text": "hello"
    }))
    .unwrap();

    // Optional chaining on a string should return null
    let results = execute_query(
        &engine,
        r#"
        FOR doc IN data
        RETURN doc.text?.length
    "#,
    );

    assert_eq!(results.len(), 1);
    assert!(results[0].is_null());
}

#[test]
fn test_optional_chaining_in_filter() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    // Filter using optional chaining
    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        FILTER doc.address?.city == "New York"
        RETURN doc.name
    "#,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

#[test]
fn test_optional_chaining_null_comparison() {
    let (engine, _tmp) = create_test_engine();
    setup_test_data(&engine);

    // Find users where address is missing or city is null
    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
        FILTER doc.address?.city == null
        RETURN doc.name
    "#,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Bob"));
}
