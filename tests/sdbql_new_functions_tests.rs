//! SDBQL New Functions Tests
//!
//! Tests for newly added SDBQL functions:
//! - Math: LOG, LOG10, LOG2, EXP, SIN, COS, TAN, ASIN, ACOS, ATAN, ATAN2, PI
//! - String: LEFT, RIGHT, CHAR_LENGTH, FIND_FIRST, FIND_LAST, REGEX_TEST
//! - Null: COALESCE, NOT_NULL
//! - Date: DATE_YEAR, DATE_MONTH, DATE_DAY, DATE_HOUR, DATE_MINUTE, DATE_SECOND, DATE_DAYOFWEEK, DATE_QUARTER
//! - Array: RANGE

use serde_json::json;
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .execute(&query)
        .expect(&format!("Failed to execute: {}", query_str))
}

fn execute_single(engine: &StorageEngine, query_str: &str) -> serde_json::Value {
    let results = execute_query(engine, query_str);
    results
        .into_iter()
        .next()
        .unwrap_or(serde_json::Value::Null)
}

// ============================================================================
// Logarithm and Exponential Functions
// ============================================================================

#[test]
fn test_log_natural() {
    let (engine, _tmp) = create_test_engine();

    // ln(e) = 1
    let result = execute_single(&engine, "RETURN LOG(2.718281828)");
    let val = result.as_f64().unwrap();
    assert!((val - 1.0).abs() < 0.001);
}

#[test]
fn test_log10() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN LOG10(100)");
    assert_eq!(result.as_f64().unwrap(), 2.0);
}

#[test]
fn test_log10_1000() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN LOG10(1000)");
    assert_eq!(result.as_f64().unwrap(), 3.0);
}

#[test]
fn test_log2() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN LOG2(8)");
    assert_eq!(result.as_f64().unwrap(), 3.0);
}

#[test]
fn test_log2_256() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN LOG2(256)");
    assert_eq!(result.as_f64().unwrap(), 8.0);
}

#[test]
fn test_exp() {
    let (engine, _tmp) = create_test_engine();

    // e^1 ≈ 2.718
    let result = execute_single(&engine, "RETURN EXP(1)");
    let val = result.as_f64().unwrap();
    assert!((val - 2.718281828).abs() < 0.001);
}

#[test]
fn test_exp_zero() {
    let (engine, _tmp) = create_test_engine();

    // e^0 = 1
    let result = execute_single(&engine, "RETURN EXP(0)");
    assert_eq!(result.as_f64().unwrap(), 1.0);
}

// ============================================================================
// Trigonometric Functions
// ============================================================================

#[test]
fn test_sin_zero() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN SIN(0)");
    assert_eq!(result.as_f64().unwrap(), 0.0);
}

#[test]
fn test_cos_zero() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN COS(0)");
    assert_eq!(result.as_f64().unwrap(), 1.0);
}

#[test]
fn test_tan_zero() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN TAN(0)");
    assert_eq!(result.as_f64().unwrap(), 0.0);
}

#[test]
fn test_asin_zero() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN ASIN(0)");
    assert_eq!(result.as_f64().unwrap(), 0.0);
}

#[test]
fn test_acos_one() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN ACOS(1)");
    assert_eq!(result.as_f64().unwrap(), 0.0);
}

#[test]
fn test_atan_zero() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN ATAN(0)");
    assert_eq!(result.as_f64().unwrap(), 0.0);
}

#[test]
fn test_atan2() {
    let (engine, _tmp) = create_test_engine();

    // atan2(0, 1) = 0
    let result = execute_single(&engine, "RETURN ATAN2(0, 1)");
    assert_eq!(result.as_f64().unwrap(), 0.0);
}

#[test]
fn test_pi() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN PI()");
    let val = result.as_f64().unwrap();
    assert!((val - 3.14159265).abs() < 0.0001);
}

#[test]
fn test_sin_cos_identity() {
    let (engine, _tmp) = create_test_engine();

    // sin^2(x) + cos^2(x) = 1
    let result = execute_single(
        &engine,
        "LET x = 1.0 RETURN POW(SIN(x), 2) + POW(COS(x), 2)",
    );
    let val = result.as_f64().unwrap();
    assert!((val - 1.0).abs() < 0.0001);
}

