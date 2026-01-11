//! Aggregation and Advanced Query Tests
//!
//! Comprehensive tests for:
//! - COLLECT clause and grouping
//! - Aggregation functions (COUNT, SUM, AVG, etc.)
//! - LET variables and subqueries
//! - Multiple collection queries
//! - Complex filter conditions
//! - SORT and LIMIT combinations

use serde_json::json;
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .execute(&query)
        .expect(&format!("Query failed: {}", query_str))
}

fn execute_single(engine: &StorageEngine, query_str: &str) -> serde_json::Value {
    let results = execute_query(engine, query_str);
    results
        .into_iter()
        .next()
        .unwrap_or(serde_json::Value::Null)
}

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

fn create_sales_data() -> (StorageEngine, TempDir) {
    let (engine, tmp_dir) = create_test_engine();

    // Create sales collection
    engine.create_collection("sales".to_string(), None).unwrap();
    let sales = engine.get_collection("sales").unwrap();

    sales.insert(json!({"_key": "s1", "product": "Widget", "category": "A", "amount": 100, "quantity": 5})).unwrap();
    sales.insert(json!({"_key": "s2", "product": "Gadget", "category": "A", "amount": 200, "quantity": 3})).unwrap();
    sales.insert(json!({"_key": "s3", "product": "Widget", "category": "A", "amount": 150, "quantity": 7})).unwrap();
    sales.insert(json!({"_key": "s4", "product": "Gizmo", "category": "B", "amount": 75, "quantity": 10})).unwrap();
    sales.insert(json!({"_key": "s5", "product": "Gadget", "category": "B", "amount": 250, "quantity": 2})).unwrap();
    sales.insert(json!({"_key": "s6", "product": "Widget", "category": "B", "amount": 50, "quantity": 20})).unwrap();

    (engine, tmp_dir)
}

fn create_users_orders() -> (StorageEngine, TempDir) {
    let (engine, tmp_dir) = create_test_engine();

    // Users collection
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    users
        .insert(json!({"_key": "u1", "name": "Alice", "country": "USA"}))
        .unwrap();
    users
        .insert(json!({"_key": "u2", "name": "Bob", "country": "UK"}))
        .unwrap();
    users
        .insert(json!({"_key": "u3", "name": "Charlie", "country": "USA"}))
        .unwrap();

    // Orders collection
    engine
        .create_collection("orders".to_string(), None)
        .unwrap();
    let orders = engine.get_collection("orders").unwrap();
    orders
        .insert(json!({"_key": "o1", "user_key": "u1", "total": 100}))
        .unwrap();
    orders
        .insert(json!({"_key": "o2", "user_key": "u1", "total": 200}))
        .unwrap();
    orders
        .insert(json!({"_key": "o3", "user_key": "u2", "total": 150}))
        .unwrap();
    orders
        .insert(json!({"_key": "o4", "user_key": "u3", "total": 300}))
        .unwrap();

    (engine, tmp_dir)
}

// ============================================================================
// LET Variable Tests
// ============================================================================

#[test]
fn test_let_simple_value() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "LET x = 42 RETURN x");
    assert_eq!(result, json!(42));
}

#[test]
fn test_let_string_value() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "LET name = 'Alice' RETURN name");
    assert_eq!(result, json!("Alice"));
}

#[test]
fn test_let_expression() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "LET x = 10 + 20 RETURN x");
    assert_eq!(result, json!(30.0));
}

#[test]
fn test_let_multiple_variables() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "LET a = 10 LET b = 20 RETURN a + b");
    assert_eq!(result, json!(30.0));
}

#[test]
fn test_let_reference_previous() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "LET a = 5 LET b = a * 2 RETURN b");
    assert_eq!(result, json!(10.0));
}

#[test]
fn test_let_array() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "LET arr = [1, 2, 3] RETURN LENGTH(arr)");
    assert_eq!(result, json!(3));
}

#[test]
fn test_let_with_for() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(
        &engine,
        "LET threshold = 100 FOR s IN sales FILTER s.amount >= threshold RETURN s.product",
    );

    // s1: 100, s2: 200, s3: 150, s5: 250 all >= 100
    assert!(results.len() >= 4);
}

// ============================================================================
// Subquery Tests
// ============================================================================

#[test]
fn test_subquery_in_let() {
    let (engine, _tmp) = create_sales_data();

    let result = execute_single(
        &engine,
        "LET total = (FOR s IN sales RETURN s.amount) RETURN SUM(total)",
    );

    // 100 + 200 + 150 + 75 + 250 + 50 = 825
    assert_eq!(result, json!(825.0));
}

#[test]
fn test_subquery_count() {
    let (engine, _tmp) = create_sales_data();

    let result = execute_single(
        &engine,
        "LET items = (FOR s IN sales RETURN s) RETURN LENGTH(items)",
    );

    assert_eq!(result, json!(6));
}

