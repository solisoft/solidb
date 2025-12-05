//! AQL Parser and Query Executor Tests
//! Tests for the AQL query language implementation

use serde_json::json;
use solidb::{parse, BindVars, QueryExecutor, StorageEngine};
use std::collections::HashMap;
use tempfile::TempDir;

/// Helper to create a test storage engine
fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

/// Setup test data in users collection
fn setup_users_collection(storage: &StorageEngine) {
    storage.create_collection("users".to_string()).unwrap();
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

// ==================== Parser Tests ====================

#[test]
fn test_parse_simple_for_return() {
    let query = parse("FOR doc IN users RETURN doc").unwrap();
    assert_eq!(query.for_clauses.len(), 1);
    assert_eq!(query.for_clauses[0].variable, "doc");
    assert_eq!(query.for_clauses[0].collection, "users");
}

#[test]
fn test_parse_filter() {
    let query = parse("FOR doc IN users FILTER doc.age > 25 RETURN doc").unwrap();
    assert_eq!(query.filter_clauses.len(), 1);
}

#[test]
fn test_parse_multiple_filters() {
    let query =
        parse("FOR doc IN users FILTER doc.age > 25 FILTER doc.active == true RETURN doc").unwrap();
    assert_eq!(query.filter_clauses.len(), 2);
}

#[test]
fn test_parse_sort() {
    let query = parse("FOR doc IN users SORT doc.age DESC RETURN doc").unwrap();
    assert!(query.sort_clause.is_some());
    let sort = query.sort_clause.unwrap();
    assert!(!sort.ascending);
}

#[test]
fn test_parse_limit() {
    let query = parse("FOR doc IN users LIMIT 10 RETURN doc").unwrap();
    assert!(query.limit_clause.is_some());
    let limit = query.limit_clause.unwrap();
    assert_eq!(limit.count, 10);
    assert_eq!(limit.offset, 0);
}

#[test]
fn test_parse_limit_with_offset() {
    let query = parse("FOR doc IN users LIMIT 5, 10 RETURN doc").unwrap();
    assert!(query.limit_clause.is_some());
    let limit = query.limit_clause.unwrap();
    assert_eq!(limit.offset, 5);
    assert_eq!(limit.count, 10);
}

#[test]
fn test_parse_object_return() {
    // Just verify it parses without error
    let _query = parse("FOR doc IN users RETURN {name: doc.name, age: doc.age}").unwrap();
}

#[test]
fn test_parse_join() {
    let query = parse("FOR u IN users FOR o IN orders RETURN {user: u.name, order: o.id}").unwrap();
    assert_eq!(query.for_clauses.len(), 2);
}

#[test]
fn test_parse_invalid_query() {
    let result = parse("INVALID QUERY");
    assert!(result.is_err());
}

#[test]
fn test_parse_missing_return() {
    let result = parse("FOR doc IN users");
    assert!(result.is_err());
}

// ==================== RETURN-only Query Tests ====================

#[test]
fn test_return_only_simple() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN 42").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(42.0));
}

#[test]
fn test_return_only_arithmetic() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN 1 + 2 * 3").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(7.0));
}

#[test]
fn test_return_only_merge() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN MERGE({a: 1, b: 2}, {c: 3, d: 4})").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["a"], json!(1.0));
    assert_eq!(results[0]["b"], json!(2.0));
    assert_eq!(results[0]["c"], json!(3.0));
    assert_eq!(results[0]["d"], json!(4.0));
}

#[test]
fn test_return_only_array() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN [1, 2, 3]").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!([1.0, 2.0, 3.0]));
}

#[test]
fn test_return_only_object() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN {name: \"test\", value: 123}").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], json!("test"));
    assert_eq!(results[0]["value"], json!(123.0));
}

#[test]
fn test_let_with_return_only() {
    let (storage, _dir) = create_test_storage();

    let query = parse("LET a = [1, 2, 3] RETURN a[0]").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(1.0));
}

#[test]
fn test_multiple_let_with_return_only() {
    let (storage, _dir) = create_test_storage();

    let query = parse("LET a = 10 LET b = 20 RETURN a + b").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(30.0));
}

// ==================== Query Executor Tests ====================

#[test]
fn test_execute_simple_query() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users RETURN doc").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);
}

#[test]
fn test_execute_filter_equality() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.city == \"Paris\" RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_execute_filter_greater_than() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.age > 25 RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_execute_filter_less_than() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.age < 30 RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert!(results.contains(&json!("Bob")));
}

#[test]
fn test_execute_filter_boolean() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.active == true RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Bob")));
}

#[test]
fn test_execute_filter_not_equal() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.city != \"Paris\" RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert!(results.contains(&json!("Bob")));
}

#[test]
fn test_execute_sort_ascending() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users SORT doc.age ASC RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0], json!("Bob")); // 25
    assert_eq!(results[1], json!("Alice")); // 30
    assert_eq!(results[2], json!("Charlie")); // 35
}

#[test]
fn test_execute_sort_descending() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users SORT doc.age DESC RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);
    assert_eq!(results[0], json!("Charlie")); // 35
    assert_eq!(results[1], json!("Alice")); // 30
    assert_eq!(results[2], json!("Bob")); // 25
}

#[test]
fn test_execute_limit() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users LIMIT 2 RETURN doc").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
}

#[test]
fn test_execute_limit_with_offset() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users SORT doc.age ASC LIMIT 1, 2 RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    // Skip Bob (25), get Alice (30) and Charlie (35)
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_execute_object_projection() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query =
        parse("FOR doc IN users FILTER doc.name == \"Alice\" RETURN {n: doc.name, a: doc.age}")
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["n"], "Alice");
    assert_eq!(results[0]["a"], 30);
}

#[test]
fn test_execute_field_access() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Bob")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_execute_multiple_filters() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(
        "FOR doc IN users FILTER doc.city == \"Paris\" FILTER doc.active == true RETURN doc.name",
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

#[test]
fn test_execute_and_condition() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query =
        parse("FOR doc IN users FILTER doc.age > 25 AND doc.age < 35 RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

#[test]
fn test_execute_or_condition() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query =
        parse("FOR doc IN users FILTER doc.age == 25 OR doc.age == 35 RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Bob")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_execute_combined_clauses() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(
        "FOR doc IN users FILTER doc.active == true SORT doc.age DESC LIMIT 1 RETURN doc.name",
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice")); // Oldest active user
}

// ==================== JOIN Tests ====================

#[test]
fn test_execute_join() {
    let (storage, _dir) = create_test_storage();

    // Create users
    storage.create_collection("users".to_string()).unwrap();
    let users = storage.get_collection("users").unwrap();
    users.insert(json!({"_key": "1", "name": "Alice"})).unwrap();
    users.insert(json!({"_key": "2", "name": "Bob"})).unwrap();

    // Create orders
    storage.create_collection("orders".to_string()).unwrap();
    let orders = storage.get_collection("orders").unwrap();
    orders
        .insert(json!({"_key": "o1", "user_id": "1", "product": "Laptop"}))
        .unwrap();
    orders
        .insert(json!({"_key": "o2", "user_id": "1", "product": "Phone"}))
        .unwrap();
    orders
        .insert(json!({"_key": "o3", "user_id": "2", "product": "Tablet"}))
        .unwrap();

    let query = parse("FOR u IN users FOR o IN orders FILTER o.user_id == u._key RETURN {user: u.name, product: o.product}").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);

    // Check Alice's orders
    let alice_orders: Vec<_> = results.iter().filter(|r| r["user"] == "Alice").collect();
    assert_eq!(alice_orders.len(), 2);
}

// ==================== Built-in Function Tests ====================

#[test]
fn test_execute_length_function() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users RETURN LENGTH(doc.name)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert!(results.contains(&json!(5))); // Alice
    assert!(results.contains(&json!(3))); // Bob
    assert!(results.contains(&json!(7))); // Charlie
}

#[test]
fn test_execute_upper_function() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query =
        parse("FOR doc IN users FILTER doc.name == \"Alice\" RETURN UPPER(doc.name)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("ALICE"));
}

#[test]
fn test_execute_lower_function() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query =
        parse("FOR doc IN users FILTER doc.name == \"Alice\" RETURN LOWER(doc.name)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("alice"));
}

#[test]
fn test_execute_round_function() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("numbers".to_string()).unwrap();
    let collection = storage.get_collection("numbers").unwrap();
    collection
        .insert(json!({"_key": "1", "value": 3.7}))
        .unwrap();

    let query = parse("FOR doc IN numbers RETURN ROUND(doc.value)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(4.0));
}

#[test]
fn test_execute_abs_function() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("numbers".to_string()).unwrap();
    let collection = storage.get_collection("numbers").unwrap();
    collection
        .insert(json!({"_key": "1", "value": -42}))
        .unwrap();

    let query = parse("FOR doc IN numbers RETURN ABS(doc.value)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(42.0));
}

#[test]
fn test_execute_concat_separator_function() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("data".to_string()).unwrap();
    let collection = storage.get_collection("data").unwrap();
    collection
        .insert(json!({
            "_key": "1",
            "tags": ["rust", "database", "aql"]
        }))
        .unwrap();

    let query = parse("FOR doc IN data RETURN CONCAT_SEPARATOR(\", \", doc.tags)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("rust, database, aql"));
}

#[test]
fn test_execute_concat_separator_with_numbers() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("data".to_string()).unwrap();
    let collection = storage.get_collection("data").unwrap();
    collection
        .insert(json!({
            "_key": "1",
            "values": [1, 2, 3, 4, 5]
        }))
        .unwrap();

    let query = parse("FOR doc IN data RETURN CONCAT_SEPARATOR(\"-\", doc.values)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("1-2-3-4-5"));
}

#[test]
fn test_execute_concat_separator_empty_array() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("data".to_string()).unwrap();
    let collection = storage.get_collection("data").unwrap();
    collection
        .insert(json!({
            "_key": "1",
            "items": []
        }))
        .unwrap();

    let query = parse("FOR doc IN data RETURN CONCAT_SEPARATOR(\",\", doc.items)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(""));
}

// ==================== MERGE Function Tests ====================

