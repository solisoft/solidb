//! Document Edge Case Tests
//!
//! Tests for document operations edge cases including:
//! - Special characters in keys
//! - Unicode content
//! - Large documents
//! - Nested structures
//! - Empty values
//! - Revision handling

use serde_json::json;
use solidb::storage::StorageEngine;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine.create_collection("docs".to_string(), None).unwrap();
    (engine, tmp_dir)
}

// ============================================================================
// Special Key Tests
// ============================================================================

#[test]
fn test_document_with_numeric_key() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({"_key": "12345", "data": "numeric key"}))
        .unwrap();

    let doc = docs.get("12345").unwrap();
    assert_eq!(doc.get("data").unwrap(), "numeric key");
}

#[test]
fn test_document_with_underscore_key() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({"_key": "my_special_key", "data": "test"}))
        .unwrap();

    let doc = docs.get("my_special_key").unwrap();
    assert_eq!(doc.get("data").unwrap(), "test");
}

#[test]
fn test_document_with_hyphen_key() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({"_key": "doc-with-hyphens", "data": "test"}))
        .unwrap();

    let doc = docs.get("doc-with-hyphens").unwrap();
    assert!(doc.key == "doc-with-hyphens");
}

// ============================================================================
// Unicode Content Tests
// ============================================================================

#[test]
fn test_document_with_unicode_content() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "unicode1",
        "japanese": "日本語テスト",
        "chinese": "中文测试",
        "korean": "한국어 테스트",
        "arabic": "اختبار عربي",
        "emoji": "🎉🚀💻🌍"
    }))
    .unwrap();

    let doc = docs.get("unicode1").unwrap();
    assert_eq!(doc.get("japanese").unwrap(), "日本語テスト");
    assert_eq!(doc.get("chinese").unwrap(), "中文测试");
    assert_eq!(doc.get("korean").unwrap(), "한국어 테스트");
    assert_eq!(doc.get("emoji").unwrap(), "🎉🚀💻🌍");
}

#[test]
fn test_document_with_mixed_unicode() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "mixed",
        "content": "Hello 世界 🌍 مرحبا"
    }))
    .unwrap();

    let doc = docs.get("mixed").unwrap();
    assert_eq!(doc.get("content").unwrap(), "Hello 世界 🌍 مرحبا");
}

// ============================================================================
// Large Document Tests
// ============================================================================

#[test]
fn test_document_with_large_string() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    // 100KB string
    let large_string = "a".repeat(100 * 1024);

    docs.insert(json!({
        "_key": "large",
        "content": large_string.clone()
    }))
    .unwrap();

    let doc = docs.get("large").unwrap();
    assert_eq!(
        doc.get("content").unwrap().as_str().unwrap().len(),
        100 * 1024
    );
}

#[test]
fn test_document_with_large_array() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    // Array with 1000 elements
    let large_array: Vec<i32> = (0..1000).collect();

    docs.insert(json!({
        "_key": "large_array",
        "items": large_array
    }))
    .unwrap();

    let doc = docs.get("large_array").unwrap();
    let items_value = doc.get("items").unwrap();
    let items = items_value.as_array().unwrap();
    assert_eq!(items.len(), 1000);
}

// ============================================================================
// Nested Structure Tests
// ============================================================================

#[test]
fn test_document_deeply_nested() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "nested",
        "level1": {
            "level2": {
                "level3": {
                    "level4": {
                        "level5": {
                            "value": "deep"
                        }
                    }
                }
            }
        }
    }))
    .unwrap();

    let doc = docs.get("nested").unwrap();
    let value = &doc.data["level1"]["level2"]["level3"]["level4"]["level5"]["value"];
    assert_eq!(value, "deep");
}

#[test]
fn test_document_mixed_nested() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "mixed_nested",
        "users": [
            {
                "name": "Alice",
                "addresses": [
                    {"city": "Paris", "zip": "75001"},
                    {"city": "Lyon", "zip": "69001"}
                ]
            },
            {
                "name": "Bob",
                "addresses": [
                    {"city": "London", "zip": "SW1A"}
                ]
            }
        ]
    }))
    .unwrap();

    let doc = docs.get("mixed_nested").unwrap();
    let users = doc.data["users"].as_array().unwrap();
    assert_eq!(users.len(), 2);
    assert_eq!(users[0]["addresses"][0]["city"], "Paris");
}

// ============================================================================
// Empty and Null Value Tests
// ============================================================================

#[test]
fn test_document_with_empty_string() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "empty_string",
        "name": "",
        "description": ""
    }))
    .unwrap();

    let doc = docs.get("empty_string").unwrap();
    assert_eq!(doc.get("name").unwrap(), "");
}

#[test]
fn test_document_with_null_values() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "with_null",
        "name": "Test",
        "optional": null
    }))
    .unwrap();

    let doc = docs.get("with_null").unwrap();
    assert!(doc.data["optional"].is_null());
}

