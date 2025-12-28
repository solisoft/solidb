//! SDBQL Function Executor Tests
//!
//! Tests for all built-in SDBQL functions covering:
//! - Aggregate functions (SUM, AVG, MIN, MAX, COUNT, etc.)
//! - String functions (UPPER, LOWER, TRIM, CONCAT, etc.)
//! - Array functions (LENGTH, FIRST, LAST, SLICE, etc.)
//! - Numeric functions (ABS, SQRT, POW, ROUND, etc.)
//! - Type functions (IS_ARRAY, IS_STRING, TYPENAME, etc.)
//! - Date functions (DATE_NOW, DATE_ADD, etc.)
//! - Object functions (HAS, KEEP, MERGE, etc.)
//! - Conversion functions (TO_STRING, TO_NUMBER, etc.)

use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;
use serde_json::json;

/// Helper to execute a query and get the first result
fn execute_query(engine: &StorageEngine, query_str: &str) -> serde_json::Value {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    let results = executor.execute(&query).expect(&format!("Query failed: {}", query_str));
    if results.is_empty() {
        serde_json::Value::Null
    } else {
        results[0].clone()
    }
}

/// Create a test engine
fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

// ============================================================================
// Numeric Functions
// ============================================================================

#[test]
fn test_function_abs() {
    let (engine, _tmp) = create_test_engine();
    
    // ABS returns floating point
    assert_eq!(execute_query(&engine, "RETURN ABS(-5)"), json!(5.0));
    assert_eq!(execute_query(&engine, "RETURN ABS(5)"), json!(5.0));
    assert_eq!(execute_query(&engine, "RETURN ABS(-3.14)"), json!(3.14));
    assert_eq!(execute_query(&engine, "RETURN ABS(0)"), json!(0.0));
}

#[test]
fn test_function_sqrt() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN SQRT(4)"), json!(2.0));
    assert_eq!(execute_query(&engine, "RETURN SQRT(9)"), json!(3.0));
    assert_eq!(execute_query(&engine, "RETURN SQRT(2)"), json!(1.4142135623730951));
}

#[test]
fn test_function_pow() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN POW(2, 3)"), json!(8.0));
    assert_eq!(execute_query(&engine, "RETURN POW(10, 2)"), json!(100.0));
    assert_eq!(execute_query(&engine, "RETURN POWER(5, 0)"), json!(1.0));
}

#[test]
fn test_function_round() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN ROUND(3.7)"), json!(4.0));
    assert_eq!(execute_query(&engine, "RETURN ROUND(3.2)"), json!(3.0));
    assert_eq!(execute_query(&engine, "RETURN ROUND(3.5)"), json!(4.0));
}

#[test]
fn test_function_floor() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN FLOOR(3.7)"), json!(3.0));
    assert_eq!(execute_query(&engine, "RETURN FLOOR(3.2)"), json!(3.0));
    assert_eq!(execute_query(&engine, "RETURN FLOOR(-2.5)"), json!(-3.0));
}

#[test]
fn test_function_ceil() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN CEIL(3.2)"), json!(4.0));
    assert_eq!(execute_query(&engine, "RETURN CEIL(3.7)"), json!(4.0));
    assert_eq!(execute_query(&engine, "RETURN CEIL(-2.5)"), json!(-2.0));
}

#[test]
fn test_function_random() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN RANDOM()");
    let val = result.as_f64().expect("RANDOM should return a number");
    assert!(val >= 0.0 && val < 1.0, "RANDOM should return value in [0, 1)");
}

// ============================================================================
// String Functions
// ============================================================================

#[test]
fn test_function_upper() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN UPPER('hello')"), json!("HELLO"));
    assert_eq!(execute_query(&engine, "RETURN UPPER('Hello World')"), json!("HELLO WORLD"));
}

#[test]
fn test_function_lower() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN LOWER('HELLO')"), json!("hello"));
    assert_eq!(execute_query(&engine, "RETURN LOWER('Hello World')"), json!("hello world"));
}

#[test]
fn test_function_trim() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN TRIM('  hello  ')"), json!("hello"));
    assert_eq!(execute_query(&engine, "RETURN LTRIM('  hello  ')"), json!("hello  "));
    assert_eq!(execute_query(&engine, "RETURN RTRIM('  hello  ')"), json!("  hello"));
}

#[test]
fn test_function_concat() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN CONCAT('Hello', ' ', 'World')"), json!("Hello World"));
    assert_eq!(execute_query(&engine, "RETURN CONCAT('a', 'b', 'c')"), json!("abc"));
}