#[test]
fn test_subquery_with_filter() {
    let (engine, _tmp) = create_sales_data();

    let result = execute_single(&engine, 
        "LET widgets = (FOR s IN sales FILTER s.product == 'Widget' RETURN s.amount) RETURN SUM(widgets)");

    // Widget amounts: 100 + 150 + 50 = 300
    assert_eq!(result, json!(300.0));
}

// ============================================================================
// Aggregation Function Tests in Queries
// ============================================================================

#[test]
fn test_sum_with_subquery() {
    let (engine, _tmp) = create_sales_data();

    let result = execute_single(
        &engine,
        "LET amounts = (FOR s IN sales RETURN s.amount) RETURN SUM(amounts)",
    );
    assert_eq!(result, json!(825.0));
}

#[test]
fn test_avg_with_subquery() {
    let (engine, _tmp) = create_sales_data();

    let result = execute_single(
        &engine,
        "LET amounts = (FOR s IN sales RETURN s.amount) RETURN AVG(amounts)",
    );

    // 825 / 6 = 137.5
    assert_eq!(result, json!(137.5));
}

#[test]
fn test_min_with_subquery() {
    let (engine, _tmp) = create_sales_data();

    let result = execute_single(
        &engine,
        "LET amounts = (FOR s IN sales RETURN s.amount) RETURN MIN(amounts)",
    );
    // MIN returns float
    assert_eq!(result, json!(50.0));
}

#[test]
fn test_max_with_subquery() {
    let (engine, _tmp) = create_sales_data();

    let result = execute_single(
        &engine,
        "LET amounts = (FOR s IN sales RETURN s.amount) RETURN MAX(amounts)",
    );
    // MAX returns float
    assert_eq!(result, json!(250.0));
}

// ============================================================================
// SORT Tests
// ============================================================================

#[test]
fn test_sort_ascending() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(&engine, "FOR s IN sales SORT s.amount ASC RETURN s.amount");

    assert_eq!(results.len(), 6);
    assert_eq!(results[0], json!(50));
    assert_eq!(results[5], json!(250));
}

#[test]
fn test_sort_descending() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(&engine, "FOR s IN sales SORT s.amount DESC RETURN s.amount");

    assert_eq!(results.len(), 6);
    assert_eq!(results[0], json!(250));
    assert_eq!(results[5], json!(50));
}

#[test]
fn test_sort_by_string() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(
        &engine,
        "FOR s IN sales SORT s.product ASC RETURN s.product",
    );

    // Should be sorted alphabetically: Gadget, Gadget, Gizmo, Widget, Widget, Widget
    assert_eq!(results[0], json!("Gadget"));
}

#[test]
fn test_sort_with_limit() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(
        &engine,
        "FOR s IN sales SORT s.amount DESC LIMIT 3 RETURN s.amount",
    );

    assert_eq!(results.len(), 3);
    assert_eq!(results[0], json!(250)); // Highest
}

#[test]
fn test_sort_with_offset_and_limit() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(
        &engine,
        "FOR s IN sales SORT s.amount DESC LIMIT 2, 2 RETURN s.amount",
    );

    // Skip 2, take 2
    assert_eq!(results.len(), 2);
}

// ============================================================================
// LIMIT Tests
// ============================================================================

#[test]
fn test_limit_simple() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(&engine, "FOR s IN sales LIMIT 3 RETURN s");
    assert_eq!(results.len(), 3);
}

#[test]
fn test_limit_with_offset() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(&engine, "FOR s IN sales LIMIT 1, 3 RETURN s._key");
    assert_eq!(results.len(), 3);
}

#[test]
fn test_limit_zero() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(&engine, "FOR s IN sales LIMIT 0 RETURN s");
    assert!(results.is_empty());
}

#[test]
fn test_limit_larger_than_collection() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(&engine, "FOR s IN sales LIMIT 100 RETURN s");
    assert_eq!(results.len(), 6); // Only 6 documents exist
}

// ============================================================================
// Complex Filter Tests
// ============================================================================

#[test]
fn test_filter_and() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(
        &engine,
        "FOR s IN sales FILTER s.category == 'A' AND s.amount > 100 RETURN s.product",
    );

    // Category A with amount > 100: s2 (200), s3 (150)
    assert_eq!(results.len(), 2);
}

#[test]
fn test_filter_or() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(
        &engine,
        "FOR s IN sales FILTER s.product == 'Widget' OR s.product == 'Gizmo' RETURN s._key",
    );

    // Widgets: s1, s3, s6; Gizmo: s4
    assert_eq!(results.len(), 4);
}

#[test]
fn test_filter_multiple_conditions() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(&engine, 
        "FOR s IN sales FILTER s.category == 'A' AND s.amount >= 100 AND s.amount <= 200 RETURN s._key");

    // Category A, amount 100-200: s1 (100), s2 (200), s3 (150)
    assert_eq!(results.len(), 3);
}

