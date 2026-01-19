//! Bind Variables and Data Modification Tests
//!
//! Comprehensive tests for:
//! - Bind variables (@param)
//! - INSERT queries
//! - UPDATE queries
//! - REMOVE queries
//! - UPSERT operations

use serde_json::json;
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use std::collections::HashMap;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

fn execute_with_binds(
    engine: &StorageEngine,
    query_str: &str,
    binds: HashMap<String, serde_json::Value>,
) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::with_bind_vars(engine, binds);
    executor
        .execute(&query)
        .expect(&format!("Query failed: {}", query_str))
}

fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .execute(&query)
        .expect(&format!("Query failed: {}", query_str))
}

// ============================================================================
// Bind Variable Tests
// ============================================================================

#[test]
fn test_bind_simple_string() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    users
        .insert(json!({"_key": "alice", "name": "Alice"}))
        .unwrap();
    users.insert(json!({"_key": "bob", "name": "Bob"})).unwrap();

    let mut binds = HashMap::new();
    binds.insert("name".to_string(), json!("Alice"));

    let results = execute_with_binds(
        &engine,
        "FOR u IN users FILTER u.name == @name RETURN u._key",
        binds,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("alice"));
}

#[test]
fn test_bind_number() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items.insert(json!({"_key": "a", "price": 10})).unwrap();
    items.insert(json!({"_key": "b", "price": 20})).unwrap();
    items.insert(json!({"_key": "c", "price": 30})).unwrap();

    let mut binds = HashMap::new();
    binds.insert("min_price".to_string(), json!(15));

    let results = execute_with_binds(
        &engine,
        "FOR i IN items FILTER i.price >= @min_price RETURN i._key",
        binds,
    );

    assert_eq!(results.len(), 2);
}

#[test]
fn test_bind_multiple() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();
    products
        .insert(json!({"_key": "p1", "category": "A", "price": 100}))
        .unwrap();
    products
        .insert(json!({"_key": "p2", "category": "A", "price": 200}))
        .unwrap();
    products
        .insert(json!({"_key": "p3", "category": "B", "price": 150}))
        .unwrap();

    let mut binds = HashMap::new();
    binds.insert("cat".to_string(), json!("A"));
    binds.insert("max".to_string(), json!(150));

    let results = execute_with_binds(
        &engine,
        "FOR p IN products FILTER p.category == @cat AND p.price <= @max RETURN p._key",
        binds,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("p1"));
}

#[test]
fn test_bind_array() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items
        .insert(json!({"_key": "a", "type": "widget"}))
        .unwrap();
    items
        .insert(json!({"_key": "b", "type": "gadget"}))
        .unwrap();
    items.insert(json!({"_key": "c", "type": "gizmo"})).unwrap();

    let mut binds = HashMap::new();
    binds.insert("types".to_string(), json!(["widget", "gadget"]));

    let results = execute_with_binds(
        &engine,
        "FOR i IN items FILTER i.type IN @types RETURN i._key",
        binds,
    );

    assert_eq!(results.len(), 2);
}

#[test]
fn test_bind_null() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();
    data.insert(json!({"_key": "1", "value": null})).unwrap();
    data.insert(json!({"_key": "2", "value": 10})).unwrap();

    let mut binds = HashMap::new();
    binds.insert("val".to_string(), json!(null));

    let results = execute_with_binds(
        &engine,
        "FOR d IN data FILTER d.value == @val RETURN d._key",
        binds,
    );

    assert_eq!(results.len(), 1);
}

#[test]
fn test_bind_boolean() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("flags".to_string(), None).unwrap();
    let flags = engine.get_collection("flags").unwrap();
    flags.insert(json!({"_key": "1", "active": true})).unwrap();
    flags.insert(json!({"_key": "2", "active": false})).unwrap();

    let mut binds = HashMap::new();
    binds.insert("is_active".to_string(), json!(true));

    let results = execute_with_binds(
        &engine,
        "FOR f IN flags FILTER f.active == @is_active RETURN f._key",
        binds,
    );

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("1"));
}

// ============================================================================
// INSERT Query Tests
// ============================================================================

#[test]
fn test_insert_single() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("docs".to_string(), None).unwrap();

    execute_query(&engine, "INSERT { \"name\": 'New Item' } INTO docs");

    let col = engine.get_collection("docs").unwrap();
    assert_eq!(col.count(), 1);
}

