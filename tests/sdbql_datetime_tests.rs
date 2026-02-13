//! SDBQL Date/Time and Utility Function Tests
//!
//! Tests for SDBQL functions that are actually implemented:
//! - DATE_NOW, DATE_ISO8601, DATE_TIMESTAMP
//! - DATE_TRUNC, DATE_DAYS_IN_MONTH, DATE_DAYOFYEAR, DATE_ISOWEEK
//! - TIME_BUCKET
//! - UUIDV4, UUIDV7
//! - MD5, SHA256
//! - MERGE, FLATTEN, UNIQUE, REVERSE

mod common;
use common::{create_test_engine, execute_query};
use serde_json::json;

// ============================================================================
// DATE_NOW Tests
// ============================================================================

#[test]
fn test_date_now() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN DATE_NOW()");

    assert_eq!(results.len(), 1);
    // DATE_NOW returns timestamp in milliseconds
    let timestamp = results[0].as_i64().unwrap();
    assert!(timestamp > 0);
    // Should be recent (within last 10 years from 2020)
    assert!(timestamp > 1577836800000); // 2020-01-01
}

// ============================================================================
// DATE_ISO8601 Tests
// ============================================================================

#[test]
fn test_date_iso8601_from_timestamp() {
    let (engine, _tmp) = create_test_engine();

    // 1609459200000 = 2021-01-01T00:00:00Z
    let results = execute_query(&engine, "RETURN DATE_ISO8601(1609459200000)");

    assert_eq!(results.len(), 1);
    let iso_str = results[0].as_str().unwrap();
    assert!(iso_str.contains("2021"));
    assert!(iso_str.contains("01"));
}

// ============================================================================
// DATE_TIMESTAMP Tests
// ============================================================================

#[test]
fn test_date_timestamp_from_iso() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN DATE_TIMESTAMP('2021-01-01T00:00:00Z')");

    assert_eq!(results.len(), 1);
    // Should return timestamp in ms
    let ts = results[0].as_i64().unwrap();
    assert!(ts > 0);
}

// ============================================================================
// DATE_TRUNC Tests
// ============================================================================

#[test]
fn test_date_trunc_to_day() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN DATE_TRUNC('2023-06-15T14:30:45Z', 'day')");

    assert_eq!(results.len(), 1);
    let truncated = results[0].as_str().unwrap();
    assert!(truncated.contains("2023-06-15"));
    assert!(truncated.contains("00:00:00"));
}

#[test]
fn test_date_trunc_to_hour() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN DATE_TRUNC('2023-06-15T14:30:45Z', 'hour')");

    assert_eq!(results.len(), 1);
    let truncated = results[0].as_str().unwrap();
    assert!(truncated.contains("14:00:00"));
}

#[test]
fn test_date_trunc_to_month() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(
        &engine,
        "RETURN DATE_TRUNC('2023-06-15T14:30:45Z', 'month')",
    );

    assert_eq!(results.len(), 1);
    let truncated = results[0].as_str().unwrap();
    assert!(truncated.contains("2023-06-01"));
}

// ============================================================================
// DATE_DAYS_IN_MONTH Tests
// ============================================================================

#[test]
fn test_date_days_in_month_february() {
    let (engine, _tmp) = create_test_engine();

    // 2024 is a leap year
    let results = execute_query(&engine, "RETURN DATE_DAYS_IN_MONTH('2024-02-15T00:00:00Z')");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_i64().unwrap(), 29);
}

#[test]
fn test_date_days_in_month_january() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN DATE_DAYS_IN_MONTH('2023-01-15T00:00:00Z')");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_i64().unwrap(), 31);
}

// ============================================================================
// DATE_DAYOFYEAR Tests
// ============================================================================

#[test]
fn test_date_dayofyear() {
    let (engine, _tmp) = create_test_engine();

    // January 15th is day 15
    let results = execute_query(&engine, "RETURN DATE_DAYOFYEAR('2023-01-15T00:00:00Z')");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_i64().unwrap(), 15);
}

#[test]
fn test_date_dayofyear_later() {
    let (engine, _tmp) = create_test_engine();

    // March 1st (non-leap year) = Jan 31 + Feb 28 + 1 = day 60
    let results = execute_query(&engine, "RETURN DATE_DAYOFYEAR('2023-03-01T00:00:00Z')");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_i64().unwrap(), 60);
}

// ============================================================================
// DATE_ISOWEEK Tests
// ============================================================================

#[test]
fn test_date_isoweek() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN DATE_ISOWEEK('2023-06-15T00:00:00Z')");

    assert_eq!(results.len(), 1);
    // June 15, 2023 is in week 24
    let week = results[0].as_i64().unwrap();
    assert!(week > 0 && week <= 53);
}

// ============================================================================
// TIME_BUCKET Tests
// ============================================================================

#[test]
fn test_time_bucket_1s() {
    let (engine, _tmp) = create_test_engine();

    // 2500ms should bucket to 2000ms with 1s bucket
    let results = execute_query(&engine, "RETURN TIME_BUCKET(2500, '1s')");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_i64().unwrap(), 2000);
}

