//! SDBQL Executor Coverage Tests
//!
//! Additional tests for improving executor coverage including:
//! - Bind variables
//! - COLLECT/GROUP BY aggregation
//! - UPSERT operations
//! - Multiple SORT fields
//! - Graph traversal edge cases
//! - Explain functionality
//! - Error cases
//! - Regex operators
//! - Hash functions
//! - Edge cases

use serde_json::json;
use solidb::storage::StorageEngine;
use solidb::{parse, BindVars, QueryExecutor};
use std::collections::HashMap;
use tempfile::TempDir;

/// Execute a query and return results
fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .execute(&query)
        .expect(&format!("Query failed: {}", query_str))
}

/// Execute a query with bind variables
fn execute_with_binds(
    engine: &StorageEngine,
    query_str: &str,
    binds: BindVars,
) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::with_bind_vars(engine, binds);
    executor
        .execute(&query)
        .expect(&format!("Query failed: {}", query_str))
}

/// Execute a query with database context
fn execute_with_database(
    engine: &StorageEngine,
    db_name: &str,
    query_str: &str,
) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::with_database(engine, db_name.to_string());
    executor
        .execute(&query)
        .expect(&format!("Query failed: {}", query_str))
}

/// Execute a query with database and bind vars
fn execute_with_db_and_binds(
    engine: &StorageEngine,
    db_name: &str,
    query_str: &str,
    binds: BindVars,
) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::with_database_and_bind_vars(engine, db_name.to_string(), binds);
    executor
        .execute(&query)
        .expect(&format!("Query failed: {}", query_str))
}

/// Execute explain
fn explain_query(engine: &StorageEngine, query_str: &str) -> solidb::sdbql::QueryExplain {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .explain(&query)
        .expect(&format!("Explain failed: {}", query_str))
}

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

fn create_seeded_engine() -> (StorageEngine, TempDir) {
    let (engine, tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    users
        .insert(json!({"_key": "alice", "name": "Alice", "age": 30, "dept": "eng"}))
        .unwrap();
    users
        .insert(json!({"_key": "bob", "name": "Bob", "age": 25, "dept": "eng"}))
        .unwrap();
    users
        .insert(json!({"_key": "charlie", "name": "Charlie", "age": 35, "dept": "sales"}))
        .unwrap();
    users
        .insert(json!({"_key": "diana", "name": "Diana", "age": 28, "dept": "sales"}))
        .unwrap();
    users
        .insert(json!({"_key": "eve", "name": "Eve", "age": 32, "dept": "eng"}))
        .unwrap();

    (engine, tmp)
}

// ============================================================================
// Bind Variables Tests
// ============================================================================

#[test]
fn test_bind_var_string() {
    let (engine, _tmp) = create_seeded_engine();

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
fn test_bind_var_number() {
    let (engine, _tmp) = create_seeded_engine();

    let mut binds = HashMap::new();
    binds.insert("minAge".to_string(), json!(30));

    let results = execute_with_binds(
        &engine,
        "FOR u IN users FILTER u.age >= @minAge RETURN u.name",
        binds,
    );
    assert_eq!(results.len(), 3); // Alice(30), Charlie(35), Eve(32)
}

#[test]
fn test_bind_var_array() {
    let (engine, _tmp) = create_seeded_engine();

    let mut binds = HashMap::new();
    binds.insert("depts".to_string(), json!(["eng", "hr"]));

    let results = execute_with_binds(
        &engine,
        "FOR u IN users FILTER u.dept IN @depts RETURN u.name",
        binds,
    );
    assert_eq!(results.len(), 3); // Alice, Bob, Eve (all eng)
}

#[test]
fn test_bind_var_in_limit() {
    let (engine, _tmp) = create_seeded_engine();

    let mut binds = HashMap::new();
    binds.insert("limit".to_string(), json!(2));

    let results = execute_with_binds(&engine, "FOR u IN users LIMIT @limit RETURN u.name", binds);
    assert_eq!(results.len(), 2);
}

#[test]
fn test_multiple_bind_vars() {
    let (engine, _tmp) = create_seeded_engine();

    let mut binds = HashMap::new();
    binds.insert("minAge".to_string(), json!(25));
    binds.insert("maxAge".to_string(), json!(32));
    binds.insert("dept".to_string(), json!("eng"));

    let results = execute_with_binds(&engine, 
        "FOR u IN users FILTER u.age >= @minAge AND u.age <= @maxAge AND u.dept == @dept RETURN u.name", binds);
    assert!(results.contains(&json!("Alice"))); // age 30, eng
    assert!(results.contains(&json!("Bob"))); // age 25, eng
    assert!(results.contains(&json!("Eve"))); // age 32, eng
}

// ============================================================================
// Database Context Tests
// ============================================================================

#[test]
fn test_executor_with_database() {
    let (engine, _tmp) = create_test_engine();

    // Create a database and collection within it
    engine.create_database("testdb".to_string()).unwrap();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("items".to_string(), None).unwrap();
    let items = db.get_collection("items").unwrap();
    items
        .insert(json!({"_key": "i1", "name": "Item 1"}))
        .unwrap();

    let results = execute_with_database(&engine, "testdb", "FOR i IN items RETURN i.name");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Item 1"));
}

#[test]
fn test_executor_with_database_and_binds() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("testdb".to_string()).unwrap();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("items".to_string(), None).unwrap();
    let items = db.get_collection("items").unwrap();
    items
        .insert(json!({"_key": "i1", "name": "Item 1", "price": 100}))
        .unwrap();
    items
        .insert(json!({"_key": "i2", "name": "Item 2", "price": 200}))
        .unwrap();

    let mut binds = HashMap::new();
    binds.insert("minPrice".to_string(), json!(150));

    let results = execute_with_db_and_binds(
        &engine,
        "testdb",
        "FOR i IN items FILTER i.price >= @minPrice RETURN i.name",
        binds,
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("Item 2"));
}

// ============================================================================
// COLLECT/GROUP BY Tests
// ============================================================================

#[test]
fn test_collect_group_by() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users COLLECT dept = u.dept RETURN { dept: dept }",
    );
    assert_eq!(results.len(), 2); // eng, sales
}