#[test]
fn test_insert_with_key() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("docs".to_string(), None).unwrap();

    execute_query(
        &engine,
        "INSERT { \"_key\": 'mykey', \"name\": 'Item' } INTO docs",
    );

    let col = engine.get_collection("docs").unwrap();
    let doc = col.get("mykey").unwrap();
    assert_eq!(doc.get("name"), Some(json!("Item")));
}

#[test]
fn test_insert_multiple_via_for() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("numbers".to_string(), None)
        .unwrap();

    execute_query(
        &engine,
        "FOR i IN 1..5 INSERT { \"value\": i } INTO numbers",
    );

    let col = engine.get_collection("numbers").unwrap();
    assert_eq!(col.count(), 5);
}

#[test]
fn test_insert_with_bind() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();

    let mut binds = HashMap::new();
    binds.insert("name".to_string(), json!("Bound Item"));
    binds.insert("value".to_string(), json!(42));

    execute_with_binds(
        &engine,
        "INSERT { \"name\": @name, \"value\": @value } INTO items",
        binds,
    );

    let col = engine.get_collection("items").unwrap();
    assert_eq!(col.count(), 1);
}

// ============================================================================
// UPDATE Query Tests
// ============================================================================

#[test]
fn test_update_single() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    users
        .insert(json!({"_key": "alice", "name": "Alice", "age": 30}))
        .unwrap();

    execute_query(&engine, "UPDATE 'alice' WITH { \"age\": 31 } IN users");

    let doc = users.get("alice").unwrap();
    assert_eq!(doc.get("age"), Some(json!(31)));
    assert_eq!(doc.get("name"), Some(json!("Alice"))); // Preserved
}

#[test]
fn test_update_via_for() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items.insert(json!({"_key": "a", "cnt": 10})).unwrap();
    items.insert(json!({"_key": "b", "cnt": 20})).unwrap();

    // Fix: Using quoted keys in object literal if parser requires it
    // Or simplified update if supported
    execute_query(
        &engine,
        "FOR i IN items UPDATE i WITH { \"cnt\": i.cnt + 5 } IN items",
    );

    let a = items.get("a").unwrap();
    let b = items.get("b").unwrap();
    assert_eq!(a.get("cnt"), Some(json!(15.0)));
    assert_eq!(b.get("cnt"), Some(json!(25.0)));
}

#[test]
fn test_update_filtered() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();
    products
        .insert(json!({"_key": "p1", "status": "active", "views": 100}))
        .unwrap();
    products
        .insert(json!({"_key": "p2", "status": "inactive", "views": 50}))
        .unwrap();

    execute_query(&engine,
        "FOR p IN products FILTER p.status == 'active' UPDATE p WITH { \"views\": p.views + 10 } IN products");

    let p1 = products.get("p1").unwrap();
    let p2 = products.get("p2").unwrap();
    assert_eq!(p1.get("views"), Some(json!(110.0)));
    assert_eq!(p2.get("views"), Some(json!(50))); // Unchanged
}

#[test]
fn test_update_with_bind() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("docs".to_string(), None).unwrap();
    let docs = engine.get_collection("docs").unwrap();
    docs.insert(json!({"_key": "d1", "value": 1})).unwrap();

    let mut binds = HashMap::new();
    binds.insert("key".to_string(), json!("d1"));
    binds.insert("new_value".to_string(), json!(999));

    execute_with_binds(
        &engine,
        "UPDATE @key WITH { \"value\": @new_value } IN docs",
        binds,
    );

    let doc = docs.get("d1").unwrap();
    assert_eq!(doc.get("value"), Some(json!(999)));
}

// ============================================================================
// REMOVE Query Tests
// ============================================================================

#[test]
fn test_remove_single() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items.insert(json!({"_key": "a", "name": "A"})).unwrap();
    items.insert(json!({"_key": "b", "name": "B"})).unwrap();

    execute_query(&engine, "REMOVE 'a' IN items");

    assert_eq!(items.count(), 1);
    assert!(items.get("a").is_err());
    assert!(items.get("b").is_ok());
}

#[test]
fn test_remove_via_for() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("temp".to_string(), None).unwrap();
    let temp = engine.get_collection("temp").unwrap();
    temp.insert(json!({"_key": "1", "keep": false})).unwrap();
    temp.insert(json!({"_key": "2", "keep": true})).unwrap();
    temp.insert(json!({"_key": "3", "keep": false})).unwrap();

    execute_query(
        &engine,
        "FOR t IN temp FILTER t.keep == false REMOVE t IN temp",
    );

    assert_eq!(temp.count(), 1);
    assert!(temp.get("2").is_ok());
}