#[test]
fn test_function_concat_separator() {
    let (engine, _tmp) = create_test_engine();
    
    // CONCAT_SEPARATOR takes separator and array
    assert_eq!(execute_query(&engine, "RETURN CONCAT_SEPARATOR(', ', ['a', 'b', 'c'])"), json!("a, b, c"));
    assert_eq!(execute_query(&engine, "RETURN CONCAT_SEPARATOR('-', ['2024', '01', '15'])"), json!("2024-01-15"));
}

#[test]
fn test_function_substring() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN SUBSTRING('Hello World', 0, 5)"), json!("Hello"));
    assert_eq!(execute_query(&engine, "RETURN SUBSTRING('Hello World', 6, 5)"), json!("World"));
}

#[test]
fn test_function_contains() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN CONTAINS('Hello World', 'World')"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN CONTAINS('Hello World', 'xyz')"), json!(false));
}

#[test]
fn test_function_split() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN SPLIT('a,b,c', ',')"), json!(["a", "b", "c"]));
    assert_eq!(execute_query(&engine, "RETURN SPLIT('hello world', ' ')"), json!(["hello", "world"]));
}

#[test]
fn test_function_substitute() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN SUBSTITUTE('Hello World', 'World', 'Universe')"), json!("Hello Universe"));
}

#[test]
fn test_function_regex_replace() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN REGEX_REPLACE('Hello 123 World', '[0-9]+', 'XXX')"), json!("Hello XXX World"));
}

#[test]
fn test_function_levenshtein() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN LEVENSHTEIN('kitten', 'sitting')"), json!(3));
    assert_eq!(execute_query(&engine, "RETURN LEVENSHTEIN('hello', 'hello')"), json!(0));
}

// ============================================================================
// Array Functions
// ============================================================================

#[test]
fn test_function_length() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN LENGTH([1, 2, 3, 4, 5])"), json!(5));
    assert_eq!(execute_query(&engine, "RETURN LENGTH('hello')"), json!(5));
    assert_eq!(execute_query(&engine, "RETURN LENGTH([])"), json!(0));
}

#[test]
fn test_function_first() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN FIRST([1, 2, 3])"), json!(1));
    assert_eq!(execute_query(&engine, "RETURN FIRST(['a', 'b', 'c'])"), json!("a"));
}

#[test]
fn test_function_last() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN LAST([1, 2, 3])"), json!(3));
    assert_eq!(execute_query(&engine, "RETURN LAST(['a', 'b', 'c'])"), json!("c"));
}

#[test]
fn test_function_nth() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN NTH([10, 20, 30, 40], 0)"), json!(10));
    assert_eq!(execute_query(&engine, "RETURN NTH([10, 20, 30, 40], 2)"), json!(30));
}

#[test]
fn test_function_slice() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN SLICE([1, 2, 3, 4, 5], 1, 3)"), json!([2, 3, 4]));
    assert_eq!(execute_query(&engine, "RETURN SLICE([1, 2, 3, 4, 5], 0, 2)"), json!([1, 2]));
}

#[test]
fn test_function_reverse() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN REVERSE([1, 2, 3])"), json!([3, 2, 1]));
    assert_eq!(execute_query(&engine, "RETURN REVERSE(['a', 'b', 'c'])"), json!(["c", "b", "a"]));
}

#[test]
fn test_function_unique() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN UNIQUE([1, 2, 2, 3, 3, 3])");
    let arr = result.as_array().expect("Should return array");
    assert_eq!(arr.len(), 3);
}

#[test]
fn test_function_sorted() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN SORTED([3, 1, 4, 1, 5])"), json!([1, 1, 3, 4, 5]));
}

#[test]
fn test_function_flatten() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN FLATTEN([[1, 2], [3, 4], [5]])"), json!([1, 2, 3, 4, 5]));
}

#[test]
fn test_function_push() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN PUSH([1, 2, 3], 4)"), json!([1, 2, 3, 4]));
}

#[test]
fn test_function_append() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN APPEND([1, 2], [3, 4])"), json!([1, 2, 3, 4]));
}

#[test]
fn test_function_union() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN UNION([1, 2], [3, 4])"), json!([1, 2, 3, 4]));
}

#[test]
fn test_function_intersection() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN INTERSECTION([1, 2, 3], [2, 3, 4])");
    let arr = result.as_array().expect("Should return array");
    assert!(arr.contains(&json!(2)));
    assert!(arr.contains(&json!(3)));
    assert!(!arr.contains(&json!(1)));
    assert!(!arr.contains(&json!(4)));
}

#[test]
fn test_function_minus() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN MINUS([1, 2, 3, 4], [2, 4])");
    let arr = result.as_array().expect("Should return array");
    assert!(arr.contains(&json!(1)));
    assert!(arr.contains(&json!(3)));
    assert!(!arr.contains(&json!(2)));
}