#[test]
fn test_merge_two_objects() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(
        r#"
        FOR doc IN users
        LIMIT 1
        RETURN MERGE({a: 1, b: 2}, {c: 3, d: 4})
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["a"], json!(1.0));
    assert_eq!(results[0]["b"], json!(2.0));
    assert_eq!(results[0]["c"], json!(3.0));
    assert_eq!(results[0]["d"], json!(4.0));
}

#[test]
fn test_merge_override_values() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Later objects should override earlier ones
    let query = parse(
        r#"
        FOR doc IN users
        LIMIT 1
        RETURN MERGE({a: 1, b: 2}, {b: 99, c: 3})
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["a"], json!(1.0));
    assert_eq!(results[0]["b"], json!(99.0)); // Overridden
    assert_eq!(results[0]["c"], json!(3.0));
}

#[test]
fn test_merge_multiple_objects() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(
        r#"
        FOR doc IN users
        LIMIT 1
        RETURN MERGE({a: 1}, {b: 2}, {c: 3}, {d: 4})
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["a"], json!(1.0));
    assert_eq!(results[0]["b"], json!(2.0));
    assert_eq!(results[0]["c"], json!(3.0));
    assert_eq!(results[0]["d"], json!(4.0));
}

#[test]
fn test_merge_with_document() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Merge document with additional fields
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == "Alice"
        RETURN MERGE(doc, {status: "premium", points: 100})
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], json!("Alice"));
    assert_eq!(results[0]["age"], json!(30));
    assert_eq!(results[0]["status"], json!("premium"));
    assert_eq!(results[0]["points"], json!(100.0));
}

#[test]
fn test_merge_with_null() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Null values should be skipped
    let query = parse(
        r#"
        FOR doc IN users
        LIMIT 1
        RETURN MERGE({a: 1}, null, {b: 2})
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["a"], json!(1.0));
    assert_eq!(results[0]["b"], json!(2.0));
}

#[test]
fn test_merge_single_object() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(
        r#"
        FOR doc IN users
        LIMIT 1
        RETURN MERGE({a: 1, b: 2})
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["a"], json!(1.0));
    assert_eq!(results[0]["b"], json!(2.0));
}

// ==================== Geo Function Tests ====================

#[test]
fn test_execute_distance_function() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("places".to_string()).unwrap();
    let collection = storage.get_collection("places").unwrap();
    collection
        .insert(json!({
            "_key": "eiffel",
            "lat": 48.8584,
            "lon": 2.2945
        }))
        .unwrap();

    // Distance from Eiffel Tower to Arc de Triomphe (approx 48.8738, 2.2950)
    let query =
        parse("FOR doc IN places RETURN ROUND(DISTANCE(doc.lat, doc.lon, 48.8738, 2.2950))")
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // Distance should be around 1700 meters
    let dist = results[0].as_f64().unwrap();
    assert!(dist > 1500.0 && dist < 2000.0);
}

// ==================== Edge Cases ====================

#[test]
fn test_empty_collection() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("empty".to_string()).unwrap();

    let query = parse("FOR doc IN empty RETURN doc").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_filter_no_matches() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.age > 100 RETURN doc").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_nested_field_access() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();
    collection
        .insert(json!({
            "_key": "alice",
            "name": "Alice",
            "address": {
                "city": "Paris",
                "country": "France"
            }
        }))
        .unwrap();

    let query = parse("FOR doc IN users RETURN doc.address.city").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Paris"));
}

#[test]
fn test_filter_nested_field() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();
    collection
        .insert(json!({
            "_key": "alice",
            "name": "Alice",
            "address": {"city": "Paris"}
        }))
        .unwrap();
    collection
        .insert(json!({
            "_key": "bob",
            "name": "Bob",
            "address": {"city": "London"}
        }))
        .unwrap();

    let query =
        parse("FOR doc IN users FILTER doc.address.city == \"Paris\" RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

#[test]
fn test_null_field_handling() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("users".to_string()).unwrap();
    let collection = storage.get_collection("users").unwrap();
    collection
        .insert(json!({
            "_key": "alice",
            "name": "Alice"
            // No age field
        }))
        .unwrap();

    let query = parse("FOR doc IN users RETURN doc.age").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(null));
}

// ==================== LET Subquery Tests ====================

#[test]
fn test_parse_let_clause() {
    let query = parse("LET x = 42 FOR doc IN users RETURN doc").unwrap();
    assert_eq!(query.let_clauses.len(), 1);
    assert_eq!(query.let_clauses[0].variable, "x");
}

#[test]
fn test_parse_let_with_subquery() {
    let query =
        parse("LET allUsers = (FOR u IN users RETURN u) FOR x IN allUsers RETURN x").unwrap();
    assert_eq!(query.let_clauses.len(), 1);
    assert_eq!(query.let_clauses[0].variable, "allUsers");
}

#[test]
fn test_parse_multiple_let_clauses() {
    let query = parse("LET x = 1 LET y = 2 FOR doc IN users RETURN doc").unwrap();
    assert_eq!(query.let_clauses.len(), 2);
    assert_eq!(query.let_clauses[0].variable, "x");
    assert_eq!(query.let_clauses[1].variable, "y");
}

#[test]
fn test_execute_let_with_literal() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query =
        parse("LET minAge = 30 FOR doc IN users FILTER doc.age >= minAge RETURN doc.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice"))); // age 30
    assert!(results.contains(&json!("Charlie"))); // age 35
}

#[test]
fn test_execute_let_with_subquery() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // LET somedata = (FOR u IN users RETURN u)
    // FOR item IN somedata RETURN item.name
    let query =
        parse("LET somedata = (FOR u IN users RETURN u) FOR item IN somedata RETURN item.name")
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Bob")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_execute_let_subquery_with_filter() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Get only active users via subquery, then iterate
    let query = parse("LET activeUsers = (FOR u IN users FILTER u.active == true RETURN u) FOR item IN activeUsers RETURN item.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Bob")));
    assert!(!results.contains(&json!("Charlie"))); // Charlie is not active
}

#[test]
fn test_execute_let_subquery_with_sort_limit() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Get top 2 oldest users
    let query = parse("LET topUsers = (FOR u IN users SORT u.age DESC LIMIT 2 RETURN u) FOR item IN topUsers RETURN item.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Charlie"))); // age 35
    assert!(results.contains(&json!("Alice"))); // age 30
}

#[test]
fn test_execute_multiple_let_clauses() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Two LET clauses with different filters
    let query = parse(
        r#"
        LET parisUsers = (FOR u IN users FILTER u.city == "Paris" RETURN u)
        LET activeUsers = (FOR u IN users FILTER u.active == true RETURN u)
        FOR p IN parisUsers
        RETURN p.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Charlie")));
}

#[test]
fn test_execute_let_with_length() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Use LENGTH function on subquery result
    let query = parse(
        "LET allUsers = (FOR u IN users RETURN u) FOR doc IN users LIMIT 1 RETURN LENGTH(allUsers)",
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(3)); // 3 users total
}

#[test]
fn test_execute_let_array_literal() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // LET with array literal
    let query = parse("LET items = [1, 2, 3] FOR x IN items RETURN x").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);
    // Numbers from lexer are f64, so compare with floats
    assert!(results.contains(&json!(1.0)));
    assert!(results.contains(&json!(2.0)));
    assert!(results.contains(&json!(3.0)));
}

#[test]
fn test_execute_let_in_return_projection() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Access LET variable in RETURN
    let query = parse("LET prefix = \"User: \" FOR doc IN users FILTER doc.name == \"Alice\" RETURN CONCAT(prefix, doc.name)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("User: Alice"));
}

// ==================== Array Access Tests ====================

#[test]
fn test_array_access_basic() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Access first element of array
    let query = parse("LET a = [1, 2, 3] FOR doc IN users LIMIT 1 RETURN a[0]").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(1.0));
}

#[test]
fn test_array_access_second_element() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Access second element
    let query = parse("LET a = [1, 2, 3] FOR doc IN users LIMIT 1 RETURN a[1]").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(2.0));
}

#[test]
fn test_array_access_last_element() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Access last element
    let query = parse("LET a = [1, 2, 3] FOR doc IN users LIMIT 1 RETURN a[2]").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(3.0));
}

#[test]
fn test_array_access_out_of_bounds() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Out of bounds should return null
    let query = parse("LET a = [1, 2, 3] FOR doc IN users LIMIT 1 RETURN a[10]").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(null));
}

#[test]
fn test_array_access_nested() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Access nested array
    let query = parse("LET a = [[1, 2], [3, 4]] FOR doc IN users LIMIT 1 RETURN a[0][1]").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(2.0));
}

#[test]
fn test_array_access_from_document() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("data".to_string()).unwrap();
    let collection = storage.get_collection("data").unwrap();
    collection
        .insert(json!({
            "_key": "1",
            "tags": ["rust", "database", "aql"]
        }))
        .unwrap();

    // Access array field from document
    let query = parse("FOR doc IN data RETURN doc.tags[0]").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("rust"));
}

// ==================== Bind Variables Tests (Security) ====================

#[test]
fn test_parse_bind_variable() {
    // Verify @variable syntax parses correctly
    let query = parse("FOR doc IN users FILTER doc.name == @name RETURN doc").unwrap();
    assert_eq!(query.filter_clauses.len(), 1);
}

#[test]
fn test_execute_bind_variable_string() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.name == @name RETURN doc.name").unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("name".to_string(), json!("Alice"));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

#[test]
fn test_execute_bind_variable_number() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.age > @minAge RETURN doc.name").unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("minAge".to_string(), json!(28));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice"))); // age 30
    assert!(results.contains(&json!("Charlie"))); // age 35
}

#[test]
fn test_execute_bind_variable_boolean() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.active == @isActive RETURN doc.name").unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("isActive".to_string(), json!(false));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Charlie"));
}

#[test]
fn test_execute_multiple_bind_variables() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query =
        parse("FOR doc IN users FILTER doc.age >= @minAge AND doc.city == @city RETURN doc.name")
            .unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("minAge".to_string(), json!(30));
    bind_vars.insert("city".to_string(), json!("Paris"));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("Alice"))); // age 30, Paris
    assert!(results.contains(&json!("Charlie"))); // age 35, Paris
}