// ============================================================================
// String Functions: LEFT, RIGHT, CHAR_LENGTH
// ============================================================================

#[test]
fn test_left() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN LEFT(\"Hello World\", 5)");
    assert_eq!(result.as_str().unwrap(), "Hello");
}

#[test]
fn test_left_unicode() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN LEFT(\"日本語テスト\", 3)");
    assert_eq!(result.as_str().unwrap(), "日本語");
}

#[test]
fn test_right() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN RIGHT(\"Hello World\", 5)");
    assert_eq!(result.as_str().unwrap(), "World");
}

#[test]
fn test_right_unicode() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN RIGHT(\"日本語テスト\", 3)");
    assert_eq!(result.as_str().unwrap(), "テスト");
}

#[test]
fn test_char_length() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN CHAR_LENGTH(\"Hello\")");
    assert_eq!(result.as_i64().unwrap(), 5);
}

#[test]
fn test_char_length_unicode() {
    let (engine, _tmp) = create_test_engine();

    // 6 characters (not bytes)
    let result = execute_single(&engine, "RETURN CHAR_LENGTH(\"日本語テスト\")");
    assert_eq!(result.as_i64().unwrap(), 6);
}

// ============================================================================
// String Functions: FIND_FIRST, FIND_LAST
// ============================================================================

#[test]
fn test_find_first() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN FIND_FIRST(\"Hello World\", \"o\")");
    assert_eq!(result.as_i64().unwrap(), 4);
}

#[test]
fn test_find_first_not_found() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN FIND_FIRST(\"Hello World\", \"z\")");
    assert_eq!(result.as_i64().unwrap(), -1);
}

#[test]
fn test_find_last() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN FIND_LAST(\"Hello World\", \"o\")");
    assert_eq!(result.as_i64().unwrap(), 7);
}

#[test]
fn test_find_last_not_found() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN FIND_LAST(\"Hello World\", \"z\")");
    assert_eq!(result.as_i64().unwrap(), -1);
}

// ============================================================================
// REGEX_TEST Function
// ============================================================================

#[test]
fn test_regex_test_match() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN REGEX_TEST(\"hello123\", \"[a-z]+[0-9]+\")");
    assert_eq!(result, json!(true));
}

#[test]
fn test_regex_test_no_match() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN REGEX_TEST(\"hello\", \"^[0-9]+$\")");
    assert_eq!(result, json!(false));
}

#[test]
fn test_regex_test_email() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN REGEX_TEST(\"test@example.com\", \"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\\\\.[a-zA-Z]{2,}$\")");
    assert_eq!(result, json!(true));
}

// ============================================================================
// COALESCE / NOT_NULL Functions
// ============================================================================

#[test]
fn test_coalesce_first_not_null() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN COALESCE(\"hello\", \"world\")");
    assert_eq!(result.as_str().unwrap(), "hello");
}

#[test]
fn test_coalesce_skip_null() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN COALESCE(NULL, \"world\")");
    assert_eq!(result.as_str().unwrap(), "world");
}

#[test]
fn test_coalesce_all_null() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN COALESCE(NULL, NULL, NULL)");
    assert!(result.is_null());
}

#[test]
fn test_not_null_alias() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN NOT_NULL(NULL, 42)");
    assert_eq!(result.as_i64().unwrap(), 42);
}

#[test]
fn test_coalesce_with_zero() {
    let (engine, _tmp) = create_test_engine();

    // 0 is not null, should be returned
    let result = execute_single(&engine, "RETURN COALESCE(0, 100)");
    assert_eq!(result.as_i64().unwrap(), 0);
}

// ============================================================================
// Date Extraction Functions
// ============================================================================

#[test]
fn test_date_year() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN DATE_YEAR(\"2024-12-30T10:30:45.000Z\")");
    assert_eq!(result.as_i64().unwrap(), 2024);
}

#[test]
fn test_date_month() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN DATE_MONTH(\"2024-12-30T10:30:45.000Z\")");
    assert_eq!(result.as_i64().unwrap(), 12);
}

#[test]
fn test_date_day() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN DATE_DAY(\"2024-12-30T10:30:45.000Z\")");
    assert_eq!(result.as_i64().unwrap(), 30);
}

