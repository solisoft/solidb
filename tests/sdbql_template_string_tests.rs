//! Tests for SDBQL Template String (String Interpolation) feature
//! Syntax: $"Hello ${expression}!" or $'Hello ${expression}!'

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

// =============================================================================
// Basic Template String Tests
// =============================================================================

#[test]
fn test_template_string_basic() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN $"Hello World""#);
    assert_eq!(result, vec![json!("Hello World")]);
}

#[test]
fn test_template_string_empty() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN $"""#);
    assert_eq!(result, vec![json!("")]);
}

#[test]
fn test_template_string_single_quotes() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN $'Hello World'"#);
    assert_eq!(result, vec![json!("Hello World")]);
}

// =============================================================================
// Expression Interpolation Tests
// =============================================================================

#[test]
fn test_template_string_simple_variable() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_var".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_var").expect("Collection not found");
    coll.insert(json!({"_key": "1", "name": "Alice"}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_var RETURN $"Hello ${doc.name}!""#);
    assert_eq!(result, vec![json!("Hello Alice!")]);
}

#[test]
fn test_template_string_multiple_expressions() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_multi".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_multi").expect("Collection not found");
    coll.insert(json!({"_key": "1", "first": "John", "last": "Doe"}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_multi RETURN $"${doc.first} ${doc.last}""#);
    assert_eq!(result, vec![json!("John Doe")]);
}

#[test]
fn test_template_string_with_arithmetic() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_arith".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_arith").expect("Collection not found");
    coll.insert(json!({"_key": "1", "price": 10, "qty": 5}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_arith RETURN $"Total: ${doc.price * doc.qty}""#);
    assert_eq!(result, vec![json!("Total: 50")]);
}

#[test]
fn test_template_string_with_function_call() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_func".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_func").expect("Collection not found");
    coll.insert(json!({"_key": "1", "name": "alice"}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_func RETURN $"Name: ${UPPER(doc.name)}""#);
    assert_eq!(result, vec![json!("Name: ALICE")]);
}

#[test]
fn test_template_string_nested_field_access() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_nested".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_nested").expect("Collection not found");
    coll.insert(json!({"_key": "1", "address": {"city": "Boston", "zip": "02101"}}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_nested RETURN $"City: ${doc.address.city}""#);
    assert_eq!(result, vec![json!("City: Boston")]);
}

// =============================================================================
// Type Coercion Tests
// =============================================================================

#[test]
fn test_template_string_number_coercion() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_num".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_num").expect("Collection not found");
    coll.insert(json!({"_key": "1", "count": 42}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_num RETURN $"Count: ${doc.count}""#);
    assert_eq!(result, vec![json!("Count: 42")]);
}

#[test]
fn test_template_string_float_coercion() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_float".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_float").expect("Collection not found");
    coll.insert(json!({"_key": "1", "price": 19.99}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_float RETURN $"Price: $$${doc.price}""#);
    assert_eq!(result, vec![json!("Price: $19.99")]);
}

#[test]
fn test_template_string_boolean_coercion() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_bool".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_bool").expect("Collection not found");
    coll.insert(json!({"_key": "1", "active": true}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_bool RETURN $"Active: ${doc.active}""#);
    assert_eq!(result, vec![json!("Active: true")]);
}

#[test]
fn test_template_string_null_coercion() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_null".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_null").expect("Collection not found");
    coll.insert(json!({"_key": "1", "value": null}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_null RETURN $"Value: ${doc.value}""#);
    assert_eq!(result, vec![json!("Value: null")]);
}

#[test]
fn test_template_string_array_coercion() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_arr".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_arr").expect("Collection not found");
    coll.insert(json!({"_key": "1", "tags": ["a", "b", "c"]}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_arr RETURN $"Tags: ${doc.tags}""#);
    assert_eq!(result, vec![json!(r#"Tags: ["a","b","c"]"#)]);
}

#[test]
fn test_template_string_object_coercion() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_obj".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_obj").expect("Collection not found");
    coll.insert(json!({"_key": "1", "data": {"x": 1}}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_obj RETURN $"Data: ${doc.data}""#);
    assert_eq!(result, vec![json!(r#"Data: {"x":1}"#)]);
}

// =============================================================================
// Escape Sequences Tests
// =============================================================================

#[test]
fn test_template_string_escaped_dollar() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN $"Price: $$50""#);
    assert_eq!(result, vec![json!("Price: $50")]);
}

#[test]
fn test_template_string_backslash_dollar() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN $"Code: \${x}""#);
    assert_eq!(result, vec![json!("Code: ${x}")]);
}

#[test]
fn test_template_string_newline_escape() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN $"Line1\nLine2""#);
    assert_eq!(result, vec![json!("Line1\nLine2")]);
}

#[test]
fn test_template_string_tab_escape() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN $"Col1\tCol2""#);
    assert_eq!(result, vec![json!("Col1\tCol2")]);
}

// =============================================================================
// Optional Chaining and Null Coalescing Tests
// =============================================================================

#[test]
fn test_template_string_with_optional_chaining() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_opt".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_opt").expect("Collection not found");
    coll.insert(json!({"_key": "1", "user": null}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_opt RETURN $"City: ${doc.user?.address?.city}""#);
    assert_eq!(result, vec![json!("City: null")]);
}

#[test]
fn test_template_string_with_null_coalescing() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_coal".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_coal").expect("Collection not found");
    coll.insert(json!({"_key": "1", "nickname": null, "name": "Bob"}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_coal RETURN $"Hello ${doc.nickname ?? doc.name}!""#);
    assert_eq!(result, vec![json!("Hello Bob!")]);
}

// =============================================================================
// Complex Expression Tests
// =============================================================================

#[test]
fn test_template_string_ternary_expression() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_tern".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_tern").expect("Collection not found");
    coll.insert(json!({"_key": "1", "age": 25}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_tern RETURN $"Status: ${doc.age >= 18 ? 'Adult' : 'Minor'}""#);
    assert_eq!(result, vec![json!("Status: Adult")]);
}

#[test]
fn test_template_string_with_concat() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_concat".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_concat").expect("Collection not found");
    coll.insert(json!({"_key": "1", "a": "Hello", "b": "World"}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_concat RETURN $"${CONCAT(doc.a, ' ', doc.b)}!""#);
    assert_eq!(result, vec![json!("Hello World!")]);
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_template_string_only_expression() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_only_expr".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_only_expr").expect("Collection not found");
    coll.insert(json!({"_key": "1", "value": "test"}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_only_expr RETURN $"${doc.value}""#);
    assert_eq!(result, vec![json!("test")]);
}

#[test]
fn test_template_string_adjacent_expressions() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_adj".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_adj").expect("Collection not found");
    coll.insert(json!({"_key": "1", "a": "Hello", "b": "World"}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_adj RETURN $"${doc.a}${doc.b}""#);
    assert_eq!(result, vec![json!("HelloWorld")]);
}

#[test]
fn test_template_string_in_object() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_in_obj".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_in_obj").expect("Collection not found");
    coll.insert(json!({"_key": "1", "name": "Alice"}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_in_obj RETURN {greeting: $"Hello ${doc.name}!"}"#);
    assert_eq!(result, vec![json!({"greeting": "Hello Alice!"})]);
}

#[test]
fn test_template_string_in_array() {
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("tpl_in_arr".to_string(), None)
        .expect("Failed to create collection");
    let coll = engine.get_collection("tpl_in_arr").expect("Collection not found");
    coll.insert(json!({"_key": "1", "name": "Bob"}))
        .expect("Insert failed");

    let result = execute_query(&engine, r#"FOR doc IN tpl_in_arr RETURN [$"Hi ${doc.name}", $"Bye ${doc.name}"]"#);
    assert_eq!(result, vec![json!(["Hi Bob", "Bye Bob"])]);
}

#[test]
fn test_template_string_with_let() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"LET name = "World" RETURN $"Hello ${name}!""#);
    assert_eq!(result, vec![json!("Hello World!")]);
}

#[test]
fn test_template_string_comparison() {
    let (engine, _tmp) = create_test_engine();
    let result = execute_query(&engine, r#"RETURN $"Hello World" == "Hello World""#);
    assert_eq!(result, vec![json!(true)]);
}