#[test]
fn test_filter_with_parentheses() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(&engine, 
        "FOR s IN sales FILTER (s.category == 'A' OR s.category == 'B') AND s.amount > 100 RETURN s");

    // Both categories, amount > 100
    assert!(results.len() >= 3);
}

#[test]
fn test_filter_in_array() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(
        &engine,
        "FOR s IN sales FILTER s.product IN ['Widget', 'Gadget'] RETURN s._key",
    );

    // Widget: s1, s3, s6; Gadget: s2, s5
    assert_eq!(results.len(), 5);
}

#[test]
fn test_filter_like() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(
        &engine,
        "FOR s IN sales FILTER s.product LIKE 'Wid%' RETURN s._key",
    );

    // Matches Widget: s1, s3, s6
    assert_eq!(results.len(), 3);
}

// ============================================================================
// Multi-Collection Queries
// ============================================================================

#[test]
fn test_nested_for_loops() {
    let (engine, _tmp) = create_users_orders();

    // Find user names with their order totals
    let results = execute_query(
        &engine,
        r#"
        FOR u IN users
            FOR o IN orders
                FILTER o.user_key == u._key
                RETURN o.total
    "#,
    );

    // Should return all 4 orders
    assert_eq!(results.len(), 4);
}

#[test]
fn test_nested_for_with_filter() {
    let (engine, _tmp) = create_users_orders();

    // Find orders from USA users
    let results = execute_query(
        &engine,
        r#"
        FOR u IN users
            FILTER u.country == 'USA'
            FOR o IN orders
                FILTER o.user_key == u._key
                RETURN o._key
    "#,
    );

    // Alice (u1) has 2 orders, Charlie (u3) has 1 order
    assert_eq!(results.len(), 3);
}

// ============================================================================
// Return Expression Tests
// ============================================================================

#[test]
fn test_return_calculated_field() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(&engine, "FOR s IN sales RETURN s.amount * s.quantity");

    assert_eq!(results.len(), 6);
    // First result: s1 = 100 * 5 = 500
}

#[test]
fn test_return_multiple_fields() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(
        &engine,
        "FOR s IN sales LIMIT 1 RETURN [s.product, s.amount]",
    );

    assert!(results[0].is_array());
}

// ============================================================================
// Empty Result Tests
// ============================================================================

#[test]
fn test_filter_no_matches() {
    let (engine, _tmp) = create_sales_data();

    let results = execute_query(
        &engine,
        "FOR s IN sales FILTER s.product == 'NonExistent' RETURN s",
    );

    assert!(results.is_empty());
}

#[test]
fn test_empty_collection_query() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("empty".to_string(), None).unwrap();

    let results = execute_query(&engine, "FOR e IN empty RETURN e");
    assert!(results.is_empty());
}

// ============================================================================
// Special Values Tests
// ============================================================================

#[test]
fn test_filter_null_field() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("data".to_string(), None).unwrap();
    let col = engine.get_collection("data").unwrap();
    col.insert(json!({"_key": "1", "value": null})).unwrap();
    col.insert(json!({"_key": "2", "value": 10})).unwrap();
    col.insert(json!({"_key": "3", "value": null})).unwrap();

    let results = execute_query(
        &engine,
        "FOR d IN data FILTER d.value == null RETURN d._key",
    );

    assert_eq!(results.len(), 2);
}

#[test]
fn test_return_null() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN null");
    assert_eq!(result, json!(null));
}

#[test]
fn test_filter_boolean() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("flags".to_string(), None).unwrap();
    let col = engine.get_collection("flags").unwrap();
    col.insert(json!({"_key": "1", "active": true})).unwrap();
    col.insert(json!({"_key": "2", "active": false})).unwrap();
    col.insert(json!({"_key": "3", "active": true})).unwrap();

    let results = execute_query(
        &engine,
        "FOR f IN flags FILTER f.active == true RETURN f._key",
    );

    assert_eq!(results.len(), 2);
}

// ============================================================================
// Range Iteration Tests
// ============================================================================

#[test]
fn test_for_in_range() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "FOR i IN 1..5 RETURN i");
    assert_eq!(results.len(), 5);
    assert_eq!(
        results,
        vec![json!(1), json!(2), json!(3), json!(4), json!(5)]
    );
}

#[test]
fn test_range_with_filter() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "FOR i IN 1..10 FILTER i > 5 RETURN i");
    assert_eq!(results.len(), 5);
}

#[test]
fn test_range_with_calculation() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(
        &engine,
        "LET nums = (FOR i IN 1..5 RETURN i) RETURN SUM(nums)",
    );

    // 1+2+3+4+5 = 15
    assert_eq!(result, json!(15.0));
}