#[test]
fn test_date_hour() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN DATE_HOUR(\"2024-12-30T10:30:45.000Z\")");
    assert_eq!(result.as_i64().unwrap(), 10);
}

#[test]
fn test_date_minute() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN DATE_MINUTE(\"2024-12-30T10:30:45.000Z\")");
    assert_eq!(result.as_i64().unwrap(), 30);
}

#[test]
fn test_date_second() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN DATE_SECOND(\"2024-12-30T10:30:45.000Z\")");
    assert_eq!(result.as_i64().unwrap(), 45);
}

#[test]
fn test_date_quarter() {
    let (engine, _tmp) = create_test_engine();

    // December is Q4
    let result = execute_single(&engine, "RETURN DATE_QUARTER(\"2024-12-30T10:30:45.000Z\")");
    assert_eq!(result.as_i64().unwrap(), 4);
}

#[test]
fn test_date_quarter_q1() {
    let (engine, _tmp) = create_test_engine();

    // March is Q1
    let result = execute_single(&engine, "RETURN DATE_QUARTER(\"2024-03-15T10:30:45.000Z\")");
    assert_eq!(result.as_i64().unwrap(), 1);
}

#[test]
fn test_date_dayofweek() {
    let (engine, _tmp) = create_test_engine();

    // 2024-12-30 is Monday (1)
    let result = execute_single(
        &engine,
        "RETURN DATE_DAYOFWEEK(\"2024-12-30T10:30:45.000Z\")",
    );
    assert_eq!(result.as_i64().unwrap(), 1); // Monday
}

#[test]
fn test_date_year_from_timestamp() {
    let (engine, _tmp) = create_test_engine();

    // 1735555845000 = 2024-12-30T...
    let result = execute_single(&engine, "RETURN DATE_YEAR(1735555845000)");
    assert_eq!(result.as_i64().unwrap(), 2024);
}

// ============================================================================
// RANGE Function
// ============================================================================

#[test]
fn test_range_simple() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN RANGE(1, 5)");
    assert_eq!(result, json!([1, 2, 3, 4, 5]));
}

#[test]
fn test_range_with_step() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN RANGE(0, 10, 2)");
    assert_eq!(result, json!([0, 2, 4, 6, 8, 10]));
}

#[test]
fn test_range_descending() {
    let (engine, _tmp) = create_test_engine();

    // Use subtraction to get negative step
    let result = execute_single(&engine, "RETURN RANGE(5, 1, 0 - 1)");
    assert_eq!(result, json!([5, 4, 3, 2, 1]));
}

#[test]
fn test_range_empty() {
    let (engine, _tmp) = create_test_engine();

    // Start > end with positive step = empty
    let result = execute_single(&engine, "RETURN RANGE(5, 1, 1)");
    assert_eq!(result, json!([]));
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_date_extraction_chain() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(
        &engine,
        "LET d = \"2024-12-30T10:30:45.000Z\"
         RETURN { year: DATE_YEAR(d), month: DATE_MONTH(d), day: DATE_DAY(d) }",
    );

    assert_eq!(result["year"].as_i64().unwrap(), 2024);
    assert_eq!(result["month"].as_i64().unwrap(), 12);
    assert_eq!(result["day"].as_i64().unwrap(), 30);
}

#[test]
fn test_coalesce_with_collection() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    users
        .insert(json!({"_key": "u1", "name": "Alice", "nickname": null}))
        .unwrap();
    users
        .insert(json!({"_key": "u2", "name": "Bob", "nickname": "Bobby"}))
        .unwrap();

    let results = execute_query(
        &engine,
        "FOR u IN users RETURN COALESCE(u.nickname, u.name)",
    );

    let names: Vec<&str> = results.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(names.contains(&"Alice")); // nickname is null, use name
    assert!(names.contains(&"Bobby")); // nickname is set
}

#[test]
fn test_range_sum() {
    let (engine, _tmp) = create_test_engine();

    // Sum of 1 to 10 = 55
    let result = execute_single(&engine, "RETURN SUM(RANGE(1, 10))");
    assert_eq!(result.as_f64().unwrap(), 55.0);
}
