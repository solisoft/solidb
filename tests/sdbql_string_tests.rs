//! SDBQL String Function Tests
//!
//! Tests for SDBQL string functions including:
//! - UPPER, LOWER, TRIM, LTRIM, RTRIM
//! - CONCAT, CONCAT_SEPARATOR
//! - SUBSTRING, LEFT, RIGHT
//! - CONTAINS, LIKE, REGEX
//! - SPLIT, REPLACE
//! - LENGTH on strings

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
// Case Conversion Tests
// ============================================================================

#[test]
fn test_upper() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN UPPER('hello world')");
    assert_eq!(result, json!("HELLO WORLD"));
}

#[test]
fn test_lower() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN LOWER('HELLO WORLD')");
    assert_eq!(result, json!("hello world"));
}

#[test]
fn test_upper_mixed_case() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN UPPER('HeLLo WoRLd')");
    assert_eq!(result, json!("HELLO WORLD"));
}

#[test]
fn test_lower_mixed_case() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN LOWER('HeLLo WoRLd')");
    assert_eq!(result, json!("hello world"));
}

// ============================================================================
// Trim Tests
// ============================================================================

#[test]
fn test_trim() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN TRIM('  hello  ')");
    assert_eq!(result, json!("hello"));
}

#[test]
fn test_ltrim() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN LTRIM('   hello')");
    assert_eq!(result, json!("hello"));
}

#[test]
fn test_rtrim() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN RTRIM('hello   ')");
    assert_eq!(result, json!("hello"));
}

#[test]
fn test_trim_no_spaces() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN TRIM('hello')");
    assert_eq!(result, json!("hello"));
}

// ============================================================================
// Concatenation Tests
// ============================================================================

#[test]
fn test_concat_two_strings() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN CONCAT('Hello', ' World')");
    assert_eq!(result, json!("Hello World"));
}

#[test]
fn test_concat_multiple_strings() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN CONCAT('a', 'b', 'c', 'd')");
    assert_eq!(result, json!("abcd"));
}



// ============================================================================
// Substring Tests
// ============================================================================

#[test]
fn test_substring_from_start() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SUBSTRING('Hello World', 0, 5)");
    assert_eq!(result, json!("Hello"));
}

#[test]
fn test_substring_from_middle() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SUBSTRING('Hello World', 6, 5)");
    assert_eq!(result, json!("World"));
}



// ============================================================================
// Contains and Search Tests
// ============================================================================

#[test]
fn test_contains_true() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN CONTAINS('Hello World', 'World')");
    assert_eq!(result, json!(true));
}

#[test]
fn test_contains_false() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN CONTAINS('Hello World', 'Foo')");
    assert_eq!(result, json!(false));
}

#[test]
fn test_contains_case_sensitive() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN CONTAINS('Hello World', 'world')");
    assert_eq!(result, json!(false));
}



// ============================================================================
// Split Tests
// ============================================================================

#[test]
fn test_split_by_comma() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SPLIT('a,b,c', ',')");
    assert_eq!(result, json!(["a", "b", "c"]));
}

#[test]
fn test_split_by_space() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SPLIT('Hello World', ' ')");
    assert_eq!(result, json!(["Hello", "World"]));
}

#[test]
fn test_split_no_delimiter() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SPLIT('abc', ',')");
    assert_eq!(result, json!(["abc"]));
}

// ============================================================================
// Replace Tests
// ============================================================================

#[test]
fn test_replace() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SUBSTITUTE('Hello World', 'World', 'Rust')");
    assert_eq!(result, json!("Hello Rust"));
}

#[test]
fn test_replace_all_occurrences() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN SUBSTITUTE('foo bar foo', 'foo', 'baz')");
    assert_eq!(result, json!("baz bar baz"));
}

// ============================================================================
// Length Tests
// ============================================================================

#[test]
fn test_length_string() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN LENGTH('Hello')");
    assert_eq!(result, json!(5));
}

#[test]
fn test_length_empty_string() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN LENGTH('')");
    assert_eq!(result, json!(0));
}





// ============================================================================
// Chained String Operations
// ============================================================================

#[test]
fn test_chained_upper_trim() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN UPPER(TRIM('  hello  '))");
    assert_eq!(result, json!("HELLO"));
}

#[test]
fn test_chained_concat_lower() {
    let (engine, _tmp) = create_test_engine();
    
    let result = execute_single(&engine, "RETURN LOWER(CONCAT('HELLO', ' ', 'WORLD'))");
    assert_eq!(result, json!("hello world"));
}

// ============================================================================
// String Operations on Collection Data
// ============================================================================

#[test]
fn test_string_functions_on_collection_data() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    
    users.insert(json!({"_key": "u1", "name": "  alice  ", "email": "Alice@Example.com"})).unwrap();
    users.insert(json!({"_key": "u2", "name": "  bob  ", "email": "Bob@Example.com"})).unwrap();
    
    let results = execute_query(&engine, 
        "FOR u IN users RETURN { name: TRIM(u.name), email: LOWER(u.email) }");
    
    assert_eq!(results.len(), 2);
    
    let names: Vec<&str> = results.iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"alice"));
    assert!(names.contains(&"bob"));
    
    for r in &results {
        let email = r["email"].as_str().unwrap();
        assert!(email == email.to_lowercase());
    }
}

#[test]
fn test_filter_with_contains() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("messages".to_string(), None).unwrap();
    let messages = engine.get_collection("messages").unwrap();
    
    messages.insert(json!({"_key": "m1", "text": "Hello World"})).unwrap();
    messages.insert(json!({"_key": "m2", "text": "Goodbye World"})).unwrap();
    messages.insert(json!({"_key": "m3", "text": "Hello Universe"})).unwrap();
    
    let results = execute_query(&engine, 
        "FOR m IN messages FILTER CONTAINS(m.text, 'Hello') RETURN m._key");
    
    assert_eq!(results.len(), 2);
    assert!(results.contains(&json!("m1")));
    assert!(results.contains(&json!("m3")));
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_empty_string_operations() {
    let (engine, _tmp) = create_test_engine();
    
    assert_eq!(execute_single(&engine, "RETURN UPPER('')"), json!(""));
    assert_eq!(execute_single(&engine, "RETURN LOWER('')"), json!(""));
    assert_eq!(execute_single(&engine, "RETURN TRIM('')"), json!(""));
}


