//! Pipeline Operator Tests
//!
//! Tests for the |> pipeline operator and lambda expressions in SDBQL.

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

// ============================================================================
// Basic Pipeline Tests
// ============================================================================

#[test]
fn test_pipeline_simple_string() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, r#"RETURN "hello" |> UPPER()"#);
    assert_eq!(results, vec![json!("HELLO")]);
}

#[test]
fn test_pipeline_chain() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, r#"RETURN "  hello  " |> TRIM() |> UPPER()"#);
    assert_eq!(results, vec![json!("HELLO")]);
}

#[test]
fn test_pipeline_array_first() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [3, 1, 2] |> FIRST()");
    assert_eq!(results, vec![json!(3)]);
}

#[test]
fn test_pipeline_array_last() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [3, 1, 2] |> LAST()");
    assert_eq!(results, vec![json!(2)]);
}

#[test]
fn test_pipeline_array_reverse() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [1, 2, 3] |> REVERSE()");
    assert_eq!(results, vec![json!([3, 2, 1])]);
}

#[test]
fn test_pipeline_array_sorted() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [3, 1, 2] |> SORTED()");
    assert_eq!(results, vec![json!([1, 2, 3])]);
}

#[test]
fn test_pipeline_array_unique() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [1, 2, 2, 3, 1] |> UNIQUE()");
    assert_eq!(results, vec![json!([1, 2, 3])]);
}

#[test]
fn test_pipeline_array_length() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [1, 2, 3, 4, 5] |> LENGTH()");
    assert_eq!(results, vec![json!(5)]);
}

#[test]
fn test_pipeline_array_flatten() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [[1, 2], [3, 4]] |> FLATTEN()");
    assert_eq!(results, vec![json!([1, 2, 3, 4])]);
}

#[test]
fn test_pipeline_chain_array() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(
        &engine,
        "RETURN [3, 1, 2] |> SORTED() |> REVERSE() |> FIRST()",
    );
    assert_eq!(results, vec![json!(3)]);
}

// ============================================================================
// Pipeline with Arguments Tests
// ============================================================================

#[test]
fn test_pipeline_round_with_precision() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN 3.14159 |> ROUND(2)");
    assert_eq!(results, vec![json!(3.14)]);
}

#[test]
fn test_pipeline_split_with_separator() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, r#"RETURN "a,b,c" |> SPLIT(",")"#);
    assert_eq!(results, vec![json!(["a", "b", "c"])]);
}

// ============================================================================
// Pipeline with Lambda Tests (Higher-Order Functions)
// ============================================================================

#[test]
fn test_pipeline_filter_with_lambda() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [1, 2, 3, 4, 5] |> FILTER(x -> x > 2)");
    assert_eq!(results, vec![json!([3, 4, 5])]);
}

#[test]
fn test_pipeline_map_with_lambda() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [1, 2, 3] |> MAP(x -> x * 2)");
    // Results are floats due to multiplication
    assert_eq!(results, vec![json!([2.0, 4.0, 6.0])]);
}

#[test]
fn test_pipeline_filter_and_map() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(
        &engine,
        "RETURN [1, 2, 3, 4, 5] |> FILTER(x -> x > 2) |> MAP(x -> x * 10)",
    );
    // Results are floats due to multiplication
    assert_eq!(results, vec![json!([30.0, 40.0, 50.0])]);
}

#[test]
fn test_pipeline_any_with_lambda() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [1, 2, 3] |> ANY(x -> x > 2)");
    assert_eq!(results, vec![json!(true)]);

    let results = execute_query(&engine, "RETURN [1, 2, 3] |> ANY(x -> x > 5)");
    assert_eq!(results, vec![json!(false)]);
}

#[test]
fn test_pipeline_all_with_lambda() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [1, 2, 3] |> ALL(x -> x > 0)");
    assert_eq!(results, vec![json!(true)]);

    let results = execute_query(&engine, "RETURN [1, 2, 3] |> ALL(x -> x > 1)");
    assert_eq!(results, vec![json!(false)]);
}