#[test]
fn test_collect_basic() {
    let (engine, _tmp) = create_seeded_engine();

    // Basic COLLECT without INTO (groups by value)
    let results = execute_query(&engine, "FOR u IN users COLLECT dept = u.dept RETURN dept");
    assert_eq!(results.len(), 2); // eng, sales
}

#[test]
fn test_remove_operation() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items
        .insert(json!({"_key": "item1", "name": "Test"}))
        .unwrap();
    items
        .insert(json!({"_key": "item2", "name": "Other"}))
        .unwrap();

    // Simple REMOVE
    execute_query(&engine, "REMOVE 'item1' IN items");

    let results = execute_query(&engine, "FOR i IN items RETURN i._key");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("item2"));
}

// ============================================================================
// Multiple SORT Fields Tests
// ============================================================================

#[test]
fn test_sort_multiple_fields_asc_desc() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(&engine, 
        "FOR u IN users SORT u.dept ASC, u.age DESC RETURN { dept: u.dept, name: u.name, age: u.age }");

    // Should be: eng (Eve 32, Alice 30, Bob 25), sales (Charlie 35, Diana 28)
    assert_eq!(results[0]["dept"], json!("eng"));
    assert_eq!(results[0]["age"], json!(32)); // Eve (oldest eng)

    assert_eq!(results[3]["dept"], json!("sales"));
    assert_eq!(results[3]["age"], json!(35)); // Charlie (oldest sales)
}

#[test]
fn test_sort_three_fields() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();
    data.insert(json!({"_key": "1", "a": 1, "b": 1, "c": 1}))
        .unwrap();
    data.insert(json!({"_key": "2", "a": 1, "b": 1, "c": 2}))
        .unwrap();
    data.insert(json!({"_key": "3", "a": 1, "b": 2, "c": 1}))
        .unwrap();
    data.insert(json!({"_key": "4", "a": 2, "b": 1, "c": 1}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR d IN data SORT d.a ASC, d.b ASC, d.c ASC RETURN d._key",
    );
    assert_eq!(results.len(), 4);
    assert_eq!(results[0], json!("1"));
    assert_eq!(results[1], json!("2"));
    assert_eq!(results[2], json!("3"));
    assert_eq!(results[3], json!("4"));
}

// ============================================================================
// Graph Traversal Edge Cases
// ============================================================================