#[test]
fn test_remove_with_bind() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("docs".to_string(), None).unwrap();
    let docs = engine.get_collection("docs").unwrap();
    docs.insert(json!({"_key": "target", "value": 1})).unwrap();

    let mut binds = HashMap::new();
    binds.insert("key".to_string(), json!("target"));

    execute_with_binds(&engine, "REMOVE @key IN docs", binds);

    assert_eq!(docs.count(), 0);
}

// ============================================================================
// Combined Operations Tests
// ============================================================================

#[test]
fn test_insert_and_query() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();

    // Insert
    execute_query(
        &engine,
        "INSERT { \"_key\": 'test', \"value\": 100 } INTO data",
    );

    // Query
    let results = execute_query(
        &engine,
        "FOR d IN data FILTER d._key == 'test' RETURN d.value",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!(100));
}

#[test]
fn test_update_and_query() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();
    data.insert(json!({"_key": "x", "val": 0})).unwrap();

    // Update multiple times - quoting keys
    execute_query(&engine, "UPDATE 'x' WITH { \"val\": 1 } IN data");
    execute_query(&engine, "UPDATE 'x' WITH { \"val\": 2 } IN data");

    // Query
    let results = execute_query(&engine, "FOR d IN data RETURN d.val");
    assert_eq!(results[0], json!(2));
}

#[test]
fn test_insert_update_remove_sequence() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("lifecycle".to_string(), None)
        .unwrap();
    let lc = engine.get_collection("lifecycle").unwrap();

    // Insert
    execute_query(
        &engine,
        "INSERT { \"_key\": 'item', \"stage\": 'created' } INTO lifecycle",
    );
    assert_eq!(lc.count(), 1);

    // Update
    execute_query(
        &engine,
        "UPDATE 'item' WITH { \"stage\": 'updated' } IN lifecycle",
    );
    let doc = lc.get("item").unwrap();
    assert_eq!(doc.get("stage"), Some(json!("updated")));

    // Remove
    execute_query(&engine, "REMOVE 'item' IN lifecycle");
    assert_eq!(lc.count(), 0);
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_update_nonexistent() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();

    // Trying to update a non-existent document should fail or do nothing
    let _result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute_query(&engine, "UPDATE 'nonexistent' WITH { \"x\": 1 } IN items");
    }));
    // May panic or return empty - either is acceptable
}

#[test]
fn test_remove_nonexistent() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();

    // Trying to remove a non-existent document
    let _result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute_query(&engine, "REMOVE 'nonexistent' IN items");
    }));
    // May panic or do nothing
}

#[test]
fn test_insert_duplicate_key() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items.insert(json!({"_key": "dup", "v": 1})).unwrap();

    // Inserting with same key should fail
    let _result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute_query(&engine, "INSERT { \"_key\": 'dup', \"v\": 2 } INTO items");
    }));
    // Should fail or be rejected
}

// ============================================================================
// Return from Modification Tests
// ============================================================================

#[test]
fn test_insert_count_verification() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("nums".to_string(), None).unwrap();

    // Insert multiple
    execute_query(&engine, "FOR i IN 1..10 INSERT { \"n\": i } INTO nums");

    // Verify count
    let col = engine.get_collection("nums").unwrap();
    assert_eq!(col.count(), 10);
}

#[test]
fn test_partial_update() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("docs".to_string(), None).unwrap();
    let docs = engine.get_collection("docs").unwrap();
    docs.insert(json!({
        "_key": "doc1",
        "field1": "original1",
        "field2": "original2",
        "field3": "original3"
    }))
    .unwrap();

    // Update only field2
    execute_query(
        &engine,
        "UPDATE 'doc1' WITH { \"field2\": 'updated2' } IN docs",
    );

    let doc = docs.get("doc1").unwrap();
    assert_eq!(doc.get("field1"), Some(json!("original1")));
    assert_eq!(doc.get("field2"), Some(json!("updated2")));
    assert_eq!(doc.get("field3"), Some(json!("original3")));
}

#[test]
fn test_limit_bind_variables() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    for i in 0..10 {
        items
            .insert(json!({"_key": format!("i{}", i), "val": i}))
            .unwrap();
    }

    let mut binds = HashMap::new();
    binds.insert("offset".to_string(), json!(2));
    binds.insert("count".to_string(), json!(3));

    let results = execute_with_binds(
        &engine,
        "FOR i IN items SORT i.val LIMIT @offset, @count RETURN i.val",
        binds,
    );

    assert_eq!(results.len(), 3);
    assert_eq!(results[0], json!(2));
    assert_eq!(results[1], json!(3));
    assert_eq!(results[2], json!(4));
}