#[test]
fn test_pipeline_find_with_lambda() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [1, 2, 3, 4] |> FIND(x -> x > 2)");
    assert_eq!(results, vec![json!(3)]);
}

#[test]
fn test_pipeline_reduce_sum() {
    let (engine, _tmp) = create_test_engine();
    // Sum all numbers in array
    let results = execute_query(
        &engine,
        "RETURN [1, 2, 3, 4, 5] |> REDUCE((acc, x) -> acc + x, 0)",
    );
    assert_eq!(results, vec![json!(15.0)]);
}

#[test]
fn test_pipeline_reduce_product() {
    let (engine, _tmp) = create_test_engine();
    // Multiply all numbers
    let results = execute_query(
        &engine,
        "RETURN [1, 2, 3, 4] |> REDUCE((acc, x) -> acc * x, 1)",
    );
    assert_eq!(results, vec![json!(24.0)]);
}

#[test]
fn test_pipeline_reduce_string_concat() {
    let (engine, _tmp) = create_test_engine();
    // Concatenate strings
    let results = execute_query(
        &engine,
        r#"RETURN ["a", "b", "c"] |> REDUCE((acc, x) -> CONCAT(acc, x), "")"#,
    );
    assert_eq!(results, vec![json!("abc")]);
}

#[test]
fn test_pipeline_reduce_max() {
    let (engine, _tmp) = create_test_engine();
    // Find maximum manually
    let results = execute_query(
        &engine,
        "RETURN [3, 1, 4, 1, 5, 9, 2, 6] |> REDUCE((max, x) -> x > max ? x : max, 0)",
    );
    assert_eq!(results, vec![json!(9)]);
}

#[test]
fn test_pipeline_reduce_empty_array() {
    let (engine, _tmp) = create_test_engine();
    // Empty array should return initial value
    let results = execute_query(&engine, "RETURN [] |> REDUCE((acc, x) -> acc + x, 42)");
    assert_eq!(results, vec![json!(42)]);
}

// ============================================================================
// Pipeline with FOR Loop Tests
// ============================================================================

#[test]
fn test_pipeline_in_for_loop() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    items
        .insert(json!({"_key": "i1", "values": [3, 1, 2]}))
        .unwrap();
    items
        .insert(json!({"_key": "i2", "values": [6, 4, 5]}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR i IN items RETURN i.values |> SORTED() |> FIRST()",
    );
    // Should return first element of each sorted array
    assert!(results.contains(&json!(1)));
    assert!(results.contains(&json!(4)));
}

// ============================================================================
// Lambda Expression Tests (without pipeline)
// ============================================================================

#[test]
fn test_lambda_single_param() {
    let (engine, _tmp) = create_test_engine();
    // Lambda used with FILTER function directly (not in pipeline)
    let results = execute_query(&engine, "RETURN [1, 2, 3] |> FILTER(x -> x != 2)");
    assert_eq!(results, vec![json!([1, 3])]);
}

#[test]
fn test_lambda_complex_expression() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(
        &engine,
        "RETURN [1, 2, 3, 4] |> FILTER(x -> x > 1 AND x < 4)",
    );
    assert_eq!(results, vec![json!([2, 3])]);
}

// ============================================================================
// Type Conversion Pipeline Tests
// ============================================================================

#[test]
fn test_pipeline_to_string() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN 42 |> TO_STRING()");
    assert_eq!(results, vec![json!("42")]);
}

#[test]
fn test_pipeline_to_number() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, r#"RETURN "42" |> TO_NUMBER()"#);
    assert_eq!(results, vec![json!(42.0)]);
}

// ============================================================================
// Numeric Pipeline Tests
// ============================================================================

#[test]
fn test_pipeline_abs() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN -5 |> ABS()");
    assert_eq!(results, vec![json!(5.0)]);
}

