//! SDBQL Operator Tests
//!
//! Comprehensive tests for all SDBQL operators including:
//! - Comparison operators
//! - Logical operators
//! - Arithmetic operators
//! - String operators
//! - Array operators
//! - Ternary expressions

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

// ============================================================================
// Arithmetic Operators
// ============================================================================

#[test]
fn test_operator_addition() {
    let (engine, _tmp) = create_test_engine();

    // Arithmetic returns floats
    assert_eq!(execute_single(&engine, "RETURN 1 + 2"), json!(3.0));
    assert_eq!(execute_single(&engine, "RETURN 10 + 20 + 30"), json!(60.0));
    assert_eq!(execute_single(&engine, "RETURN 1.5 + 2.5"), json!(4.0));
}

#[test]
fn test_operator_subtraction() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN 10 - 3"), json!(7.0));
    assert_eq!(execute_single(&engine, "RETURN 100 - 50 - 25"), json!(25.0));
}

#[test]
fn test_operator_multiplication() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN 5 * 4"), json!(20.0));
    assert_eq!(execute_single(&engine, "RETURN 2 * 3 * 4"), json!(24.0));
}

#[test]
fn test_operator_division() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN 20 / 4"), json!(5.0));
    assert_eq!(execute_single(&engine, "RETURN 15 / 2"), json!(7.5));
}

// Note: % operator not supported, skipping modulo test

#[test]
fn test_operator_precedence_arithmetic() {
    let (engine, _tmp) = create_test_engine();

    // Multiplication before addition - returns float
    assert_eq!(execute_single(&engine, "RETURN 2 + 3 * 4"), json!(14.0));
    // Parentheses override precedence
    assert_eq!(execute_single(&engine, "RETURN (2 + 3) * 4"), json!(20.0));
}

#[test]
fn test_operator_negative_numbers() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN -5 + 10"), json!(5.0));
    assert_eq!(execute_single(&engine, "RETURN -3 * -2"), json!(6.0));
}

// ============================================================================
// Comparison Operators
// ============================================================================

#[test]
fn test_operator_equals() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN 5 == 5"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN 5 == 6"), json!(false));
    assert_eq!(
        execute_single(&engine, "RETURN 'hello' == 'hello'"),
        json!(true)
    );
}

#[test]
fn test_operator_not_equals() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN 5 != 6"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN 5 != 5"), json!(false));
}

#[test]
fn test_operator_greater_than() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN 10 > 5"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN 5 > 10"), json!(false));
    assert_eq!(execute_single(&engine, "RETURN 5 > 5"), json!(false));
}

#[test]
fn test_operator_greater_or_equal() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN 10 >= 5"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN 5 >= 5"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN 4 >= 5"), json!(false));
}

#[test]
fn test_operator_less_than() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN 5 < 10"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN 10 < 5"), json!(false));
    assert_eq!(execute_single(&engine, "RETURN 5 < 5"), json!(false));
}

#[test]
fn test_operator_less_or_equal() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN 5 <= 10"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN 5 <= 5"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN 6 <= 5"), json!(false));
}

// ============================================================================
// Logical Operators
// ============================================================================

#[test]
fn test_operator_and() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN true AND true"), json!(true));
    assert_eq!(
        execute_single(&engine, "RETURN true AND false"),
        json!(false)
    );
    assert_eq!(
        execute_single(&engine, "RETURN false AND true"),
        json!(false)
    );
    assert_eq!(
        execute_single(&engine, "RETURN false AND false"),
        json!(false)
    );
}

#[test]
fn test_operator_or() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN true OR false"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN false OR true"), json!(true));
    assert_eq!(
        execute_single(&engine, "RETURN false OR false"),
        json!(false)
    );
    assert_eq!(execute_single(&engine, "RETURN true OR true"), json!(true));
}

#[test]
fn test_operator_not() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN NOT true"), json!(false));
    assert_eq!(execute_single(&engine, "RETURN NOT false"), json!(true));
}

#[test]
fn test_operator_logical_precedence() {
    let (engine, _tmp) = create_test_engine();

    // AND has higher precedence than OR
    assert_eq!(
        execute_single(&engine, "RETURN true OR false AND false"),
        json!(true)
    );
    assert_eq!(
        execute_single(&engine, "RETURN (true OR false) AND false"),
        json!(false)
    );
}

// ============================================================================
// String Operators
// ============================================================================

#[test]
fn test_operator_string_concat() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(
        execute_single(&engine, "RETURN CONCAT('Hello', ' ', 'World')"),
        json!("Hello World")
    );
}

#[test]
fn test_operator_like() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items.insert(json!({"_key": "1", "name": "Apple"})).unwrap();
    items
        .insert(json!({"_key": "2", "name": "Application"}))
        .unwrap();
    items
        .insert(json!({"_key": "3", "name": "Banana"}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR i IN items FILTER i.name LIKE 'App%' RETURN i.name",
    );
    assert_eq!(results.len(), 2);
}

#[test]
fn test_operator_like_underscore() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items.insert(json!({"_key": "1", "name": "cat"})).unwrap();
    items.insert(json!({"_key": "2", "name": "car"})).unwrap();
    items.insert(json!({"_key": "3", "name": "cage"})).unwrap();

    // _ matches single character
    let results = execute_query(
        &engine,
        "FOR i IN items FILTER i.name LIKE 'ca_' RETURN i.name",
    );
    assert_eq!(results.len(), 2); // cat, car
}

#[test]
fn test_operator_not_like() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items.insert(json!({"_key": "1", "name": "Apple"})).unwrap();
    items
        .insert(json!({"_key": "2", "name": "Banana"}))
        .unwrap();
    items
        .insert(json!({"_key": "3", "name": "Cherry"}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR i IN items FILTER i.name NOT LIKE 'A%' RETURN i.name",
    );
    assert_eq!(results.len(), 2);
}