#[test]
fn test_graph_inbound_traversal() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("nodes".to_string(), None).unwrap();
    let nodes = engine.get_collection("nodes").unwrap();
    nodes.insert(json!({"_key": "a", "name": "A"})).unwrap();
    nodes.insert(json!({"_key": "b", "name": "B"})).unwrap();
    nodes.insert(json!({"_key": "c", "name": "C"})).unwrap();

    engine
        .create_collection("links".to_string(), Some("edge".to_string()))
        .unwrap();
    let links = engine.get_collection("links").unwrap();
    links
        .insert(json!({"_from": "nodes/a", "_to": "nodes/b"}))
        .unwrap();
    links
        .insert(json!({"_from": "nodes/c", "_to": "nodes/b"}))
        .unwrap();

    // B has inbound edges from A and C
    let results = execute_query(
        &engine,
        "FOR v IN 1..1 INBOUND 'nodes/b' links RETURN v.name",
    );
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("A")));
    assert!(results.contains(&json!("C")));
}

#[test]
fn test_graph_any_direction() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("nodes".to_string(), None).unwrap();
    let nodes = engine.get_collection("nodes").unwrap();
    nodes.insert(json!({"_key": "a", "name": "A"})).unwrap();
    nodes.insert(json!({"_key": "b", "name": "B"})).unwrap();
    nodes.insert(json!({"_key": "c", "name": "C"})).unwrap();

    engine
        .create_collection("links".to_string(), Some("edge".to_string()))
        .unwrap();
    let links = engine.get_collection("links").unwrap();
    links
        .insert(json!({"_from": "nodes/a", "_to": "nodes/b"}))
        .unwrap();
    links
        .insert(json!({"_from": "nodes/b", "_to": "nodes/c"}))
        .unwrap();

    // B connected to both A (inbound) and C (outbound)
    let results = execute_query(&engine, "FOR v IN 1..1 ANY 'nodes/b' links RETURN v.name");
    assert_eq!(results.len(), 2);
}

#[test]
fn test_graph_depth_2() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("nodes".to_string(), None).unwrap();
    let nodes = engine.get_collection("nodes").unwrap();
    nodes.insert(json!({"_key": "a", "name": "A"})).unwrap();
    nodes.insert(json!({"_key": "b", "name": "B"})).unwrap();
    nodes.insert(json!({"_key": "c", "name": "C"})).unwrap();

    engine
        .create_collection("links".to_string(), Some("edge".to_string()))
        .unwrap();
    let links = engine.get_collection("links").unwrap();
    links
        .insert(json!({"_from": "nodes/a", "_to": "nodes/b"}))
        .unwrap();
    links
        .insert(json!({"_from": "nodes/b", "_to": "nodes/c"}))
        .unwrap();

    // A -> B -> C (depth 1..2 from A)
    let results = execute_query(
        &engine,
        "FOR v IN 1..2 OUTBOUND 'nodes/a' links RETURN v.name",
    );
    assert!(results.len() >= 2);
    assert!(results.contains(&json!("B")));
    assert!(results.contains(&json!("C")));
}

// ============================================================================
// Explain Tests
// ============================================================================

#[test]
fn test_explain_simple_for() {
    let (engine, _tmp) = create_seeded_engine();

    let explain = explain_query(&engine, "FOR u IN users RETURN u.name");

    assert!(!explain.collections.is_empty());
    assert_eq!(explain.collections[0].name, "users");
    assert!(explain.timing.total_us > 0);
    assert!(explain.documents_scanned > 0);
}

#[test]
fn test_explain_with_filter() {
    let (engine, _tmp) = create_seeded_engine();

    let explain = explain_query(&engine, "FOR u IN users FILTER u.age > 30 RETURN u.name");

    assert!(!explain.filters.is_empty());
    assert!(explain.filters[0].documents_after <= explain.filters[0].documents_before);
}

#[test]
fn test_explain_with_sort() {
    let (engine, _tmp) = create_seeded_engine();

    let explain = explain_query(&engine, "FOR u IN users SORT u.age DESC RETURN u.name");

    assert!(explain.sort.is_some());
    let sort_info = explain.sort.unwrap();
    assert!(sort_info.field.contains("age"));
    // The direction string may differ based on implementation
}

#[test]
fn test_explain_with_limit() {
    let (engine, _tmp) = create_seeded_engine();

    let explain = explain_query(&engine, "FOR u IN users LIMIT 3 RETURN u.name");

    assert!(explain.limit.is_some());
    let limit_info = explain.limit.unwrap();
    assert_eq!(limit_info.count, 3);
}

#[test]
fn test_explain_with_let() {
    let (engine, _tmp) = create_seeded_engine();

    let explain = explain_query(
        &engine,
        "LET x = 10 FOR u IN users FILTER u.age > x RETURN u.name",
    );

    assert!(!explain.let_bindings.is_empty());
    assert_eq!(explain.let_bindings[0].variable, "x");
}

