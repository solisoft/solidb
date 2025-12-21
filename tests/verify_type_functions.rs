//! Verify type checking functions in SDBQL
//!
//! Run with: cargo test --test verify_type_functions

use serde_json::json;
use solidb::{parse, QueryExecutor, StorageEngine};
use tempfile::TempDir;

fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

// ==================== IS_ARRAY ====================

#[test]
fn test_is_array_true() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_ARRAY([1, 2, 3])").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(true));
}

#[test]
fn test_is_array_false() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN IS_ARRAY("hello")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(false));
}

// ==================== IS_BOOLEAN ====================

#[test]
fn test_is_boolean_true() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_BOOLEAN(true)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(true));
}

#[test]
fn test_is_boolean_false() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_BOOLEAN(123)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(false));
}

// ==================== IS_NUMBER ====================

#[test]
fn test_is_number_int() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_NUMBER(42)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(true));
}

#[test]
fn test_is_number_float() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_NUMBER(3.14)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(true));
}

#[test]
fn test_is_number_false() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN IS_NUMBER("42")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(false));
}

// ==================== IS_INTEGER ====================

#[test]
fn test_is_integer_true() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_INTEGER(42)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(true));
}

#[test]
fn test_is_integer_float_false() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_INTEGER(3.14)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(false));
}

// ==================== IS_STRING ====================

#[test]
fn test_is_string_true() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN IS_STRING("hello")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(true));
}

#[test]
fn test_is_string_false() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_STRING(123)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(false));
}

// ==================== IS_OBJECT ====================

#[test]
fn test_is_object_true() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_OBJECT({a: 1, b: 2})").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(true));
}

#[test]
fn test_is_object_false() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_OBJECT([1, 2])").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(false));
}

// ==================== IS_NULL ====================

#[test]
fn test_is_null_true() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN IS_NULL(null)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(true));
}

#[test]
fn test_is_null_false() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN IS_NULL("")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(false));
}

// ==================== IS_DATETIME ====================

#[test]
fn test_is_datetime_iso_string() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN IS_DATETIME("2024-01-15T10:30:00Z")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(true));
}

#[test]
fn test_is_datetime_invalid() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN IS_DATETIME("not a date")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!(false));
}

// ==================== TYPENAME ====================

#[test]
fn test_typename_string() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN TYPENAME("hello")"#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("string"));
}

#[test]
fn test_typename_array() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN TYPENAME([1, 2, 3])").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("array"));
}

#[test]
fn test_typename_object() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN TYPENAME({a: 1})").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("object"));
}

#[test]
fn test_typename_bool() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN TYPENAME(true)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("bool"));
}

#[test]
fn test_typename_null() {
    let (storage, _dir) = create_test_storage();

    let query = parse("RETURN TYPENAME(null)").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("null"));
}

// ==================== Combined usage ====================

#[test]
fn test_type_check_with_ternary() {
    let (storage, _dir) = create_test_storage();

    let query = parse(r#"RETURN IS_STRING("test") ? "is string" : "not string""#).unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results[0], json!("is string"));
}