#[test]
fn test_execute_bind_variable_in_return() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(
        "FOR doc IN users FILTER doc.name == \"Alice\" RETURN { name: doc.name, label: @label }",
    )
    .unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("label".to_string(), json!("VIP Customer"));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "Alice");
    assert_eq!(results[0]["label"], "VIP Customer");
}

#[test]
fn test_bind_variable_missing_returns_empty() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.name == @name RETURN doc").unwrap();

    // Don't provide the bind variable - filter evaluations with missing
    // bind vars will fail gracefully and return no matches
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Returns empty because the filter with missing @name doesn't match anything
    assert_eq!(results.len(), 0);
}

#[test]
fn test_bind_variable_missing_in_return_error() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Missing bind var in RETURN should error (not in filter)
    let query = parse("FOR doc IN users RETURN @missing").unwrap();

    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    // Should fail because @missing is used in RETURN, not just filter
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("@missing"));
}

#[test]
fn test_bind_variable_prevents_injection() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // This malicious input would be dangerous with string concatenation
    // but is safe with bind variables
    let malicious_input = "Alice\" OR 1==1 OR \"x\"==\"x";

    let query = parse("FOR doc IN users FILTER doc.name == @name RETURN doc.name").unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("name".to_string(), json!(malicious_input));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    // Should return empty because no user has this exact name
    // (the malicious payload is treated as a literal string, not AQL code)
    assert_eq!(results.len(), 0);
}

// ==================== Dynamic Field Access Tests ====================

#[test]
fn test_parse_dynamic_field_access() {
    // Verify doc[@field] syntax parses correctly
    let query = parse("FOR doc IN users RETURN doc[@field]").unwrap();
    assert_eq!(query.for_clauses.len(), 1);
}

#[test]
fn test_dynamic_field_access_with_bind_var() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Use @fieldName to dynamically access a field
    let query = parse("FOR doc IN users FILTER doc[@field] == @value RETURN doc.name").unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("field".to_string(), json!("name"));
    bind_vars.insert("value".to_string(), json!("Alice"));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

#[test]
fn test_dynamic_field_access_different_fields() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Access "age" field dynamically
    let query = parse("FOR doc IN users FILTER doc[@field] > @minVal RETURN doc.name").unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("field".to_string(), json!("age"));
    bind_vars.insert("minVal".to_string(), json!(30));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Charlie")); // age 35
}

#[test]
fn test_dynamic_field_access_in_return() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Return dynamic field
    let query = parse("FOR doc IN users FILTER doc.name == \"Alice\" RETURN doc[@field]").unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("field".to_string(), json!("city"));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Paris"));
}

#[test]
fn test_dynamic_field_access_with_string_literal() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // doc["name"] should work like doc.name
    let query = parse("FOR doc IN users FILTER doc[\"name\"] == \"Alice\" RETURN doc.age").unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(30));
}

#[test]
fn test_dynamic_field_access_combined_with_static() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Combine dynamic and static field access
    let query = parse(
        "FOR doc IN users FILTER doc[@field] == @value AND doc.active == true RETURN doc.name",
    )
    .unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("field".to_string(), json!("city"));
    bind_vars.insert("value".to_string(), json!("Paris"));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    // Alice is in Paris and active, Charlie is in Paris but not active
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));
}

// ==================== Explain/Profile Tests ====================

#[test]
fn test_explain_simple_query() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users RETURN doc").unwrap();
    let executor = QueryExecutor::new(&storage);
    let explain = executor.explain(&query).unwrap();

    // Check collections accessed
    assert_eq!(explain.collections.len(), 1);
    assert_eq!(explain.collections[0].name, "users");
    assert_eq!(explain.collections[0].variable, "doc");
    assert_eq!(explain.collections[0].documents_count, 3);

    // Check timing exists
    assert!(explain.timing.total_us > 0);
    assert_eq!(explain.documents_returned, 3);
}

#[test]
fn test_explain_with_filter() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.age > 25 RETURN doc").unwrap();
    let executor = QueryExecutor::new(&storage);
    let explain = executor.explain(&query).unwrap();

    // Check filter info
    assert_eq!(explain.filters.len(), 1);
    assert_eq!(explain.filters[0].documents_before, 3);
    assert_eq!(explain.filters[0].documents_after, 2); // Alice (30) and Charlie (35)
    assert!(explain.filters[0].time_us >= 0);

    assert_eq!(explain.documents_scanned, 3);
    assert_eq!(explain.documents_returned, 2);
}

#[test]
fn test_explain_with_sort() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users SORT doc.age DESC RETURN doc").unwrap();
    let executor = QueryExecutor::new(&storage);
    let explain = executor.explain(&query).unwrap();

    // Check sort info
    assert!(explain.sort.is_some());
    let sort = explain.sort.unwrap();
    assert_eq!(sort.field, "doc.age");
    assert_eq!(sort.direction, "DESC");
    assert!(sort.time_us >= 0);
}

#[test]
fn test_explain_with_limit() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users LIMIT 2 RETURN doc").unwrap();
    let executor = QueryExecutor::new(&storage);
    let explain = executor.explain(&query).unwrap();

    // Check limit info
    assert!(explain.limit.is_some());
    let limit = explain.limit.unwrap();
    assert_eq!(limit.offset, 0);
    assert_eq!(limit.count, 2);

    assert_eq!(explain.documents_returned, 2);
}

#[test]
fn test_explain_with_subquery() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // LET with subquery
    let query = parse("LET activeUsers = (FOR u IN users FILTER u.active == true RETURN u) FOR item IN activeUsers RETURN item.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let explain = executor.explain(&query).unwrap();

    // Check LET binding info
    assert_eq!(explain.let_bindings.len(), 1);
    assert_eq!(explain.let_bindings[0].variable, "activeUsers");
    assert!(explain.let_bindings[0].is_subquery);
    assert!(explain.let_bindings[0].time_us >= 0);

    // Should return Alice and Bob (active users)
    assert_eq!(explain.documents_returned, 2);

    // Check timing breakdown exists
    assert!(explain.timing.let_clauses_us >= 0);
    assert!(explain.timing.total_us > 0);
}

#[test]
fn test_explain_with_multiple_subqueries() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(
        r#"
        LET parisUsers = (FOR u IN users FILTER u.city == "Paris" RETURN u)
        LET activeUsers = (FOR u IN users FILTER u.active == true RETURN u)
        FOR p IN parisUsers
        RETURN p.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let explain = executor.explain(&query).unwrap();

    // Check LET bindings
    assert_eq!(explain.let_bindings.len(), 2);
    assert_eq!(explain.let_bindings[0].variable, "parisUsers");
    assert!(explain.let_bindings[0].is_subquery);
    assert_eq!(explain.let_bindings[1].variable, "activeUsers");
    assert!(explain.let_bindings[1].is_subquery);

    // Paris users: Alice and Charlie
    assert_eq!(explain.documents_returned, 2);
}

#[test]
fn test_explain_complex_query() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Complex query with subquery, filter, sort, and limit
    let query = parse(
        r#"
        LET topUsers = (FOR u IN users FILTER u.age >= 25 SORT u.age DESC RETURN u)
        FOR user IN topUsers
        FILTER user.active == true
        LIMIT 1
        RETURN { name: user.name, age: user.age }
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let explain = executor.explain(&query).unwrap();

    // Check all components are present
    assert_eq!(explain.let_bindings.len(), 1);
    assert!(explain.let_bindings[0].is_subquery);

    assert_eq!(explain.filters.len(), 1);
    assert!(explain.limit.is_some());

    // All timing fields should be populated
    assert!(explain.timing.total_us > 0);
    assert!(explain.timing.let_clauses_us >= 0);
    assert!(explain.timing.filter_us >= 0);
    assert!(explain.timing.limit_us >= 0);
    assert!(explain.timing.return_projection_us >= 0);

    // Should return Alice (age 30, active)
    assert_eq!(explain.documents_returned, 1);
}

#[test]
fn test_explain_with_bind_vars() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse("FOR doc IN users FILTER doc.age > @minAge RETURN doc.name").unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("minAge".to_string(), json!(28));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let explain = executor.explain(&query).unwrap();

    // Should filter correctly with bind var
    assert_eq!(explain.filters.len(), 1);
    assert_eq!(explain.filters[0].documents_before, 3);
    assert_eq!(explain.filters[0].documents_after, 2); // Alice (30) and Charlie (35)
    assert_eq!(explain.documents_returned, 2);
}

// ==================== Fulltext Search Tests ====================

fn setup_articles_collection(storage: &StorageEngine) {
    storage.create_collection("articles".to_string()).unwrap();
    let collection = storage.get_collection("articles").unwrap();

    // Insert articles with text content
    collection
        .insert(json!({
            "_key": "1",
            "title": "Introduction to Rust Programming",
            "content": "Rust is a systems programming language focused on safety and performance"
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "2",
            "title": "Learning Python for Beginners",
            "content": "Python is a great language for beginners to learn programming"
        }))
        .unwrap();

    collection.insert(json!({
        "_key": "3",
        "title": "Advanced Rust Patterns",
        "content": "This article covers advanced patterns in Rust including traits and lifetimes"
    })).unwrap();

    collection
        .insert(json!({
            "_key": "4",
            "title": "Database Design Fundamentals",
            "content": "Learn about database normalization, indexing, and query optimization"
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "5",
            "title": "Rust vs Go Comparison",
            "content": "Comparing Rust and Go for systems programming and web services"
        }))
        .unwrap();

    // Create fulltext index on title
    collection
        .create_fulltext_index("ft_title".to_string(), "title".to_string(), Some(3))
        .unwrap();
}

#[test]
fn test_fulltext_index_creation() {
    let (storage, _dir) = create_test_storage();
    setup_articles_collection(&storage);

    let collection = storage.get_collection("articles").unwrap();
    let indexes = collection.list_fulltext_indexes();

    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0].name, "ft_title");
    assert_eq!(indexes[0].field, "title");
}