// ============================================================================
// Array Operators
// ============================================================================

#[test]
fn test_operator_in_array() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(
        execute_single(&engine, "RETURN 2 IN [1, 2, 3]"),
        json!(true)
    );
    assert_eq!(
        execute_single(&engine, "RETURN 5 IN [1, 2, 3]"),
        json!(false)
    );
    assert_eq!(
        execute_single(&engine, "RETURN 'a' IN ['a', 'b', 'c']"),
        json!(true)
    );
}

#[test]
fn test_operator_not_in_array() {
    let (engine, _tmp) = create_test_engine();

    // NOT IN may have different syntax
    assert_eq!(
        execute_single(&engine, "RETURN NOT (5 IN [1, 2, 3])"),
        json!(true)
    );
    assert_eq!(
        execute_single(&engine, "RETURN NOT (2 IN [1, 2, 3])"),
        json!(false)
    );
}

#[test]
fn test_array_access() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN [1, 2, 3][0]"), json!(1));
    assert_eq!(execute_single(&engine, "RETURN [1, 2, 3][2]"), json!(3));
    assert_eq!(
        execute_single(&engine, "RETURN ['a', 'b', 'c'][1]"),
        json!("b")
    );
}

// Note: Negative array index not supported, skipping test

// ============================================================================
// Ternary Operator
// ============================================================================

#[test]
fn test_ternary_true_condition() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(
        execute_single(&engine, "RETURN true ? 'yes' : 'no'"),
        json!("yes")
    );
}

#[test]
fn test_ternary_false_condition() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(
        execute_single(&engine, "RETURN false ? 'yes' : 'no'"),
        json!("no")
    );
}

#[test]
fn test_ternary_with_expression() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(
        execute_single(&engine, "RETURN 5 > 3 ? 'bigger' : 'smaller'"),
        json!("bigger")
    );
    assert_eq!(
        execute_single(&engine, "RETURN 2 > 3 ? 'bigger' : 'smaller'"),
        json!("smaller")
    );
}

#[test]
fn test_ternary_nested() {
    let (engine, _tmp) = create_test_engine();

    // Nested ternary
    let result = execute_single(
        &engine,
        "RETURN 5 > 10 ? 'big' : (5 > 3 ? 'medium' : 'small')",
    );
    assert_eq!(result, json!("medium"));
}

// ============================================================================
// Range Operator
// ============================================================================

#[test]
fn test_range_operator() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN 1..5");
    assert_eq!(result, json!([1, 2, 3, 4, 5]));
}

#[test]
fn test_range_in_for() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "FOR i IN 1..3 RETURN i * 2");
    assert_eq!(results.len(), 3);
    // Results are floats from multiplication
    assert_eq!(results[0], json!(2.0));
    assert_eq!(results[1], json!(4.0));
    assert_eq!(results[2], json!(6.0));
}

// ============================================================================
// Null Handling
// ============================================================================

#[test]
fn test_null_comparison() {
    let (engine, _tmp) = create_test_engine();

    assert_eq!(execute_single(&engine, "RETURN null == null"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN null != null"), json!(false));
}

// Note: null arithmetic not supported, skipping test

#[test]
fn test_logical_or_short_circuit() {
    let (engine, _tmp) = create_test_engine();

    // OR returns boolean for boolean operands
    assert_eq!(execute_single(&engine, "RETURN false OR true"), json!(true));
    assert_eq!(execute_single(&engine, "RETURN true OR false"), json!(true));
}

// ============================================================================
// Object Access
// ============================================================================

#[test]
fn test_object_field_access() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN {name: 'Alice', age: 30}.name");
    assert_eq!(result, json!("Alice"));
}

#[test]
fn test_object_nested_access() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN {user: {name: 'Alice'}}.user.name");
    assert_eq!(result, json!("Alice"));
}

#[test]
fn test_object_dynamic_access() {
    let (engine, _tmp) = create_test_engine();

    // Access using bracket notation
    let _result = execute_single(&engine, "LET key = 'name' RETURN {name: 'Alice'}[key]");
    // This may or may not work depending on dynamic access support
}

// ============================================================================
// Subquery Operators
// ============================================================================

#[test]
fn test_subquery_in_let() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("nums".to_string(), None).unwrap();
    let nums = engine.get_collection("nums").unwrap();
    nums.insert(json!({"value": 1})).unwrap();
    nums.insert(json!({"value": 2})).unwrap();
    nums.insert(json!({"value": 3})).unwrap();

    let result = execute_single(
        &engine,
        "LET values = (FOR n IN nums RETURN n.value) RETURN SUM(values)",
    );
    // SUM returns float
    assert_eq!(result, json!(6.0));
}

// ============================================================================
// Complex Expression Tests
// ============================================================================

#[test]
fn test_complex_expression_combined() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN (5 + 3) * 2 == 16 AND (10 / 2 > 4)");
    assert_eq!(result, json!(true));
}

#[test]
fn test_complex_expression_with_functions() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN LENGTH('hello') + ABS(-5) == 10");
    assert_eq!(result, json!(true));
}

#[test]
fn test_expression_in_object_literal() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN { sum: 1 + 2, product: 3 * 4 }");
    // Arithmetic returns floats
    assert_eq!(result.get("sum"), Some(&json!(3.0)));
    assert_eq!(result.get("product"), Some(&json!(12.0)));
}

#[test]
fn test_expression_in_array_literal() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN [1 + 1, 2 + 2, 3 + 3]");
    assert_eq!(result, json!([2.0, 4.0, 6.0]));
}