#[test]
fn test_document_with_empty_array() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "empty_array",
        "tags": []
    }))
    .unwrap();

    let doc = docs.get("empty_array").unwrap();
    assert!(doc.data["tags"].as_array().unwrap().is_empty());
}

#[test]
fn test_document_with_empty_object() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "empty_obj",
        "metadata": {}
    }))
    .unwrap();

    let doc = docs.get("empty_obj").unwrap();
    assert!(doc.data["metadata"].as_object().unwrap().is_empty());
}

// ============================================================================
// Numeric Value Tests
// ============================================================================

#[test]
fn test_document_with_float_values() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "floats",
        "pi": 3.14159265359,
        "e": 2.71828182845,
        "tiny": 0.000001,
        "large": 999999999.999999
    }))
    .unwrap();

    let doc = docs.get("floats").unwrap();
    let pi = doc.get("pi").unwrap().as_f64().unwrap();
    assert!((pi - 3.14159265359).abs() < 0.0001);
}

#[test]
fn test_document_with_integer_edge_cases() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "integers",
        "zero": 0,
        "negative": -42,
        "large": 9007199254740992_i64  // Max safe integer in JSON
    }))
    .unwrap();

    let doc = docs.get("integers").unwrap();
    assert_eq!(doc.get("zero").unwrap(), 0);
    assert_eq!(doc.get("negative").unwrap(), -42);
}

// ============================================================================
// Boolean Value Tests
// ============================================================================

#[test]
fn test_document_with_boolean_values() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "bools",
        "active": true,
        "deleted": false
    }))
    .unwrap();

    let doc = docs.get("bools").unwrap();
    assert_eq!(doc.get("active").unwrap(), true);
    assert_eq!(doc.get("deleted").unwrap(), false);
}

// ============================================================================
// Update Operation Tests
// ============================================================================

#[test]
fn test_update_add_new_field() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({"_key": "doc1", "name": "Test"}))
        .unwrap();

    docs.update("doc1", json!({"age": 30})).unwrap();

    let doc = docs.get("doc1").unwrap();
    assert_eq!(doc.get("name").unwrap(), "Test");
    assert_eq!(doc.get("age").unwrap(), 30);
}

#[test]
fn test_update_modify_existing_field() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({"_key": "doc1", "count": 0})).unwrap();

    docs.update("doc1", json!({"count": 10})).unwrap();

    let doc = docs.get("doc1").unwrap();
    assert_eq!(doc.get("count").unwrap(), 10);
}

#[test]
fn test_update_nested_field() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    docs.insert(json!({
        "_key": "doc1",
        "user": {"name": "Alice", "age": 25}
    }))
    .unwrap();

    docs.update("doc1", json!({"user": {"name": "Alice", "age": 26}}))
        .unwrap();

    let doc = docs.get("doc1").unwrap();
    assert_eq!(doc.data["user"]["age"], 26);
}

// ============================================================================
// Scan Operation Tests
// ============================================================================

#[test]
fn test_scan_all_documents() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    for i in 0..10 {
        docs.insert(json!({"_key": format!("doc{}", i), "num": i}))
            .unwrap();
    }

    let all_docs = docs.scan(None);
    assert_eq!(all_docs.len(), 10);
}

#[test]
fn test_scan_with_limit() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    for i in 0..20 {
        docs.insert(json!({"_key": format!("doc{}", i), "num": i}))
            .unwrap();
    }

    let limited = docs.scan(Some(5));
    assert_eq!(limited.len(), 5);
}

// ============================================================================
// Count Operation Tests
// ============================================================================

#[test]
fn test_count_empty_collection() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    assert_eq!(docs.count(), 0);
}

#[test]
fn test_count_after_inserts() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    for i in 0..25 {
        docs.insert(json!({"_key": format!("d{}", i)})).unwrap();
    }

    assert_eq!(docs.count(), 25);
}

#[test]
fn test_count_after_delete() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    for i in 0..10 {
        docs.insert(json!({"_key": format!("d{}", i)})).unwrap();
    }

    docs.delete("d5").unwrap();
    docs.delete("d6").unwrap();

    assert_eq!(docs.count(), 8);
}

// ============================================================================
// Truncate Operation Tests
// ============================================================================

#[test]
fn test_truncate_collection() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    for i in 0..50 {
        docs.insert(json!({"_key": format!("d{}", i)})).unwrap();
    }

    assert_eq!(docs.count(), 50);

    docs.truncate().unwrap();

    assert_eq!(docs.count(), 0);
}

// ============================================================================
// Auto-generated Key Tests
// ============================================================================

#[test]
fn test_auto_generated_key() {
    let (engine, _tmp) = create_test_engine();
    let docs = engine.get_collection("docs").unwrap();

    // Insert without _key
    let inserted = docs.insert(json!({"name": "Auto Key"})).unwrap();

    // Should have a generated key (from Document struct)
    let key_str = &inserted.key;
    assert!(!key_str.is_empty());

    // Should be retrievable by the generated key
    let doc = docs.get(key_str).unwrap();
    assert_eq!(doc.get("name").unwrap(), "Auto Key");
}