#[test]
fn test_fulltext_exact_match() {
    let (storage, _dir) = create_test_storage();
    setup_articles_collection(&storage);

    let collection = storage.get_collection("articles").unwrap();

    // Search for exact term "rust"
    let results = collection.fulltext_search("title", "rust", 0).unwrap();

    // Should find articles with "Rust" in title
    assert!(results.len() >= 2);
    let doc_keys: Vec<&str> = results.iter().map(|m| m.doc_key.as_str()).collect();
    assert!(doc_keys.contains(&"1") || doc_keys.contains(&"3") || doc_keys.contains(&"5"));
}

#[test]
fn test_fulltext_fuzzy_match() {
    let (storage, _dir) = create_test_storage();
    setup_articles_collection(&storage);

    let collection = storage.get_collection("articles").unwrap();

    // Search for "ryst" with fuzzy matching - shares "yst" with no direct match
    // but should find "rust" via n-gram overlap ("rus" from rust) and Levenshtein
    // Let's use "introduction" which is in article 1 - search for "introductoin" (typo)
    let results = collection
        .fulltext_search("title", "introductoin", 2)
        .unwrap();

    // Should find "introduction" via fuzzy match (distance 2: swap o and i)
    assert!(
        !results.is_empty(),
        "Expected fuzzy match for 'introductoin' -> 'introduction'"
    );
}

#[test]
fn test_fulltext_multiple_terms() {
    let (storage, _dir) = create_test_storage();
    setup_articles_collection(&storage);

    let collection = storage.get_collection("articles").unwrap();

    // Search for multiple terms
    let results = collection
        .fulltext_search("title", "rust programming", 1)
        .unwrap();

    // Should find articles matching either term
    assert!(!results.is_empty());
}

#[test]
fn test_fulltext_score_ordering() {
    let (storage, _dir) = create_test_storage();
    setup_articles_collection(&storage);

    let collection = storage.get_collection("articles").unwrap();

    let results = collection.fulltext_search("title", "rust", 1).unwrap();

    // Results should be ordered by score descending
    if results.len() >= 2 {
        for i in 0..results.len() - 1 {
            assert!(results[i].score >= results[i + 1].score);
        }
    }
}

#[test]
fn test_levenshtein_function() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Test LEVENSHTEIN function - need FOR clause
    let query = parse(r#"FOR doc IN users LIMIT 1 RETURN LEVENSHTEIN("hello", "hallo")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Distance should be 1 (one character different)
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(1));
}

#[test]
fn test_levenshtein_identical_strings() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(r#"FOR doc IN users LIMIT 1 RETURN LEVENSHTEIN("test", "test")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Distance should be 0 for identical strings
    assert_eq!(results[0], json!(0));
}

#[test]
fn test_levenshtein_empty_string() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(r#"FOR doc IN users LIMIT 1 RETURN LEVENSHTEIN("abc", "")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Distance should be length of non-empty string
    assert_eq!(results[0], json!(3));
}

#[test]
fn test_fulltext_aql_function() {
    let (storage, _dir) = create_test_storage();
    setup_articles_collection(&storage);

    // Use FULLTEXT function in AQL - need to iterate over result
    let query = parse(
        r#"
        LET matches = FULLTEXT("articles", "title", "rust")
        FOR m IN matches
        RETURN m
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert!(
        !results.is_empty(),
        "Expected FULLTEXT to return matches for 'rust'"
    );

    // Each match should have doc, score, and matched fields
    let first_match = &results[0];
    assert!(first_match.get("doc").is_some());
    assert!(first_match.get("score").is_some());
    assert!(first_match.get("matched").is_some());
}

#[test]
fn test_fulltext_with_max_distance() {
    let (storage, _dir) = create_test_storage();
    setup_articles_collection(&storage);

    // Use FULLTEXT function with custom max distance
    let query = parse(
        r#"
        LET matches = FULLTEXT("articles", "title", "pythn", 2)
        FOR m IN matches
        RETURN m
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Should find "python" with distance 2 from "pythn"
    assert!(
        !results.is_empty(),
        "Expected fuzzy match for 'pythn' -> 'python'"
    );
}

// ==================== Correlated Subquery Tests ====================

fn setup_orders_collection(storage: &StorageEngine) {
    storage.create_collection("orders".to_string()).unwrap();
    let collection = storage.get_collection("orders").unwrap();

    // Orders referencing users by name
    collection
        .insert(json!({
            "_key": "o1",
            "user": "Alice",
            "product": "Laptop",
            "amount": 1200
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "o2",
            "user": "Alice",
            "product": "Mouse",
            "amount": 50
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "o3",
            "user": "Bob",
            "product": "Keyboard",
            "amount": 100
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "o4",
            "user": "Charlie",
            "product": "Monitor",
            "amount": 500
        }))
        .unwrap();
}

#[test]
fn test_correlated_subquery_basic() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);
    setup_orders_collection(&storage);

    // For each user, get their orders using a correlated subquery
    let query = parse(
        r#"
        FOR u IN users
        LET userOrders = (FOR o IN orders FILTER o.user == u.name RETURN o.product)
        RETURN { name: u.name, orders: userOrders }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3); // 3 users

    // Find Alice's result
    let alice = results
        .iter()
        .find(|r| r["name"] == "Alice")
        .expect("Alice should be in results");
    let alice_orders = alice["orders"].as_array().unwrap();
    assert_eq!(alice_orders.len(), 2); // Alice has 2 orders
    assert!(alice_orders.contains(&json!("Laptop")));
    assert!(alice_orders.contains(&json!("Mouse")));

    // Find Bob's result
    let bob = results
        .iter()
        .find(|r| r["name"] == "Bob")
        .expect("Bob should be in results");
    let bob_orders = bob["orders"].as_array().unwrap();
    assert_eq!(bob_orders.len(), 1);
    assert!(bob_orders.contains(&json!("Keyboard")));
}

#[test]
fn test_correlated_subquery_with_aggregation() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);
    setup_orders_collection(&storage);

    // For each user, count their orders
    let query = parse(
        r#"
        FOR u IN users
        LET orderCount = LENGTH((FOR o IN orders FILTER o.user == u.name RETURN o))
        RETURN { name: u.name, orderCount: orderCount }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);

    let alice = results.iter().find(|r| r["name"] == "Alice").unwrap();
    assert_eq!(alice["orderCount"], 2);

    let bob = results.iter().find(|r| r["name"] == "Bob").unwrap();
    assert_eq!(bob["orderCount"], 1);

    let charlie = results.iter().find(|r| r["name"] == "Charlie").unwrap();
    assert_eq!(charlie["orderCount"], 1);
}

#[test]
fn test_correlated_subquery_with_sum() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);
    setup_orders_collection(&storage);

    // For each user, sum their order amounts
    let query = parse(
        r#"
        FOR u IN users
        LET totalSpent = SUM((FOR o IN orders FILTER o.user == u.name RETURN o.amount))
        RETURN { name: u.name, totalSpent: totalSpent }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    let alice = results.iter().find(|r| r["name"] == "Alice").unwrap();
    assert_eq!(alice["totalSpent"], 1250.0); // 1200 + 50

    let bob = results.iter().find(|r| r["name"] == "Bob").unwrap();
    assert_eq!(bob["totalSpent"], 100.0);
}

#[test]
fn test_correlated_subquery_empty_result() {
    let (storage, _dir) = create_test_storage();

    // Add a user with no orders
    storage.create_collection("users2".to_string()).unwrap();
    let users = storage.get_collection("users2").unwrap();
    users
        .insert(json!({"_key": "1", "name": "NoOrders"}))
        .unwrap();

    storage.create_collection("orders2".to_string()).unwrap();
    // No orders collection is empty

    let query = parse(
        r#"
        FOR u IN users2
        LET orders = (FOR o IN orders2 FILTER o.user == u.name RETURN o)
        RETURN { name: u.name, orders: orders }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["orders"], json!([])); // Empty array
}

#[test]
fn test_multiple_correlated_subqueries() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);
    setup_orders_collection(&storage);

    // Multiple LET clauses inside FOR
    let query = parse(
        r#"
        FOR u IN users
        LET orders = (FOR o IN orders FILTER o.user == u.name RETURN o.product)
        LET orderCount = LENGTH(orders)
        FILTER orderCount > 0
        RETURN { name: u.name, products: orders, count: orderCount }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3); // All users have at least 1 order

    for result in &results {
        let count = result["count"].as_f64().unwrap() as usize;
        let products = result["products"].as_array().unwrap();
        assert_eq!(products.len(), count);
    }
}

#[test]
fn test_correlated_subquery_nested_filter() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);
    setup_orders_collection(&storage);

    // Get users who spent more than 500 total
    let query = parse(
        r#"
        FOR u IN users
        LET totalSpent = SUM((FOR o IN orders FILTER o.user == u.name RETURN o.amount))
        FILTER totalSpent > 500
        RETURN { name: u.name, spent: totalSpent }
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Only Alice (1250) has spent more than 500
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["name"], "Alice");
}

#[test]
fn test_correlated_let_simple_expression() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // LET inside FOR with simple expression using outer variable
    let query = parse(
        r#"
        FOR u IN users
        LET greeting = CONCAT("Hello, ", u.name)
        RETURN greeting
    "#,
    )
    .unwrap();

    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);
    assert!(results.contains(&json!("Hello, Alice")));
    assert!(results.contains(&json!("Hello, Bob")));
    assert!(results.contains(&json!("Hello, Charlie")));
}

// ==================== COLLECTION_COUNT Function Tests ====================

#[test]
fn test_collection_count_basic() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(r#"RETURN COLLECTION_COUNT("users")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(3)); // 3 users in setup
}

#[test]
fn test_collection_count_empty_collection() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("empty".to_string()).unwrap();

    let query = parse(r#"RETURN COLLECTION_COUNT("empty")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(0));
}

#[test]
fn test_collection_count_multiple_collections() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);
    setup_orders_collection(&storage);

    let query = parse(
        r#"RETURN {
        users: COLLECTION_COUNT("users"),
        orders: COLLECTION_COUNT("orders")
    }"#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["users"], json!(3));
    assert_eq!(results[0]["orders"], json!(4));
}

#[test]
fn test_collection_count_with_bind_var() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(r#"RETURN COLLECTION_COUNT(@col)"#).unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("col".to_string(), json!("users"));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(3));
}

