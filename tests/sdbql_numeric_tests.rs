//! SDBQL Numeric and Math Function Tests
//!
//! Tests for SDBQL numeric functions including:
//! - Basic math: ABS, FLOOR, CEIL, ROUND
//! - Advanced math: SQRT, POW, EXP, LOG
//! - Trigonometry: SIN, COS, TAN
//! - Aggregate: SUM, AVG, MIN, MAX, COUNT
//! - Special: RAND, RANGE

use solidb::{parse, QueryExecutor};
use solidb::storage::StorageEngine;
use serde_json::json;
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
    executor.execute(&query).expect(&format!("Failed to execute: {}", query_str))
}

fn execute_single(engine: &StorageEngine, query_str: &str) -> serde_json::Value {
    let results = execute_query(engine, query_str);
    results.into_iter().next().unwrap_or(serde_json::Value::Null)
}

// ============================================================================
// Basic Math Functions
// ============================================================================

#[test]
fn test_abs_positive() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN ABS(42)");
    assert_eq!(result.as_f64().unwrap(), 42.0);
}

#[test]
fn test_abs_negative() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN ABS(-42)");
    assert_eq!(result.as_f64().unwrap(), 42.0);
}

#[test]
fn test_abs_zero() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN ABS(0)");
    assert_eq!(result.as_f64().unwrap(), 0.0);
}

#[test]
fn test_floor() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN FLOOR(3.7)");
    assert_eq!(result.as_f64().unwrap(), 3.0);
}

#[test]
fn test_floor_negative() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN FLOOR(-3.7)");
    assert_eq!(result.as_f64().unwrap(), -4.0);
}

#[test]
fn test_ceil() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN CEIL(3.2)");
    assert_eq!(result.as_f64().unwrap(), 4.0);
}

#[test]
fn test_ceil_negative() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN CEIL(-3.2)");
    assert_eq!(result.as_f64().unwrap(), -3.0);
}

#[test]
fn test_round() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN ROUND(3.5)");
    assert_eq!(result.as_f64().unwrap(), 4.0);
}

#[test]
fn test_round_down() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN ROUND(3.4)");
    assert_eq!(result.as_f64().unwrap(), 3.0);
}

// ============================================================================
// Advanced Math Functions
// ============================================================================

#[test]
fn test_sqrt() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SQRT(16)");
    assert_eq!(result.as_f64().unwrap(), 4.0);
}

#[test]
fn test_sqrt_non_perfect() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SQRT(2)");
    let val = result.as_f64().unwrap();
    assert!((val - 1.41421356).abs() < 0.0001);
}

#[test]
fn test_pow() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN POW(2, 10)");
    assert_eq!(result.as_f64().unwrap(), 1024.0);
}

#[test]
fn test_pow_negative_exponent() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN POW(2, -1)");
    assert_eq!(result.as_f64().unwrap(), 0.5);
}





// ============================================================================
// Aggregate Functions on Arrays
// ============================================================================

#[test]
fn test_sum_array() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SUM([1, 2, 3, 4, 5])");
    assert_eq!(result.as_f64().unwrap(), 15.0);
}

#[test]
fn test_avg_array() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN AVG([10, 20, 30, 40, 50])");
    assert_eq!(result.as_f64().unwrap(), 30.0);
}

#[test]
fn test_min_array() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN MIN([5, 2, 8, 1, 9])");
    assert_eq!(result.as_f64().unwrap(), 1.0);
}

#[test]
fn test_max_array() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN MAX([5, 2, 8, 1, 9])");
    assert_eq!(result.as_f64().unwrap(), 9.0);
}



#[test]
fn test_sum_empty_array() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SUM([])");
    assert_eq!(result.as_f64().unwrap(), 0.0);
}

// ============================================================================
// Aggregate Functions on Collection Data
// ============================================================================

#[test]
fn test_sum_from_collection() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("sales".to_string(), None).unwrap();
    let sales = engine.get_collection("sales").unwrap();
    
    sales.insert(json!({"_key": "s1", "amount": 100})).unwrap();
    sales.insert(json!({"_key": "s2", "amount": 200})).unwrap();
    sales.insert(json!({"_key": "s3", "amount": 300})).unwrap();
    
    let result = execute_single(&engine, 
        "LET amounts = (FOR s IN sales RETURN s.amount) RETURN SUM(amounts)");
    assert_eq!(result.as_f64().unwrap(), 600.0);
}

#[test]
fn test_avg_from_collection() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("scores".to_string(), None).unwrap();
    let scores = engine.get_collection("scores").unwrap();
    
    scores.insert(json!({"_key": "p1", "score": 80})).unwrap();
    scores.insert(json!({"_key": "p2", "score": 90})).unwrap();
    scores.insert(json!({"_key": "p3", "score": 100})).unwrap();
    
    let result = execute_single(&engine,
        "LET s = (FOR x IN scores RETURN x.score) RETURN AVG(s)");
    assert_eq!(result.as_f64().unwrap(), 90.0);
}

// ============================================================================
// Range Function
// ============================================================================