#[test]
fn test_function_position() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN POSITION([10, 20, 30], 20)"), json!(1));
    assert_eq!(execute_query(&engine, "RETURN POSITION([10, 20, 30], 40)"), json!(-1));
}

// ============================================================================
// Aggregate Functions
// ============================================================================

#[test]
fn test_function_sum() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN SUM([1, 2, 3, 4, 5])"), json!(15.0));
    assert_eq!(execute_query(&engine, "RETURN SUM([10, 20, 30])"), json!(60.0));
}

#[test]
fn test_function_avg() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN AVG([1, 2, 3, 4, 5])"), json!(3.0));
    assert_eq!(execute_query(&engine, "RETURN AVG([10, 20, 30])"), json!(20.0));
}

#[test]
fn test_function_min() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN MIN([5, 2, 8, 1, 9])"), json!(1.0));
}

#[test]
fn test_function_max() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN MAX([5, 2, 8, 1, 9])"), json!(9.0));
}

#[test]
fn test_function_length_as_count() {
    let (engine, _tmp) = create_test_engine();
    
    // COUNT is a reserved keyword in parser, use LENGTH for counting
    assert_eq!(execute_query(&engine, "RETURN LENGTH([1, 2, 3, 4, 5])"), json!(5));
}

#[test]
fn test_function_count_distinct() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN COUNT_DISTINCT([1, 2, 2, 3, 3, 3])"), json!(3));
}

#[test]
fn test_function_median() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN MEDIAN([1, 2, 3, 4, 5])"), json!(3.0));
    assert_eq!(execute_query(&engine, "RETURN MEDIAN([1, 2, 3, 4])"), json!(2.5));
}

#[test]
fn test_function_variance() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN VARIANCE([2, 4, 4, 4, 5, 5, 7, 9])");
    let val = result.as_f64().expect("Should return number");
    assert!((val - 4.0).abs() < 0.0001, "Variance should be 4.0");
}

#[test]
fn test_function_stddev() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN STDDEV([2, 4, 4, 4, 5, 5, 7, 9])");
    let val = result.as_f64().expect("Should return number");
    // Sample standard deviation for this dataset
    assert!(val > 1.9 && val < 2.2, "StdDev should be approximately 2.0, got {}", val);
}

// ============================================================================
// Type Checking Functions
// ============================================================================

#[test]
fn test_function_is_array() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN IS_ARRAY([1, 2, 3])"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN IS_ARRAY('not an array')"), json!(false));
    assert_eq!(execute_query(&engine, "RETURN IS_ARRAY(123)"), json!(false));
}

#[test]
fn test_function_is_string() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN IS_STRING('hello')"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN IS_STRING(123)"), json!(false));
    assert_eq!(execute_query(&engine, "RETURN IS_STRING([1, 2])"), json!(false));
}

#[test]
fn test_function_is_number() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN IS_NUMBER(42)"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN IS_NUMBER(3.14)"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN IS_NUMBER('42')"), json!(false));
}

#[test]
fn test_function_is_bool() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN IS_BOOL(true)"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN IS_BOOL(false)"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN IS_BOOL(1)"), json!(false));
}

#[test]
fn test_function_is_null() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN IS_NULL(null)"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN IS_NULL(0)"), json!(false));
    assert_eq!(execute_query(&engine, "RETURN IS_NULL('')"), json!(false));
}

#[test]
fn test_function_is_object() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN IS_OBJECT({a: 1})"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN IS_OBJECT([1, 2])"), json!(false));
    assert_eq!(execute_query(&engine, "RETURN IS_OBJECT('string')"), json!(false));
}

#[test]
fn test_function_typename() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN TYPENAME(42)"), json!("int"));
    assert_eq!(execute_query(&engine, "RETURN TYPENAME('hello')"), json!("string"));
    assert_eq!(execute_query(&engine, "RETURN TYPENAME([1, 2])"), json!("array"));
    assert_eq!(execute_query(&engine, "RETURN TYPENAME({a: 1})"), json!("object"));
    assert_eq!(execute_query(&engine, "RETURN TYPENAME(true)"), json!("bool"));
    assert_eq!(execute_query(&engine, "RETURN TYPENAME(null)"), json!("null"));
}

// ============================================================================
// Conversion Functions
// ============================================================================

#[test]
fn test_function_to_string() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN TO_STRING(42)"), json!("42"));
    assert_eq!(execute_query(&engine, "RETURN TO_STRING(true)"), json!("true"));
}