#[test]
fn test_time_bucket_1m() {
    let (engine, _tmp) = create_test_engine();

    // 90000ms (1.5 minutes) should bucket to 60000ms with 1m bucket
    let results = execute_query(&engine, "RETURN TIME_BUCKET(90000, '1m')");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_i64().unwrap(), 60000);
}

#[test]
fn test_time_bucket_1h() {
    let (engine, _tmp) = create_test_engine();

    // 5400000ms (1.5 hours) should bucket to 3600000ms with 1h bucket
    let results = execute_query(&engine, "RETURN TIME_BUCKET(5400000, '1h')");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_i64().unwrap(), 3600000);
}

// ============================================================================
// UUID Functions Tests
// ============================================================================

#[test]
fn test_uuidv4() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN UUIDV4()");

    assert_eq!(results.len(), 1);
    let uuid = results[0].as_str().unwrap();
    // UUID should be 36 characters (8-4-4-4-12 with hyphens)
    assert_eq!(uuid.len(), 36);
    assert!(uuid.contains('-'));
}

#[test]
fn test_uuidv7() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN UUIDV7()");

    assert_eq!(results.len(), 1);
    let uuid = results[0].as_str().unwrap();
    assert_eq!(uuid.len(), 36);
}

#[test]
fn test_uuidv4_uniqueness() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("data".to_string(), None).unwrap();
    let data = engine.get_collection("data").unwrap();

    // Insert dummy data so we can use FOR loop
    for i in 0..10 {
        data.insert(json!({"_key": format!("d{}", i)})).unwrap();
    }

    let results = execute_query(&engine, "FOR d IN data RETURN UUIDV4()");

    assert_eq!(results.len(), 10);

    // All UUIDs should be unique
    let mut seen = std::collections::HashSet::new();
    for r in &results {
        let uuid = r.as_str().unwrap();
        assert!(seen.insert(uuid.to_string()), "Duplicate UUID found");
    }
}

// ============================================================================
// HASH Functions Tests
// ============================================================================

#[test]
fn test_md5() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN MD5('hello')");

    assert_eq!(results.len(), 1);
    let hash = results[0].as_str().unwrap();
    // MD5 produces 32 hex characters
    assert_eq!(hash.len(), 32);
    // Known MD5 of "hello"
    assert_eq!(hash, "5d41402abc4b2a76b9719d911017c592");
}

#[test]
fn test_sha256() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN SHA256('hello')");

    assert_eq!(results.len(), 1);
    let hash = results[0].as_str().unwrap();
    // SHA256 produces 64 hex characters
    assert_eq!(hash.len(), 64);
    // Known SHA256 of "hello"
    assert_eq!(
        hash,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
}

// ============================================================================
// JSON Functions Tests
// ============================================================================

#[test]
fn test_json_stringify() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(
        &engine,
        "RETURN JSON_STRINGIFY({ name: 'test', value: 42 })",
    );

    assert_eq!(results.len(), 1);
    let json_str = results[0].as_str().unwrap();
    assert!(json_str.contains("name"));
    assert!(json_str.contains("test"));
    assert!(json_str.contains("42"));
}

#[test]
fn test_json_parse() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, r#"RETURN JSON_PARSE('{"key":"value"}')"#);

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["key"], "value");
}

// ============================================================================
// MERGE Function Tests
// ============================================================================

#[test]
fn test_merge_objects() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN MERGE({ a: 1 }, { b: 2 })");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["a"], 1);
    assert_eq!(results[0]["b"], 2);
}

#[test]
fn test_merge_override() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN MERGE({ a: 1, b: 2 }, { b: 3 })");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["a"], 1);
    assert_eq!(results[0]["b"], 3); // b should be overridden
}

// ============================================================================
// UNIQUE Function Tests
// ============================================================================

#[test]
fn test_unique() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN UNIQUE([1, 2, 2, 3, 3, 3, 4])");

    assert_eq!(results.len(), 1);
    let unique = results[0].as_array().unwrap();
    assert_eq!(unique.len(), 4);
}

// ============================================================================
// FLATTEN Function Tests
// ============================================================================

#[test]
fn test_flatten() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN FLATTEN([[1, 2], [3, 4], [5]])");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!([1, 2, 3, 4, 5]));
}

// ============================================================================
// REVERSE Function Tests (Array only)
// ============================================================================

#[test]
fn test_reverse_array() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(&engine, "RETURN REVERSE([1, 2, 3, 4, 5])");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0], json!([5, 4, 3, 2, 1]));
}

// ============================================================================
// Collection Count Function Tests
// ============================================================================

#[test]
fn test_count_collection() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();

    for i in 0..5 {
        items.insert(json!({"_key": format!("i{}", i)})).unwrap();
    }

    let results = execute_query(&engine, "RETURN LENGTH((FOR i IN items RETURN i))");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].as_i64().unwrap(), 5);
}