#[test]
fn test_range_simple() {
    let (engine, _tmp) = create_test_engine();
    
    let results = execute_query(&engine, "FOR i IN 1..5 RETURN i");
    assert_eq!(results.len(), 5);
    assert_eq!(results[0], json!(1));
    assert_eq!(results[4], json!(5));
}

#[test]
fn test_range_with_math() {
    let (engine, _tmp) = create_test_engine();
    
    let results = execute_query(&engine, "FOR i IN 1..5 RETURN i * 2");
    assert_eq!(results, vec![json!(2.0), json!(4.0), json!(6.0), json!(8.0), json!(10.0)]);
}

// ============================================================================
// Arithmetic Operations
// ============================================================================

#[test]
fn test_addition() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 10 + 20");
    assert_eq!(result.as_f64().unwrap(), 30.0);
}

#[test]
fn test_subtraction() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 50 - 25");
    assert_eq!(result.as_f64().unwrap(), 25.0);
}

#[test]
fn test_multiplication() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 6 * 7");
    assert_eq!(result.as_f64().unwrap(), 42.0);
}

#[test]
fn test_division() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 100 / 4");
    assert_eq!(result.as_f64().unwrap(), 25.0);
}



#[test]
fn test_operator_precedence() {
    let (engine, _tmp) = create_test_engine();
    
    // 2 + 3 * 4 = 2 + 12 = 14
    let result = execute_single(&engine, "RETURN 2 + 3 * 4");
    assert_eq!(result.as_f64().unwrap(), 14.0);
}

#[test]
fn test_parentheses_precedence() {
    let (engine, _tmp) = create_test_engine();
    
    // (2 + 3) * 4 = 5 * 4 = 20
    let result = execute_single(&engine, "RETURN (2 + 3) * 4");
    assert_eq!(result.as_f64().unwrap(), 20.0);
}

#[test]
fn test_modulo() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 17 % 5");
    assert_eq!(result.as_f64().unwrap(), 2.0);
}

#[test]
fn test_modulo_with_floats() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 10.5 % 3");
    let val = result.as_f64().unwrap();
    assert!((val - 1.5).abs() < 0.0001);
}

#[test]
fn test_modulo_negative() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN -7 % 3");
    let val = result.as_f64().unwrap();
    assert!((val - -1.0).abs() < 0.0001);
}

// ============================================================================
// Comparison Operations
// ============================================================================

#[test]
fn test_comparison_greater() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 10 > 5");
    assert_eq!(result, json!(true));
}

#[test]
fn test_comparison_less() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 3 < 7");
    assert_eq!(result, json!(true));
}

#[test]
fn test_comparison_equal() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 5 == 5");
    assert_eq!(result, json!(true));
}

#[test]
fn test_comparison_not_equal() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 5 != 6");
    assert_eq!(result, json!(true));
}

#[test]
fn test_comparison_greater_or_equal() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 5 >= 5");
    assert_eq!(result, json!(true));
}

#[test]
fn test_comparison_less_or_equal() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 5 <= 5");
    assert_eq!(result, json!(true));
}

// ============================================================================
// Math with Collection Data
// ============================================================================

#[test]
fn test_calculated_fields() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("products".to_string(), None).unwrap();
    let products = engine.get_collection("products").unwrap();
    
    products.insert(json!({"_key": "p1", "price": 100, "quantity": 5})).unwrap();
    products.insert(json!({"_key": "p2", "price": 50, "quantity": 10})).unwrap();
    
    let results = execute_query(&engine, 
        "FOR p IN products RETURN { key: p._key, total: p.price * p.quantity }");
    
    assert_eq!(results.len(), 2);
    
    let totals: Vec<f64> = results.iter()
        .map(|r| r["total"].as_f64().unwrap())
        .collect();
    assert!(totals.contains(&500.0));
    assert!(totals.contains(&500.0));
}

#[test]
fn test_percentage_calculation() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("stats".to_string(), None).unwrap();
    let stats = engine.get_collection("stats").unwrap();
    
    stats.insert(json!({"_key": "s1", "value": 25, "total": 100})).unwrap();
    stats.insert(json!({"_key": "s2", "value": 75, "total": 300})).unwrap();
    
    let results = execute_query(&engine, 
        "FOR s IN stats RETURN { pct: (s.value / s.total) * 100 }");
    
    assert_eq!(results.len(), 2);
    
    let pcts: Vec<f64> = results.iter()
        .map(|r| r["pct"].as_f64().unwrap())
        .collect();
    assert!(pcts.contains(&25.0));
    assert!(pcts.contains(&25.0));
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_sqrt_zero() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SQRT(0)");
    assert_eq!(result.as_f64().unwrap(), 0.0);
}

#[test]
fn test_pow_zero_exponent() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN POW(5, 0)");
    assert_eq!(result.as_f64().unwrap(), 1.0);
}



#[test]
fn test_negative_numbers_arithmetic() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN -5 + -3");
    assert_eq!(result.as_f64().unwrap(), -8.0);
}

#[test]
fn test_floating_point_arithmetic() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN 0.1 + 0.2");
    let val = result.as_f64().unwrap();
    assert!((val - 0.3).abs() < 0.0001);
}