#[test]
fn test_function_to_number() {
    let (engine, _tmp) = create_test_engine();
    
    // TO_NUMBER may return integer for whole numbers
    let result = execute_query(&engine, "RETURN TO_NUMBER('42')");
    assert!(result.as_f64().unwrap() == 42.0, "Should convert to 42");
    
    let result = execute_query(&engine, "RETURN TO_NUMBER('3.14')");
    assert!((result.as_f64().unwrap() - 3.14).abs() < 0.001, "Should convert to 3.14");
}

#[test]
fn test_function_to_bool() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN TO_BOOL(1)"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN TO_BOOL(0)"), json!(false));
    assert_eq!(execute_query(&engine, "RETURN TO_BOOL('true')"), json!(true));
}

#[test]
fn test_function_to_array() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN TO_ARRAY('hello')"), json!(["hello"]));
    assert_eq!(execute_query(&engine, "RETURN TO_ARRAY([1, 2])"), json!([1, 2]));
}

// ============================================================================
// Object Functions
// ============================================================================

#[test]
fn test_function_has() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN HAS({name: 'Alice', age: 30}, 'name')"), json!(true));
    assert_eq!(execute_query(&engine, "RETURN HAS({name: 'Alice', age: 30}, 'email')"), json!(false));
}

#[test]
fn test_function_attributes() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN ATTRIBUTES({a: 1, b: 2, c: 3})");
    let arr = result.as_array().expect("Should return array");
    assert!(arr.contains(&json!("a")));
    assert!(arr.contains(&json!("b")));
    assert!(arr.contains(&json!("c")));
}

#[test]
fn test_function_values() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN VALUES({a: 1, b: 2, c: 3})");
    let arr = result.as_array().expect("Should return array");
    assert!(arr.contains(&json!(1)));
    assert!(arr.contains(&json!(2)));
    assert!(arr.contains(&json!(3)));
}

#[test]
fn test_function_keep() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN KEEP({a: 1, b: 2, c: 3}, 'a', 'c')");
    let obj = result.as_object().expect("Should return object");
    assert_eq!(obj.get("a"), Some(&json!(1)));
    assert_eq!(obj.get("c"), Some(&json!(3)));
    assert_eq!(obj.get("b"), None);
}

#[test]
fn test_function_unset() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN UNSET({a: 1, b: 2, c: 3}, 'b')");
    let obj = result.as_object().expect("Should return object");
    assert_eq!(obj.get("a"), Some(&json!(1)));
    assert_eq!(obj.get("c"), Some(&json!(3)));
    assert_eq!(obj.get("b"), None);
}

#[test]
fn test_function_merge() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN MERGE({a: 1}, {b: 2}, {c: 3})");
    let obj = result.as_object().expect("Should return object");
    assert_eq!(obj.get("a"), Some(&json!(1)));
    assert_eq!(obj.get("b"), Some(&json!(2)));
    assert_eq!(obj.get("c"), Some(&json!(3)));
}

// ============================================================================
// Conditional Functions
// ============================================================================

#[test]
fn test_function_if() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_query(&engine, "RETURN IF(true, 'yes', 'no')"), json!("yes"));
    assert_eq!(execute_query(&engine, "RETURN IF(false, 'yes', 'no')"), json!("no"));
    assert_eq!(execute_query(&engine, "RETURN IF(1 > 0, 'positive', 'negative')"), json!("positive"));
}

// ============================================================================
// JSON Functions
// ============================================================================

#[test]
fn test_function_json_parse() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, r#"RETURN JSON_PARSE('{"name": "Alice"}')"#);
    let obj = result.as_object().expect("Should return object");
    assert_eq!(obj.get("name"), Some(&json!("Alice")));
}

#[test]
fn test_function_json_stringify() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN JSON_STRINGIFY({name: 'Alice'})");
    assert!(result.is_string());
    let s = result.as_str().unwrap();
    assert!(s.contains("name") && s.contains("Alice"));
}

// ============================================================================
// Date Functions
// ============================================================================

#[test]
fn test_function_date_now() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN DATE_NOW()");
    assert!(result.is_number(), "DATE_NOW should return a timestamp");
    let ts = result.as_i64().unwrap();
    assert!(ts > 1700000000000, "Timestamp should be recent"); // After Nov 2023
}

#[test]
fn test_function_date_timestamp() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_query(&engine, "RETURN DATE_TIMESTAMP('2024-01-15T12:00:00Z')");
    assert!(result.is_number());
}

// ============================================================================
// Geo Functions
// ============================================================================

#[test]
fn test_function_distance() {
    let (engine, _tmp) = create_test_engine();
    
    // Distance from Paris to London (approx 344 km)
    let result = execute_query(&engine, "RETURN DISTANCE(48.8566, 2.3522, 51.5074, -0.1278)");
    let dist = result.as_f64().expect("Should return distance");
    assert!(dist > 340000.0 && dist < 350000.0, "Distance should be ~344km in meters");
}