#[test]
fn test_collection_count_nonexistent_collection() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN COLLECTION_COUNT("nonexistent")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    // Should error because collection doesn't exist
    assert!(result.is_err());
}

#[test]
fn test_collection_count_in_for_loop() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Use COLLECTION_COUNT inside a FOR loop
    let query = parse(
        r#"
        FOR doc IN users
        LIMIT 1
        RETURN COLLECTION_COUNT("users")
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(3));
}

// ==================== DATE_TIMESTAMP Function Tests ====================

#[test]
fn test_date_timestamp_basic() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TIMESTAMP("2025-12-03T13:59:47.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(1764770387000_i64));
}

#[test]
fn test_date_timestamp_with_timezone() {
    let (storage, _dir) = create_test_storage();

    // ISO 8601 with timezone offset
    let query = parse(r#"RETURN DATE_TIMESTAMP("2025-12-03T14:59:47.000+01:00")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // Same moment in time as 13:59:47 UTC
    assert_eq!(results[0], json!(1764770387000_i64));
}

#[test]
fn test_date_timestamp_roundtrip() {
    let (storage, _dir) = create_test_storage();

    // Convert timestamp to ISO and back
    let query = parse(
        r#"
        LET ts = 1764770387000
        LET iso = DATE_ISO8601(ts)
        RETURN DATE_TIMESTAMP(iso)
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(1764770387000_i64));
}

#[test]
fn test_date_timestamp_invalid_format() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TIMESTAMP("not-a-date")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    // Should error because the date format is invalid
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("DATE_TIMESTAMP"));
}

#[test]
fn test_date_timestamp_with_milliseconds() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TIMESTAMP("2025-01-15T10:30:45.123Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // Verify the result is a number (timestamp in milliseconds)
    assert!(results[0].is_i64());
    let ts = results[0].as_i64().unwrap();
    // Should end in 123 for the milliseconds
    assert_eq!(ts % 1000, 123);
}

#[test]
fn test_date_now_returns_current_timestamp() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_NOW()"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    let ts = results[0].as_i64().unwrap();

    // Should be a reasonable timestamp (after 2024)
    assert!(ts > 1704067200000); // 2024-01-01 00:00:00 UTC
}

#[test]
fn test_date_iso8601_and_timestamp_consistency() {
    let (storage, _dir) = create_test_storage();

    // Get current time, convert to ISO, parse back
    let query = parse(
        r#"
        LET now = DATE_NOW()
        LET iso = DATE_ISO8601(now)
        LET back = DATE_TIMESTAMP(iso)
        RETURN { original: now, converted: back, match: now == back }
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["match"], json!(true));
}

#[test]
fn test_date_timestamp_epoch() {
    let (storage, _dir) = create_test_storage();

    // Unix epoch
    let query = parse(r#"RETURN DATE_TIMESTAMP("1970-01-01T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(0_i64));
}

// ==================== DATE_TRUNC Function Tests ====================

#[test]
fn test_date_trunc_year() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "year")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-01-01T00:00:00.000Z"));
}

#[test]
fn test_date_trunc_month() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "month")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-01T00:00:00.000Z"));
}

#[test]
fn test_date_trunc_day() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "day")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T00:00:00.000Z"));
}

#[test]
fn test_date_trunc_hour() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "hour")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T14:00:00.000Z"));
}

#[test]
fn test_date_trunc_minute() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "minute")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T14:30:00.000Z"));
}

#[test]
fn test_date_trunc_second() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "second")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T14:30:45.000Z"));
}

#[test]
fn test_date_trunc_millisecond() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "milliseconds")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T14:30:45.123Z"));
}

#[test]
fn test_date_trunc_short_units() {
    let (storage, _dir) = create_test_storage();

    // Test short unit names: y, m, d, h, i, s, f
    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "h")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("2025-06-15T14:00:00.000Z"));

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "i")"#).unwrap();
    let results = executor.execute(&query).unwrap();
    assert_eq!(results[0], json!("2025-06-15T14:30:00.000Z"));
}

#[test]
fn test_date_trunc_with_timestamp() {
    let (storage, _dir) = create_test_storage();

    // Use numeric timestamp instead of ISO string
    // 1750000000000 ms = July 15, 2025, 14:40:00 UTC
    let query = parse(r#"RETURN DATE_TRUNC(1750000000000, "day")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // Should truncate to start of day
    assert!(results[0].as_str().unwrap().contains("T00:00:00.000Z"));
}

#[test]
fn test_date_trunc_with_timezone() {
    let (storage, _dir) = create_test_storage();

    // 2025-06-15T20:30:00Z in New York (UTC-4 during DST) is 2025-06-15T16:30:00 local
    // Truncating to day in NY timezone should give 2025-06-15T00:00:00 NY = 2025-06-15T04:00:00Z
    let query =
        parse(r#"RETURN DATE_TRUNC("2025-06-15T20:30:00.000Z", "day", "America/New_York")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T04:00:00.000Z"));
}

#[test]
fn test_date_trunc_with_utc_timezone() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "day", "UTC")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T00:00:00.000Z"));
}

#[test]
fn test_date_trunc_europe_timezone() {
    let (storage, _dir) = create_test_storage();

    // 2025-06-15T01:30:00Z in Berlin (UTC+2 during DST) is 2025-06-15T03:30:00 local
    // Truncating to day in Berlin timezone should give 2025-06-15T00:00:00 Berlin = 2025-06-14T22:00:00Z
    let query =
        parse(r#"RETURN DATE_TRUNC("2025-06-15T01:30:00.000Z", "day", "Europe/Berlin")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-14T22:00:00.000Z"));
}

#[test]
fn test_date_trunc_invalid_unit() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "invalid")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown unit"));
}

#[test]
fn test_date_trunc_invalid_timezone() {
    let (storage, _dir) = create_test_storage();

    let query =
        parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "day", "Invalid/Timezone")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown timezone"));
}

#[test]
fn test_date_trunc_case_insensitive_unit() {
    let (storage, _dir) = create_test_storage();

    // Units should be case-insensitive
    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "YEAR")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("2025-01-01T00:00:00.000Z"));

    let query = parse(r#"RETURN DATE_TRUNC("2025-06-15T14:30:45.123Z", "Day")"#).unwrap();
    let results = executor.execute(&query).unwrap();
    assert_eq!(results[0], json!("2025-06-15T00:00:00.000Z"));
}

// ==================== DATE_FORMAT Function Tests ====================

#[test]
fn test_date_format_basic() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "%Y-%m-%d")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15"));
}

#[test]
fn test_date_format_time() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "%H:%M:%S")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("14:30:45"));
}

#[test]
fn test_date_format_full_datetime() {
    let (storage, _dir) = create_test_storage();

    let query =
        parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "%Y-%m-%d %H:%M:%S")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("2025-06-15 14:30:45"));
}

#[test]
fn test_date_format_weekday() {
    let (storage, _dir) = create_test_storage();

    // %A = full weekday name, %a = abbreviated
    let query = parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "%A")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("Sunday"));
}

#[test]
fn test_date_format_month_name() {
    let (storage, _dir) = create_test_storage();

    // %B = full month name, %b = abbreviated
    let query = parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "%B")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("June"));
}

#[test]
fn test_date_format_with_timezone() {
    let (storage, _dir) = create_test_storage();

    // 14:30 UTC should be 10:30 in New York (EDT, UTC-4)
    let query =
        parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "%H:%M", "America/New_York")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("10:30"));
}

#[test]
fn test_date_format_timezone_date_change() {
    let (storage, _dir) = create_test_storage();

    // 02:00 UTC on June 15 should be June 14 22:00 in New York (EDT, UTC-4)
    let query =
        parse(r#"RETURN DATE_FORMAT("2025-06-15T02:00:00.000Z", "%Y-%m-%d", "America/New_York")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("2025-06-14"));
}

#[test]
fn test_date_format_with_timestamp() {
    let (storage, _dir) = create_test_storage();

    // Use numeric timestamp
    let query = parse(r#"RETURN DATE_FORMAT(1750000000000, "%Y-%m-%d")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // Verify it's a valid date format
    assert!(results[0].as_str().unwrap().contains("-"));
}

#[test]
fn test_date_format_custom_format() {
    let (storage, _dir) = create_test_storage();

    // Custom format with text
    let query =
        parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "Date: %d/%m/%Y Time: %H:%M")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("Date: 15/06/2025 Time: 14:30"));
}

#[test]
fn test_date_format_iso_week() {
    let (storage, _dir) = create_test_storage();

    // %V = ISO week number, %G = ISO year
    let query =
        parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "Week %V of %G")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("Week 24 of 2025"));
}

#[test]
fn test_date_format_12_hour() {
    let (storage, _dir) = create_test_storage();

    // %I = 12-hour, %p = AM/PM
    let query = parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "%I:%M %p")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("02:30 PM"));
}

#[test]
fn test_date_format_day_of_year() {
    let (storage, _dir) = create_test_storage();

    // %j = day of year (001-366)
    let query = parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "%j")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("166")); // June 15 is day 166 of the year
}

#[test]
fn test_date_format_invalid_timezone() {
    let (storage, _dir) = create_test_storage();

    let query =
        parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "%Y-%m-%d", "Invalid/TZ")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown timezone"));
}