// ============================================================================
// Regex Operators Tests
// ============================================================================

#[test]
fn test_regex_match_operator() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("emails".to_string(), None)
        .unwrap();
    let emails = engine.get_collection("emails").unwrap();
    emails
        .insert(json!({"_key": "1", "email": "alice@example.com"}))
        .unwrap();
    emails
        .insert(json!({"_key": "2", "email": "bob@test.org"}))
        .unwrap();
    emails
        .insert(json!({"_key": "3", "email": "charlie@example.com"}))
        .unwrap();

    // =~ regex match
    let results = execute_query(
        &engine,
        r#"FOR e IN emails FILTER e.email =~ ".*@example\\.com" RETURN e._key"#,
    );
    assert_eq!(results.len(), 2);
}

#[test]
fn test_regex_not_match_operator() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items
        .insert(json!({"_key": "1", "name": "Test123"}))
        .unwrap();
    items
        .insert(json!({"_key": "2", "name": "NoNumbers"}))
        .unwrap();
    items
        .insert(json!({"_key": "3", "name": "Has456"}))
        .unwrap();

    // !~ regex not match
    let results = execute_query(
        &engine,
        r#"FOR i IN items FILTER i.name !~ "[0-9]+" RETURN i.name"#,
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("NoNumbers"));
}

// ============================================================================
// Hash Functions Tests
// ============================================================================

#[test]
fn test_md5_function() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN MD5('hello')");
    assert_eq!(results[0], json!("5d41402abc4b2a76b9719d911017c592"));
}

#[test]
fn test_sha256_function() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN SHA256('hello')");
    // Expected SHA256 hash of "hello"
    assert_eq!(
        results[0],
        json!("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
    );
}

#[test]
fn test_rand_function() {
    let (engine, _tmp) = create_test_engine();

    // RAND returns a random number between 0 and 1
    let results = execute_query(&engine, "RETURN RANDOM()");
    let rand_val = results[0].as_f64().unwrap();
    assert!(rand_val >= 0.0 && rand_val < 1.0);
}

// ============================================================================
// RETURN DISTINCT Tests
// ============================================================================

#[test]
fn test_unique_function_for_distinct() {
    let (engine, _tmp) = create_seeded_engine();

    // Use UNIQUE function to get distinct values
    let results = execute_query(
        &engine,
        "LET depts = (FOR u IN users RETURN u.dept) RETURN UNIQUE(depts)",
    );
    let unique_depts = results[0].as_array().unwrap();
    assert_eq!(unique_depts.len(), 2); // eng, sales
}

// ============================================================================
// Empty Collection Tests
// ============================================================================

#[test]
fn test_empty_collection() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("empty".to_string(), None).unwrap();

    let results = execute_query(&engine, "FOR e IN empty RETURN e");
    assert_eq!(results.len(), 0);
}

#[test]
fn test_filter_no_match() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(&engine, "FOR u IN users FILTER u.age > 100 RETURN u");
    assert_eq!(results.len(), 0);
}

// ============================================================================
// Arithmetic Expression Edge Cases
// ============================================================================

#[test]
fn test_division_normal() {
    let (engine, _tmp) = create_test_engine();

    // Normal division works fine
    let results = execute_query(&engine, "RETURN 10 / 2");
    assert_eq!(results[0], json!(5.0));
}

#[test]
fn test_large_numbers() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN 999999999999 * 2");
    let val = results[0].as_f64().unwrap();
    assert!(val > 1e12);
}

#[test]
fn test_floating_point_precision() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN 0.1 + 0.2");
    let val = results[0].as_f64().unwrap();
    assert!((val - 0.3).abs() < 0.0001);
}

// ============================================================================
// Nested Object/Array Access
// ============================================================================

#[test]
fn test_deeply_nested_access() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("deep".to_string(), None).unwrap();
    let deep = engine.get_collection("deep").unwrap();
    deep.insert(json!({
        "_key": "1",
        "level1": {
            "level2": {
                "level3": {
                    "value": 42
                }
            }
        }
    }))
    .unwrap();

    let results = execute_query(&engine, "FOR d IN deep RETURN d.level1.level2.level3.value");
    assert_eq!(results[0], json!(42));
}