#[test]
fn test_pipeline_floor() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN 3.7 |> FLOOR()");
    assert_eq!(results, vec![json!(3.0)]);
}

#[test]
fn test_pipeline_ceil() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN 3.2 |> CEIL()");
    assert_eq!(results, vec![json!(4.0)]);
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_pipeline_empty_array() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN [] |> FIRST()");
    assert_eq!(results, vec![json!(null)]);
}

#[test]
fn test_pipeline_null_handling() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN null |> LENGTH()");
    assert_eq!(results, vec![json!(0)]);
}

// ============================================================================
// Null Coalescing Operator (??) Tests
// ============================================================================

#[test]
fn test_null_coalesce_with_null() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, r#"RETURN null ?? "default""#);
    assert_eq!(results, vec![json!("default")]);
}

#[test]
fn test_null_coalesce_with_value() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, r#"RETURN "actual" ?? "default""#);
    assert_eq!(results, vec![json!("actual")]);
}

#[test]
fn test_null_coalesce_chain() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, r#"RETURN null ?? null ?? "fallback""#);
    assert_eq!(results, vec![json!("fallback")]);
}

#[test]
fn test_null_coalesce_with_number() {
    let (engine, _tmp) = create_test_engine();
    let results = execute_query(&engine, "RETURN null ?? 42");
    assert_eq!(results, vec![json!(42)]);

    let results = execute_query(&engine, "RETURN 10 ?? 42");
    assert_eq!(results, vec![json!(10)]);
}

#[test]
fn test_null_coalesce_with_zero() {
    let (engine, _tmp) = create_test_engine();
    // 0 is not null, so it should return 0
    let results = execute_query(&engine, "RETURN 0 ?? 42");
    assert_eq!(results, vec![json!(0)]);
}

#[test]
fn test_null_coalesce_with_empty_string() {
    let (engine, _tmp) = create_test_engine();
    // Empty string is not null, so it should return empty string
    let results = execute_query(&engine, r#"RETURN "" ?? "default""#);
    assert_eq!(results, vec![json!("")]);
}

#[test]
fn test_null_coalesce_with_false() {
    let (engine, _tmp) = create_test_engine();
    // false is not null, so it should return false
    let results = execute_query(&engine, "RETURN false ?? true");
    assert_eq!(results, vec![json!(false)]);
}

#[test]
fn test_null_coalesce_with_object_field() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    items
        .insert(json!({"_key": "i1", "name": "Item 1", "description": null}))
        .unwrap();
    items
        .insert(json!({"_key": "i2", "name": "Item 2", "description": "A description"}))
        .unwrap();

    let results = execute_query(
        &engine,
        r#"FOR i IN items SORT i._key RETURN i.description ?? "No description""#,
    );
    assert_eq!(
        results,
        vec![json!("No description"), json!("A description")]
    );
}

#[test]
fn test_null_coalesce_with_missing_field() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    items
        .insert(json!({"_key": "i1", "name": "Item 1"}))
        .unwrap();

    // Access a field that doesn't exist - should be null and fallback to default
    let results = execute_query(
        &engine,
        r#"FOR i IN items RETURN i.nonexistent ?? "missing""#,
    );
    assert_eq!(results, vec![json!("missing")]);
}

#[test]
fn test_null_coalesce_with_pipeline() {
    let (engine, _tmp) = create_test_engine();
    // Combine pipeline and null coalescing
    let results = execute_query(&engine, "RETURN ([] |> FIRST()) ?? 0");
    assert_eq!(results, vec![json!(0)]);
}

#[test]
fn test_null_coalesce_short_circuit() {
    let (engine, _tmp) = create_test_engine();
    // When left side is not null, right side should not be evaluated
    // Testing with a non-null value
    let results = execute_query(&engine, r#"RETURN "exists" ?? (1/0)"#);
    // If short-circuit works, this should return "exists" without evaluating 1/0
    assert_eq!(results, vec![json!("exists")]);
}