#[test]
fn test_date_format_europe_timezone() {
    let (storage, _dir) = create_test_storage();

    // 14:30 UTC should be 16:30 in Berlin (CEST, UTC+2)
    let query =
        parse(r#"RETURN DATE_FORMAT("2025-06-15T14:30:45.123Z", "%H:%M", "Europe/Berlin")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("16:30"));
}

// ==================== DATE_ISOWEEK Function Tests ====================

#[test]
fn test_date_isoweek_basic() {
    let (storage, _dir) = create_test_storage();

    // June 15, 2025 is in ISO week 24
    let query = parse(r#"RETURN DATE_ISOWEEK("2025-06-15T14:30:45.123Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(24));
}

#[test]
fn test_date_isoweek_first_week() {
    let (storage, _dir) = create_test_storage();

    // January 6, 2025 is in ISO week 2
    let query = parse(r#"RETURN DATE_ISOWEEK("2025-01-06T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(2));
}

#[test]
fn test_date_isoweek_last_week() {
    let (storage, _dir) = create_test_storage();

    // December 29, 2025 is in ISO week 1 of 2026 (or week 52/53 depending on year)
    let query = parse(r#"RETURN DATE_ISOWEEK("2025-12-29T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // December 29, 2025 is Monday of week 1 of 2026
    assert_eq!(results[0], json!(1));
}

#[test]
fn test_date_isoweek_with_timestamp() {
    let (storage, _dir) = create_test_storage();

    // Use numeric timestamp
    let query = parse(r#"RETURN DATE_ISOWEEK(1750000000000)"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // Should return a valid week number (1-53)
    let week = results[0].as_u64().unwrap();
    assert!(week >= 1 && week <= 53);
}

#[test]
fn test_date_isoweek_year_boundary() {
    let (storage, _dir) = create_test_storage();

    // January 1, 2025 is a Wednesday, so it's still in ISO week 1 of 2025
    let query = parse(r#"RETURN DATE_ISOWEEK("2025-01-01T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(1));
}

#[test]
fn test_date_isoweek_mid_year() {
    let (storage, _dir) = create_test_storage();

    // July 1, 2025 should be around week 27
    let query = parse(r#"RETURN DATE_ISOWEEK("2025-07-01T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(27));
}

// ==================== DATE_DAYOFYEAR Function Tests ====================

#[test]
fn test_date_dayofyear_basic() {
    let (storage, _dir) = create_test_storage();

    // June 15 is day 166 of the year (31+28+31+30+31+15 = 166)
    let query = parse(r#"RETURN DATE_DAYOFYEAR("2025-06-15T14:30:45.123Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(166));
}

#[test]
fn test_date_dayofyear_first_day() {
    let (storage, _dir) = create_test_storage();

    // January 1 is day 1
    let query = parse(r#"RETURN DATE_DAYOFYEAR("2025-01-01T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(1));
}

#[test]
fn test_date_dayofyear_last_day() {
    let (storage, _dir) = create_test_storage();

    // December 31, 2025 is day 365 (non-leap year)
    let query = parse(r#"RETURN DATE_DAYOFYEAR("2025-12-31T23:59:59.999Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(365));
}

#[test]
fn test_date_dayofyear_leap_year() {
    let (storage, _dir) = create_test_storage();

    // December 31, 2024 is day 366 (leap year)
    let query = parse(r#"RETURN DATE_DAYOFYEAR("2024-12-31T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(366));
}

#[test]
fn test_date_dayofyear_with_timestamp() {
    let (storage, _dir) = create_test_storage();

    // Use numeric timestamp
    let query = parse(r#"RETURN DATE_DAYOFYEAR(1750000000000)"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // Should return a valid day number (1-366)
    let day = results[0].as_u64().unwrap();
    assert!(day >= 1 && day <= 366);
}

#[test]
fn test_date_dayofyear_with_timezone() {
    let (storage, _dir) = create_test_storage();

    // 2025-01-01T02:00:00Z in New York (UTC-5 in winter) is still Dec 31, 2024
    let query =
        parse(r#"RETURN DATE_DAYOFYEAR("2025-01-01T02:00:00.000Z", "America/New_York")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // In New York it's still Dec 31, 2024 (leap year), so day 366
    assert_eq!(results[0], json!(366));
}

#[test]
fn test_date_dayofyear_timezone_europe() {
    let (storage, _dir) = create_test_storage();

    // 2025-06-15T22:00:00Z in Berlin (UTC+2 during DST) is June 16, 00:00
    let query =
        parse(r#"RETURN DATE_DAYOFYEAR("2025-06-15T22:00:00.000Z", "Europe/Berlin")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // In Berlin it's June 16, so day 167
    assert_eq!(results[0], json!(167));
}

#[test]
fn test_date_dayofyear_invalid_timezone() {
    let (storage, _dir) = create_test_storage();

    let query =
        parse(r#"RETURN DATE_DAYOFYEAR("2025-06-15T14:30:45.123Z", "Invalid/TZ")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown timezone"));
}

// ==================== DATE_DAYS_IN_MONTH Function Tests ====================

#[test]
fn test_date_days_in_month_january() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_DAYS_IN_MONTH("2025-01-15T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(31));
}

#[test]
fn test_date_days_in_month_february_non_leap() {
    let (storage, _dir) = create_test_storage();

    // 2025 is not a leap year
    let query = parse(r#"RETURN DATE_DAYS_IN_MONTH("2025-02-15T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(28));
}

#[test]
fn test_date_days_in_month_february_leap() {
    let (storage, _dir) = create_test_storage();

    // 2024 is a leap year
    let query = parse(r#"RETURN DATE_DAYS_IN_MONTH("2024-02-15T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(29));
}

#[test]
fn test_date_days_in_month_april() {
    let (storage, _dir) = create_test_storage();

    // April has 30 days
    let query = parse(r#"RETURN DATE_DAYS_IN_MONTH("2025-04-15T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(30));
}

#[test]
fn test_date_days_in_month_december() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_DAYS_IN_MONTH("2025-12-15T00:00:00.000Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(31));
}

#[test]
fn test_date_days_in_month_with_timestamp() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_DAYS_IN_MONTH(1750000000000)"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Should return a valid number of days (28-31)
    let days = results[0].as_u64().unwrap();
    assert!(days >= 28 && days <= 31);
}

#[test]
fn test_date_days_in_month_with_timezone() {
    let (storage, _dir) = create_test_storage();

    // 2025-02-01T02:00:00Z in New York (UTC-5) is still January 31
    // January has 31 days
    let query =
        parse(r#"RETURN DATE_DAYS_IN_MONTH("2025-02-01T02:00:00.000Z", "America/New_York")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // In New York it's still January, so 31 days
    assert_eq!(results[0], json!(31));
}

#[test]
fn test_date_days_in_month_timezone_february_leap() {
    let (storage, _dir) = create_test_storage();

    // 2024-03-01T02:00:00Z in New York (UTC-5) is still February 29 (leap year)
    let query =
        parse(r#"RETURN DATE_DAYS_IN_MONTH("2024-03-01T02:00:00.000Z", "America/New_York")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // In New York it's still February 2024 (leap year), so 29 days
    assert_eq!(results[0], json!(29));
}

#[test]
fn test_date_days_in_month_invalid_timezone() {
    let (storage, _dir) = create_test_storage();

    let query =
        parse(r#"RETURN DATE_DAYS_IN_MONTH("2025-06-15T14:30:45.123Z", "Invalid/TZ")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown timezone"));
}

// ==================== DATE_ADD Function Tests ====================

#[test]
fn test_date_add_years() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 3, "years")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2028-06-15T14:30:45.123Z"));
}

#[test]
fn test_date_add_months() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 3, "months")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-09-15T14:30:45.123Z"));
}

#[test]
fn test_date_add_weeks() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 2, "weeks")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-29T14:30:45.123Z"));
}

#[test]
fn test_date_add_days() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 10, "days")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-25T14:30:45.123Z"));
}

#[test]
fn test_date_add_hours() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 5, "hours")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T19:30:45.123Z"));
}

#[test]
fn test_date_add_minutes() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 45, "minutes")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T15:15:45.123Z"));
}

#[test]
fn test_date_add_seconds() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 30, "seconds")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T14:31:15.123Z"));
}

#[test]
fn test_date_add_milliseconds() {
    let (storage, _dir) = create_test_storage();

    let query =
        parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 500, "milliseconds")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T14:30:45.623Z"));
}

#[test]
fn test_date_add_negative_amount() {
    let (storage, _dir) = create_test_storage();

    // Subtract 7 days
    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", -7, "days")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-08T14:30:45.123Z"));
}

#[test]
fn test_date_add_negative_months() {
    let (storage, _dir) = create_test_storage();

    // Subtract 3 months
    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", -3, "months")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-03-15T14:30:45.123Z"));
}

#[test]
fn test_date_add_with_timestamp() {
    let (storage, _dir) = create_test_storage();

    // Use timestamp instead of ISO string
    // 1733234387000 ms = 2024-12-03T13:59:47.000Z
    let query = parse(r#"RETURN DATE_ADD(1733234387000, 2, "hours")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2024-12-03T15:59:47.000Z"));
}

#[test]
fn test_date_add_month_boundary() {
    let (storage, _dir) = create_test_storage();

    // Jan 31 + 1 month should give Feb 28 (or 29 in leap year)
    let query = parse(r#"RETURN DATE_ADD("2025-01-31T12:00:00.000Z", 1, "month")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-02-28T12:00:00.000Z"));
}

#[test]
fn test_date_add_month_boundary_leap_year() {
    let (storage, _dir) = create_test_storage();

    // Jan 31, 2024 + 1 month should give Feb 29 (leap year)
    let query = parse(r#"RETURN DATE_ADD("2024-01-31T12:00:00.000Z", 1, "month")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2024-02-29T12:00:00.000Z"));
}

#[test]
fn test_date_add_short_units() {
    let (storage, _dir) = create_test_storage();

    // Test short unit names
    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 1, "y")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2026-06-15T14:30:45.123Z"));
}

#[test]
fn test_date_add_case_insensitive() {
    let (storage, _dir) = create_test_storage();

    // Test case-insensitive unit matching
    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 5, "DAYS")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-20T14:30:45.123Z"));
}

#[test]
fn test_date_add_with_timezone() {
    let (storage, _dir) = create_test_storage();

    // Add 1 day with timezone
    let query =
        parse(r#"RETURN DATE_ADD("2025-06-15T20:00:00Z", 1, "day", "America/New_York")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // Should add 1 day in New York time, then convert back to UTC
    assert_eq!(results[0], json!("2025-06-16T20:00:00.000Z"));
}

#[test]
fn test_date_add_year_boundary() {
    let (storage, _dir) = create_test_storage();

    // Add months across year boundary
    let query = parse(r#"RETURN DATE_ADD("2025-11-15T12:00:00.000Z", 3, "months")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2026-02-15T12:00:00.000Z"));
}

#[test]
fn test_date_add_leap_year_feb_29() {
    let (storage, _dir) = create_test_storage();

    // Feb 29, 2024 + 1 year should give Feb 28, 2025 (not a leap year)
    let query = parse(r#"RETURN DATE_ADD("2024-02-29T12:00:00.000Z", 1, "year")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-02-28T12:00:00.000Z"));
}

#[test]
fn test_date_add_invalid_unit() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 5, "invalid")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown unit"));
}

#[test]
fn test_date_add_invalid_amount() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", "invalid", "days")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("amount must be a number"));
}

#[test]
fn test_date_add_invalid_date() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_ADD("invalid-date", 5, "days")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("invalid ISO 8601 date"));
}

