//! SDBQL Fuzzy Search Tests
//!
//! Tests for fuzzy string matching features:
//! - FUZZY_MATCH(text, pattern, max_distance?) - boolean fuzzy match
//! - SIMILARITY(a, b) - trigram similarity score (0.0-1.0)
//! - ~= operator - fuzzy comparison in FILTER

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
// FUZZY_MATCH Function Tests
// ============================================================================

#[test]
fn test_fuzzy_match_exact() {
    let (engine, _tmp) = create_test_engine();

    // Exact match should return true with distance 0
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('hello', 'hello', 0)");
    assert_eq!(result, json!(true));
}

#[test]
fn test_fuzzy_match_one_edit() {
    let (engine, _tmp) = create_test_engine();

    // One character difference should match with distance 1
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('hello', 'hallo', 1)");
    assert_eq!(result, json!(true));

    // One character missing should match with distance 1
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('hello', 'helo', 1)");
    assert_eq!(result, json!(true));

    // One character extra should match with distance 1
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('hello', 'helloo', 1)");
    assert_eq!(result, json!(true));
}

#[test]
fn test_fuzzy_match_two_edits() {
    let (engine, _tmp) = create_test_engine();

    // Two character differences should match with distance 2
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('hello', 'halla', 2)");
    assert_eq!(result, json!(true));

    // Should fail with distance 1
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('hello', 'halla', 1)");
    assert_eq!(result, json!(false));
}

#[test]
fn test_fuzzy_match_default_distance() {
    let (engine, _tmp) = create_test_engine();

    // Default distance is 2
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('jonathan', 'jonathen')");
    assert_eq!(result, json!(true));

    let result = execute_single(&engine, "RETURN FUZZY_MATCH('jonathan', 'jonatan')");
    assert_eq!(result, json!(true));
}

#[test]
fn test_fuzzy_match_no_match() {
    let (engine, _tmp) = create_test_engine();

    // Completely different strings shouldn't match
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('hello', 'world', 2)");
    assert_eq!(result, json!(false));

    let result = execute_single(&engine, "RETURN FUZZY_MATCH('abc', 'xyz', 2)");
    assert_eq!(result, json!(false));
}

#[test]
fn test_fuzzy_match_case_sensitive() {
    let (engine, _tmp) = create_test_engine();

    // Case differences count as edits
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('Hello', 'hello', 1)");
    assert_eq!(result, json!(true));

    let result = execute_single(&engine, "RETURN FUZZY_MATCH('HELLO', 'hello', 5)");
    assert_eq!(result, json!(true));
}

#[test]
fn test_fuzzy_match_empty_strings() {
    let (engine, _tmp) = create_test_engine();

    // Empty strings
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('', '', 0)");
    assert_eq!(result, json!(true));

    let result = execute_single(&engine, "RETURN FUZZY_MATCH('abc', '', 3)");
    assert_eq!(result, json!(true));

    let result = execute_single(&engine, "RETURN FUZZY_MATCH('abc', '', 2)");
    assert_eq!(result, json!(false));
}

// ============================================================================
// SIMILARITY Function Tests
// ============================================================================

#[test]
fn test_similarity_identical() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN SIMILARITY('hello', 'hello')");
    assert_eq!(result, json!(1.0));
}

#[test]
fn test_similarity_completely_different() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN SIMILARITY('abc', 'xyz')");
    assert_eq!(result, json!(0.0));
}

#[test]
fn test_similarity_partial_match() {
    let (engine, _tmp) = create_test_engine();

    // Similar strings should have similarity > 0
    let result = execute_single(&engine, "RETURN SIMILARITY('hello', 'hallo')");
    let sim = result.as_f64().unwrap();
    assert!(
        sim > 0.0 && sim < 1.0,
        "Expected partial similarity, got {}",
        sim
    );
}

#[test]
fn test_similarity_with_filter() {
    let (engine, _tmp) = create_test_engine();

    // Create test collection
    engine
        .create_collection("products".to_string(), None)
        .expect("Failed to create collection");

    let coll = engine.get_collection("products").unwrap();
    coll.insert(json!({"_key": "1", "name": "iPhone"})).unwrap();
    coll.insert(json!({"_key": "2", "name": "iPad"})).unwrap();
    coll.insert(json!({"_key": "3", "name": "Android"}))
        .unwrap();

    // Use SIMILARITY in a query with filter - use lower threshold for trigram matching
    let results = execute_query(
        &engine,
        r#"
        FOR doc IN products
            LET sim = SIMILARITY(doc.name, "iPhone")
            FILTER sim > 0.5
            RETURN {name: doc.name, similarity: sim}
    "#,
    );
    // iPhone should match exactly with similarity 1.0
    assert!(!results.is_empty(), "Expected match for 'iPhone'");
}

#[test]
fn test_similarity_short_strings() {
    let (engine, _tmp) = create_test_engine();

    // Short strings (less than n-gram size)
    let result = execute_single(&engine, "RETURN SIMILARITY('ab', 'ab')");
    assert_eq!(result, json!(1.0));

    let result = execute_single(&engine, "RETURN SIMILARITY('a', 'b')");
    assert_eq!(result, json!(0.0));
}