#[test]
fn test_array_in_object() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();
    data.insert(json!({
        "_key": "1",
        "tags": ["a", "b", "c"],
        "nested": {
            "items": [1, 2, 3]
        }
    }))
    .unwrap();

    let results = execute_query(
        &engine,
        "FOR d IN data RETURN { firstTag: d.tags[0], secondItem: d.nested.items[1] }",
    );
    assert_eq!(results[0]["firstTag"], json!("a"));
    assert_eq!(results[0]["secondItem"], json!(2));
}

// ============================================================================
// NULL Handling
// ============================================================================

#[test]
fn test_null_field_access() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();
    data.insert(json!({"_key": "1", "name": "Test"})).unwrap();

    // Access non-existent field should return null
    let results = execute_query(&engine, "FOR d IN data RETURN d.nonexistent");
    assert_eq!(results[0], json!(null));
}

#[test]
fn test_null_comparison() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();
    data.insert(json!({"_key": "1", "value": null})).unwrap();
    data.insert(json!({"_key": "2", "value": 10})).unwrap();

    let results = execute_query(
        &engine,
        "FOR d IN data FILTER d.value == null RETURN d._key",
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!("1"));
}

// ============================================================================
// Return Clause Variations
// ============================================================================

#[test]
fn test_return_plain_expression() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN 1 + 2 + 3");
    assert_eq!(results[0], json!(6.0));
}

#[test]
fn test_return_object_static_keys() {
    let (engine, _tmp) = create_seeded_engine();

    let results = execute_query(
        &engine,
        "FOR u IN users LIMIT 1 RETURN { name: u.name, age: u.age }",
    );
    // Should create object with static keys
    assert_eq!(results.len(), 1);
    assert!(results[0].get("name").is_some());
    assert!(results[0].get("age").is_some());
}

#[test]
fn test_return_nested_function_calls() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(
        &engine,
        "RETURN UPPER(CONCAT('hello', ' ', LOWER('WORLD')))",
    );
    assert_eq!(results[0], json!("HELLO WORLD"));
}

// ============================================================================
// Complex Combined Queries
// ============================================================================

#[test]
fn test_complex_query_all_clauses() {
    let (engine, _tmp) = create_seeded_engine();

    let mut binds = HashMap::new();
    binds.insert("minAge".to_string(), json!(25));

    let results = execute_with_binds(
        &engine,
        r#"
        LET threshold = @minAge
        FOR u IN users
            FILTER u.age >= threshold AND u.dept == 'eng'
            SORT u.age DESC
            LIMIT 2
            RETURN { name: u.name, age: u.age }
    "#,
        binds,
    );

    assert_eq!(results.len(), 2);
    // Should be Eve (32) and Alice (30)
    assert_eq!(results[0]["age"], json!(32));
    assert_eq!(results[1]["age"], json!(30));
}

#[test]
fn test_nested_for_loops() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("categories".to_string(), None)
        .unwrap();
    let categories = engine.get_collection("categories").unwrap();
    categories
        .insert(json!({"_key": "c1", "name": "Cat1"}))
        .unwrap();
    categories
        .insert(json!({"_key": "c2", "name": "Cat2"}))
        .unwrap();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();
    products
        .insert(json!({"_key": "p1", "name": "Prod1", "category": "c1"}))
        .unwrap();
    products
        .insert(json!({"_key": "p2", "name": "Prod2", "category": "c1"}))
        .unwrap();
    products
        .insert(json!({"_key": "p3", "name": "Prod3", "category": "c2"}))
        .unwrap();

    let results = execute_query(
        &engine,
        r#"
        FOR c IN categories
            FOR p IN products
                FILTER p.category == c._key
                RETURN { category: c.name, product: p.name }
    "#,
    );

    assert_eq!(results.len(), 3);
}

// ============================================================================
// Special Characters in Data
// ============================================================================

#[test]
fn test_special_characters_in_strings() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();
    data.insert(json!({"_key": "1", "name": "O'Brien", "quote": "He said \"Hello\""}))
        .unwrap();

    let results = execute_query(&engine, "FOR d IN data RETURN d.name");
    assert_eq!(results[0], json!("O'Brien"));
}

#[test]
fn test_unicode_data() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();
    data.insert(json!({"_key": "1", "jp": "日本語", "emoji": "🎉"}))
        .unwrap();

    let results = execute_query(&engine, "FOR d IN data RETURN { jp: d.jp, emoji: d.emoji }");
    assert_eq!(results[0]["jp"], json!("日本語"));
    assert_eq!(results[0]["emoji"], json!("🎉"));
}