#[test]
fn test_date_add_invalid_timezone() {
    let (storage, _dir) = create_test_storage();

    let query =
        parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 1, "day", "Invalid/Timezone")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown timezone"));
}

#[test]
fn test_date_add_zero_amount() {
    let (storage, _dir) = create_test_storage();

    // Adding 0 should return the same date
    let query = parse(r#"RETURN DATE_ADD("2025-06-15T14:30:45.123Z", 0, "days")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T14:30:45.123Z"));
}

#[test]
fn test_date_add_large_amount() {
    let (storage, _dir) = create_test_storage();

    // Add 1000 days
    let query = parse(r#"RETURN DATE_ADD("2025-01-01T00:00:00.000Z", 1000, "days")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2027-09-28T00:00:00.000Z"));
}

// ==================== DATE_SUBTRACT Function Tests ====================

#[test]
fn test_date_subtract_days() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_SUBTRACT("2025-06-15T14:30:45.123Z", 7, "days")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-08T14:30:45.123Z"));
}

#[test]
fn test_date_subtract_months() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_SUBTRACT("2025-06-15T14:30:45.123Z", 3, "months")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-03-15T14:30:45.123Z"));
}

#[test]
fn test_date_subtract_years() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_SUBTRACT("2025-06-15T14:30:45.123Z", 2, "years")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2023-06-15T14:30:45.123Z"));
}

#[test]
fn test_date_subtract_hours() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN DATE_SUBTRACT("2025-06-15T14:30:45.123Z", 5, "hours")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T09:30:45.123Z"));
}

#[test]
fn test_date_subtract_with_timestamp() {
    let (storage, _dir) = create_test_storage();

    // Use timestamp instead of ISO string
    let query = parse(r#"RETURN DATE_SUBTRACT(1733234387000, 1, "day")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2024-12-02T13:59:47.000Z"));
}

#[test]
fn test_date_subtract_with_timezone() {
    let (storage, _dir) = create_test_storage();

    let query =
        parse(r#"RETURN DATE_SUBTRACT("2025-06-15T20:00:00Z", 1, "day", "America/New_York")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-14T20:00:00.000Z"));
}

#[test]
fn test_date_subtract_month_boundary() {
    let (storage, _dir) = create_test_storage();

    // March 31 - 1 month should give Feb 28 (or 29 in leap year)
    let query = parse(r#"RETURN DATE_SUBTRACT("2025-03-31T12:00:00.000Z", 1, "month")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-02-28T12:00:00.000Z"));
}

#[test]
fn test_date_subtract_zero() {
    let (storage, _dir) = create_test_storage();

    // Subtracting 0 should return the same date
    let query = parse(r#"RETURN DATE_SUBTRACT("2025-06-15T14:30:45.123Z", 0, "days")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("2025-06-15T14:30:45.123Z"));
}

#[test]
fn test_date_subtract_invalid_amount() {
    let (storage, _dir) = create_test_storage();

    let query =
        parse(r#"RETURN DATE_SUBTRACT("2025-06-15T14:30:45.123Z", "invalid", "days")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("amount must be a number"));
}

// ==================== DATE_DIFF Function Tests ====================

#[test]
fn test_date_diff_days() {
    let (storage, _dir) = create_test_storage();

    // 10 days difference
    let query =
        parse(r#"RETURN DATE_DIFF("2025-06-01T00:00:00Z", "2025-06-11T00:00:00Z", "days")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(10.0));
}

#[test]
fn test_date_diff_hours() {
    let (storage, _dir) = create_test_storage();

    // 5 hours difference
    let query =
        parse(r#"RETURN DATE_DIFF("2025-06-15T10:00:00Z", "2025-06-15T15:00:00Z", "hours")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(5.0));
}

#[test]
fn test_date_diff_months() {
    let (storage, _dir) = create_test_storage();

    // 3 months difference
    let query =
        parse(r#"RETURN DATE_DIFF("2025-03-15T00:00:00Z", "2025-06-15T00:00:00Z", "months")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(3.0));
}

#[test]
fn test_date_diff_years() {
    let (storage, _dir) = create_test_storage();

    // 2 years difference
    let query =
        parse(r#"RETURN DATE_DIFF("2023-06-15T00:00:00Z", "2025-06-15T00:00:00Z", "years")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(2.0));
}

#[test]
fn test_date_diff_negative() {
    let (storage, _dir) = create_test_storage();

    // Negative difference (date2 before date1)
    let query =
        parse(r#"RETURN DATE_DIFF("2025-06-15T00:00:00Z", "2025-06-10T00:00:00Z", "days")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(-5.0));
}

#[test]
fn test_date_diff_with_float() {
    let (storage, _dir) = create_test_storage();

    // With asFloat=true for decimal precision
    let query =
        parse(r#"RETURN DATE_DIFF("2025-06-15T00:00:00Z", "2025-06-15T12:00:00Z", "days", true)"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    let result = results[0].as_f64().unwrap();
    assert!((result - 0.5).abs() < 0.01); // 12 hours = 0.5 days
}

#[test]
fn test_date_diff_milliseconds() {
    let (storage, _dir) = create_test_storage();

    // Millisecond difference
    let query = parse(r#"RETURN DATE_DIFF("2025-06-15T14:30:45.000Z", "2025-06-15T14:30:45.500Z", "milliseconds")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(500.0));
}

#[test]
fn test_date_diff_with_timestamps() {
    let (storage, _dir) = create_test_storage();

    // Using numeric timestamps
    // 1 day = 86400000 ms
    let query = parse(r#"RETURN DATE_DIFF(1000000000000, 1000086400000, "days")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(1.0));
}

#[test]
fn test_date_diff_with_timezone() {
    let (storage, _dir) = create_test_storage();

    // Same UTC time but different local times due to timezone
    let query = parse(r#"RETURN DATE_DIFF("2025-06-15T00:00:00Z", "2025-06-16T00:00:00Z", "days", false, "America/New_York")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(1.0));
}

#[test]
fn test_date_diff_minutes() {
    let (storage, _dir) = create_test_storage();

    // 90 minutes difference
    let query =
        parse(r#"RETURN DATE_DIFF("2025-06-15T14:00:00Z", "2025-06-15T15:30:00Z", "minutes")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(90.0));
}

#[test]
fn test_date_diff_seconds() {
    let (storage, _dir) = create_test_storage();

    // 120 seconds difference
    let query =
        parse(r#"RETURN DATE_DIFF("2025-06-15T14:30:00Z", "2025-06-15T14:32:00Z", "seconds")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(120.0));
}

#[test]
fn test_date_diff_weeks() {
    let (storage, _dir) = create_test_storage();

    // 2 weeks difference
    let query =
        parse(r#"RETURN DATE_DIFF("2025-06-01T00:00:00Z", "2025-06-15T00:00:00Z", "weeks")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(2.0));
}

#[test]
fn test_date_diff_zero() {
    let (storage, _dir) = create_test_storage();

    // Same date
    let query =
        parse(r#"RETURN DATE_DIFF("2025-06-15T14:30:00Z", "2025-06-15T14:30:00Z", "days")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(0.0));
}

#[test]
fn test_date_diff_invalid_unit() {
    let (storage, _dir) = create_test_storage();

    let query =
        parse(r#"RETURN DATE_DIFF("2025-06-15T00:00:00Z", "2025-06-16T00:00:00Z", "invalid")"#)
            .unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query);

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("unknown unit"));
}

// ==================== INSERT Statement Tests ====================

#[test]
fn test_insert_simple() {
    let (storage, _dir) = create_test_storage();
    storage.create_database("testdb".to_string()).unwrap();
    storage.create_collection("numbers".to_string()).unwrap();

    // Insert documents using FOR loop with LET array
    let query = parse(
        r#"
        LET nums = [1, 2, 3, 4, 5]
        FOR i IN nums
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
    storage.create_database("testdb".to_string()).unwrap();
    storage.create_collection("users".to_string()).unwrap();

    // Insert with object construction
    let query = parse(
        r#"
        LET nums = [1, 2, 3]
        FOR i IN nums
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

// ==================== UPDATE Statement Tests ====================

#[test]
fn test_update_simple() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Get Alice's document and update it
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == "Alice"
        UPDATE doc WITH { status: "premium" } IN users
        RETURN doc.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));

    // Verify the update
    let collection = storage.get_collection("users").unwrap();
    let alice = collection.get("alice").unwrap();
    assert_eq!(alice.to_value()["status"], json!("premium"));
    // Original fields should still exist
    assert_eq!(alice.to_value()["name"], json!("Alice"));
    assert_eq!(alice.to_value()["age"], json!(30));
}

#[test]
fn test_update_multiple_fields() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Update multiple fields
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == "Bob"
        UPDATE doc WITH { status: "vip", level: 5, verified: true } IN users
        RETURN doc.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);

    // Verify all new fields
    let collection = storage.get_collection("users").unwrap();
    let bob = collection.get("bob").unwrap();
    assert_eq!(bob.to_value()["status"], json!("vip"));
    assert_eq!(bob.to_value()["level"], json!(5.0));
    assert_eq!(bob.to_value()["verified"], json!(true));
}

#[test]
fn test_update_overwrite_field() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Update (overwrite) an existing field
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == "Charlie"
        UPDATE doc WITH { age: 40 } IN users
        RETURN doc.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);

    // Verify the field was overwritten
    let collection = storage.get_collection("users").unwrap();
    let charlie = collection.get("charlie").unwrap();
    assert_eq!(charlie.to_value()["age"], json!(40.0)); // Was 35, now 40
                                                        // Original name should still exist
    assert_eq!(charlie.to_value()["name"], json!("Charlie"));
}

#[test]
fn test_update_multiple_documents() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Update all users from Paris
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.city == "Paris"
        UPDATE doc WITH { region: "Europe" } IN users
        RETURN doc.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2); // Alice and Charlie are from Paris

    // Verify both documents were updated
    let collection = storage.get_collection("users").unwrap();
    let alice = collection.get("alice").unwrap();
    let charlie = collection.get("charlie").unwrap();
    assert_eq!(alice.to_value()["region"], json!("Europe"));
    assert_eq!(charlie.to_value()["region"], json!("Europe"));
}

#[test]
fn test_update_with_expression() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Update with computed value using CONCAT
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == "Alice"
        UPDATE doc WITH { fullName: CONCAT(doc.name, " from ", doc.city) } IN users
        RETURN doc.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);

    // Verify the computed field
    let collection = storage.get_collection("users").unwrap();
    let alice = collection.get("alice").unwrap();
    assert_eq!(alice.to_value()["fullName"], json!("Alice from Paris"));
}

#[test]
fn test_update_with_bind_vars() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == @name
        UPDATE doc WITH { points: @points } IN users
        RETURN doc.name
    "#,
    )
    .unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("name".to_string(), json!("Alice"));
    bind_vars.insert("points".to_string(), json!(100));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);

    // Verify the update with bind var value
    let collection = storage.get_collection("users").unwrap();
    let alice = collection.get("alice").unwrap();
    assert_eq!(alice.to_value()["points"], json!(100));
}

#[test]
fn test_update_no_match() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Filter matches no documents - update should not fail
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == "NonExistent"
        UPDATE doc WITH { status: "updated" } IN users
        RETURN doc.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 0); // No documents matched
}

#[test]
fn test_update_preserves_key() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Update should preserve _key
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == "Alice"
        UPDATE doc WITH { name: "Alice Updated" } IN users
        RETURN doc._key
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("alice"));

    // Verify the document still exists with same key
    let collection = storage.get_collection("users").unwrap();
    let alice = collection.get("alice").unwrap();
    assert_eq!(alice.to_value()["name"], json!("Alice Updated"));
    assert_eq!(alice.to_value()["_key"], json!("alice"));
}

// ==================== Range Expression Tests ====================

#[test]
fn test_range_basic() {
    let (storage, _dir) = create_test_storage();

    // Basic range 1..5 should produce [1, 2, 3, 4, 5]
    let query = parse(r#"RETURN 1..5"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!([1, 2, 3, 4, 5]));
}

#[test]
fn test_range_single_element() {
    let (storage, _dir) = create_test_storage();

    // Range 3..3 should produce [3]
    let query = parse(r#"RETURN 3..3"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!([3]));
}

#[test]
fn test_range_negative_numbers() {
    let (storage, _dir) = create_test_storage();

    // Range with negative numbers
    let query = parse(r#"RETURN -2..2"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!([-2, -1, 0, 1, 2]));
}

#[test]
fn test_range_empty() {
    let (storage, _dir) = create_test_storage();

    // Range 5..3 should produce empty array (start > end)
    let query = parse(r#"RETURN 5..3"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!([]));
}

#[test]
fn test_range_in_for_loop() {
    let (storage, _dir) = create_test_storage();

    // Use range in FOR loop
    let query = parse(
        r#"
        FOR i IN 1..5
        RETURN i * 2
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 5);
    assert_eq!(results[0], json!(2.0));
    assert_eq!(results[1], json!(4.0));
    assert_eq!(results[2], json!(6.0));
    assert_eq!(results[3], json!(8.0));
    assert_eq!(results[4], json!(10.0));
}

#[test]
fn test_range_with_expressions() {
    let (storage, _dir) = create_test_storage();

    // Range with expressions
    let query = parse(
        r#"
        LET start = 2
        LET end = 6
        RETURN start..end
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!([2, 3, 4, 5, 6]));
}

#[test]
fn test_range_with_arithmetic() {
    let (storage, _dir) = create_test_storage();

    // Range with arithmetic expressions
    let query = parse(r#"RETURN (1 + 1)..(3 * 2)"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!([2, 3, 4, 5, 6]));
}

#[test]
fn test_range_for_insert() {
    let (storage, _dir) = create_test_storage();
    storage.create_database("testdb".to_string()).unwrap();
    storage.create_collection("items".to_string()).unwrap();

    // Use range in FOR loop for insert
    let query = parse(
        r#"
        FOR i IN 1..3
        INSERT { index: i, name: CONCAT("Item", i) } INTO items
        RETURN i
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);

    // Verify documents were inserted
    let collection = storage.get_collection("items").unwrap();
    assert_eq!(collection.count(), 3);
}

// ==================== REMOVE Statement Tests ====================

#[test]
fn test_remove_simple() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Verify we have users before
    let collection = storage.get_collection("users").unwrap();
    let count_before = collection.count();
    assert!(count_before > 0);

    // Remove Alice
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == "Alice"
        REMOVE doc IN users
        RETURN doc.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Alice"));

    // Verify Alice was deleted
    let collection = storage.get_collection("users").unwrap();
    assert_eq!(collection.count(), count_before - 1);
    assert!(collection.get("alice").is_err());
}

#[test]
fn test_remove_multiple_documents() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Remove all users from Paris (Alice and Charlie)
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.city == "Paris"
        REMOVE doc IN users
        RETURN doc.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);

    // Verify they were deleted
    let collection = storage.get_collection("users").unwrap();
    assert!(collection.get("alice").is_err());
    assert!(collection.get("charlie").is_err());
    // Bob should still exist
    assert!(collection.get("bob").is_ok());
}

#[test]
fn test_remove_by_key() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Remove by key string
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc._key == "bob"
        REMOVE doc IN users
        RETURN doc._key
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("bob"));

    // Verify Bob was deleted
    let collection = storage.get_collection("users").unwrap();
    assert!(collection.get("bob").is_err());
}

#[test]
fn test_remove_no_match() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let collection = storage.get_collection("users").unwrap();
    let count_before = collection.count();

    // Filter matches no documents - remove should not fail
    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == "NonExistent"
        REMOVE doc IN users
        RETURN doc.name
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 0);

    // Count should be unchanged
    let collection = storage.get_collection("users").unwrap();
    assert_eq!(collection.count(), count_before);
}