// ============================================================================
// Fuzzy Operator (~=) Tests
// ============================================================================

#[test]
fn test_fuzzy_operator_exact_match() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(&engine, "RETURN 'hello' ~= 'hello'");
    assert_eq!(result, json!(true));
}

#[test]
fn test_fuzzy_operator_close_match() {
    let (engine, _tmp) = create_test_engine();

    // Within default distance of 2
    let result = execute_single(&engine, "RETURN 'hello' ~= 'hallo'");
    assert_eq!(result, json!(true));

    let result = execute_single(&engine, "RETURN 'hello' ~= 'helo'");
    assert_eq!(result, json!(true));

    let result = execute_single(&engine, "RETURN 'jonathan' ~= 'jonathen'");
    assert_eq!(result, json!(true));
}

#[test]
fn test_fuzzy_operator_no_match() {
    let (engine, _tmp) = create_test_engine();

    // Beyond distance of 2
    let result = execute_single(&engine, "RETURN 'hello' ~= 'world'");
    assert_eq!(result, json!(false));

    let result = execute_single(&engine, "RETURN 'abc' ~= 'xyz'");
    assert_eq!(result, json!(false));
}

#[test]
fn test_fuzzy_operator_in_filter() {
    let (engine, _tmp) = create_test_engine();

    // Create test collection
    engine
        .create_collection("users".to_string(), None)
        .expect("Failed to create collection");

    let coll = engine.get_collection("users").unwrap();
    coll.insert(json!({"_key": "1", "name": "john"})).unwrap();
    coll.insert(json!({"_key": "2", "name": "jon"})).unwrap();
    coll.insert(json!({"_key": "3", "name": "jonathan"}))
        .unwrap();
    coll.insert(json!({"_key": "4", "name": "jane"})).unwrap();
    coll.insert(json!({"_key": "5", "name": "bob"})).unwrap();

    // Use ~= in FILTER - should match john, jon (within 2 edits of "john")
    let results = execute_query(
        &engine,
        r#"
        FOR doc IN users
            FILTER doc.name ~= "john"
            RETURN doc.name
    "#,
    );

    assert!(results.contains(&json!("john")), "Should match 'john'");
    assert!(results.contains(&json!("jon")), "Should match 'jon'");
    assert!(!results.contains(&json!("jane")), "Should not match 'jane'");
    assert!(!results.contains(&json!("bob")), "Should not match 'bob'");
}

#[test]
fn test_fuzzy_operator_with_variable() {
    let (engine, _tmp) = create_test_engine();

    let result = execute_single(
        &engine,
        r#"
        LET search = "hello"
        RETURN search ~= "hallo"
    "#,
    );
    assert_eq!(result, json!(true));
}

// ============================================================================
// Combined Usage Tests
// ============================================================================

#[test]
fn test_combined_fuzzy_and_similarity() {
    let (engine, _tmp) = create_test_engine();

    // Create test collection
    engine
        .create_collection("items".to_string(), None)
        .expect("Failed to create collection");

    let coll = engine.get_collection("items").unwrap();
    coll.insert(json!({"_key": "1", "title": "hello world"}))
        .unwrap();
    coll.insert(json!({"_key": "2", "title": "hallo world"}))
        .unwrap();
    coll.insert(json!({"_key": "3", "title": "goodbye world"}))
        .unwrap();

    // Use both FUZZY_MATCH and SIMILARITY together
    let results = execute_query(
        &engine,
        r#"
        FOR doc IN items
            LET sim = SIMILARITY(doc.title, "hello world")
            FILTER FUZZY_MATCH(doc.title, "hello world", 2) OR sim > 0.5
            SORT sim DESC
            RETURN {title: doc.title, score: sim}
    "#,
    );

    assert!(results.len() >= 2, "Expected at least 2 matches");
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_fuzzy_match_unicode() {
    let (engine, _tmp) = create_test_engine();

    // Unicode characters
    let result = execute_single(&engine, "RETURN FUZZY_MATCH('café', 'cafe', 1)");
    assert_eq!(result, json!(true));
}

#[test]
fn test_similarity_unicode() {
    let (engine, _tmp) = create_test_engine();

    // Unicode characters are handled by n-grams, but may have lower similarity
    // due to different character representations
    let result = execute_single(&engine, "RETURN SIMILARITY('münchen', 'munchen')");
    let sim = result.as_f64().unwrap();
    // Just verify it returns a valid similarity score
    assert!(
        sim >= 0.0 && sim <= 1.0,
        "Expected valid similarity score, got {}",
        sim
    );
}

#[test]
fn test_fuzzy_operator_numbers_as_strings() {
    let (engine, _tmp) = create_test_engine();

    // Numbers treated as strings
    let result = execute_single(&engine, "RETURN '12345' ~= '12346'");
    assert_eq!(result, json!(true));
}