#[test]
fn test_remove_with_bind_vars() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    let query = parse(
        r#"
        FOR doc IN users
        FILTER doc.name == @name
        REMOVE doc IN users
        RETURN doc.name
    "#,
    )
    .unwrap();

    let mut bind_vars: BindVars = HashMap::new();
    bind_vars.insert("name".to_string(), json!("Charlie"));

    let executor = QueryExecutor::with_bind_vars(&storage, bind_vars);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Charlie"));

    // Verify Charlie was deleted
    let collection = storage.get_collection("users").unwrap();
    assert!(collection.get("charlie").is_err());
}

#[test]
fn test_remove_all() {
    let (storage, _dir) = create_test_storage();
    setup_users_collection(&storage);

    // Remove all documents
    let query = parse(
        r#"
        FOR doc IN users
        REMOVE doc IN users
        RETURN doc._key
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3); // Alice, Bob, Charlie

    // Verify collection is empty
    let collection = storage.get_collection("users").unwrap();
    assert_eq!(collection.count(), 0);
}

// ==================== BM25 Scoring Tests ====================

#[test]
fn test_bm25_basic_scoring() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("articles".to_string()).unwrap();
    let collection = storage.get_collection("articles").unwrap();

    collection
        .insert(json!({
            "_key": "1",
            "title": "Introduction to Machine Learning",
            "content": "Machine learning is a subset of artificial intelligence"
        }))
        .unwrap();

    let query = parse(
        r#"
        FOR doc IN articles
        RETURN BM25(doc.content, "machine learning")
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // BM25 score should be positive for matching terms
    assert!(results[0].as_f64().unwrap() > 0.0);
}

#[test]
fn test_bm25_sort_descending() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("articles".to_string()).unwrap();
    let collection = storage.get_collection("articles").unwrap();

    collection
        .insert(json!({
            "_key": "1",
            "title": "ML Basics",
            "content": "Machine learning and artificial intelligence"
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "2",
            "title": "Advanced ML",
            "content": "Machine learning machine learning deep learning neural networks"
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "3",
            "title": "Other Topic",
            "content": "This is about something completely different"
        }))
        .unwrap();

    let query = parse(
        r#"
        FOR doc IN articles
        SORT BM25(doc.content, "machine learning") DESC
        RETURN {title: doc.title, score: BM25(doc.content, "machine learning")}
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);
    // Document 2 should have highest score (more occurrences of "machine learning")
    assert_eq!(results[0]["title"], json!("Advanced ML"));
    // Document 3 should have lowest score (no matching terms)
    assert_eq!(results[2]["title"], json!("Other Topic"));
}

#[test]
fn test_bm25_with_limit() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("articles".to_string()).unwrap();
    let collection = storage.get_collection("articles").unwrap();

    for i in 1..=10 {
        collection
            .insert(json!({
                "_key": i.to_string(),
                "content": format!("Article {} about machine learning", i)
            }))
            .unwrap();
    }

    let query = parse(
        r#"
        FOR doc IN articles
        SORT BM25(doc.content, "machine learning") DESC
        LIMIT 3
        RETURN doc._key
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);
}

#[test]
fn test_bm25_no_matches() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("articles".to_string()).unwrap();
    let collection = storage.get_collection("articles").unwrap();

    collection
        .insert(json!({
            "_key": "1",
            "content": "This is about databases and storage"
        }))
        .unwrap();

    let query = parse(
        r#"
        FOR doc IN articles
        RETURN BM25(doc.content, "machine learning")
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // Score should be 0 for no matching terms
    assert_eq!(results[0].as_f64().unwrap(), 0.0);
}

#[test]
fn test_bm25_empty_query() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("articles".to_string()).unwrap();
    let collection = storage.get_collection("articles").unwrap();

    collection
        .insert(json!({
            "_key": "1",
            "content": "Some content here"
        }))
        .unwrap();

    let query = parse(
        r#"
        FOR doc IN articles
        RETURN BM25(doc.content, "")
    "#,
    )
    .unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 1);
    // Empty query should return 0 score
    assert_eq!(results[0].as_f64().unwrap(), 0.0);
}
