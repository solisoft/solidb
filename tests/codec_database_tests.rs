//! Codec and Database Operation Tests
//!
//! Tests for:
//! - Key encoding/decoding
//! - Binary-comparable key ordering
//! - Database CRUD operations
//! - Collection listing and stats

use solidb::storage::StorageEngine;
use solidb::storage::codec::{encode_key, decode_key};
use serde_json::json;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

// ============================================================================
// Codec Tests - Key Encoding
// ============================================================================

#[test]
fn test_encode_null() {
    let encoded = encode_key(&json!(null));
    assert!(!encoded.is_empty());
    assert_eq!(encoded[0], 0x01); // Null type marker
}

#[test]
fn test_encode_bool_false() {
    let encoded = encode_key(&json!(false));
    assert_eq!(encoded[0], 0x02); // Bool type marker
    assert_eq!(encoded[1], 0x00); // false = 0
}

#[test]
fn test_encode_bool_true() {
    let encoded = encode_key(&json!(true));
    assert_eq!(encoded[0], 0x02); // Bool type marker
    assert_eq!(encoded[1], 0x01); // true = 1
}

#[test]
fn test_encode_number_integer() {
    let encoded = encode_key(&json!(42));
    assert_eq!(encoded[0], 0x03); // Number type marker
    assert_eq!(encoded.len(), 9); // 1 type + 8 bytes f64
}

#[test]
fn test_encode_number_float() {
    let encoded = encode_key(&json!(3.14));
    assert_eq!(encoded[0], 0x03);
    assert_eq!(encoded.len(), 9);
}

#[test]
fn test_encode_number_negative() {
    let encoded = encode_key(&json!(-100));
    assert_eq!(encoded[0], 0x03);
    assert_eq!(encoded.len(), 9);
}

#[test]
fn test_encode_string() {
    let encoded = encode_key(&json!("hello"));
    assert_eq!(encoded[0], 0x04); // String type marker
    assert!(encoded.ends_with(&[0x00])); // Null terminator
}

#[test]
fn test_encode_empty_string() {
    let encoded = encode_key(&json!(""));
    assert_eq!(encoded[0], 0x04);
    assert_eq!(encoded.len(), 2); // type + null terminator
}

#[test]
fn test_encode_array() {
    let encoded = encode_key(&json!([1, 2, 3]));
    assert_eq!(encoded[0], 0x05); // Complex type marker
}

#[test]
fn test_encode_object() {
    let encoded = encode_key(&json!({"a": 1}));
    assert_eq!(encoded[0], 0x05); // Complex type marker
}

// ============================================================================
// Codec Tests - Key Decoding (Round-trip)
// ============================================================================

#[test]
fn test_decode_null() {
    let encoded = encode_key(&json!(null));
    let decoded = decode_key(&encoded).unwrap();
    assert_eq!(decoded, json!(null));
}

#[test]
fn test_decode_bool() {
    let encoded_true = encode_key(&json!(true));
    let encoded_false = encode_key(&json!(false));
    
    assert_eq!(decode_key(&encoded_true).unwrap(), json!(true));
    assert_eq!(decode_key(&encoded_false).unwrap(), json!(false));
}

#[test]
fn test_decode_number_integer() {
    let original = json!(42);
    let encoded = encode_key(&original);
    let decoded = decode_key(&encoded).unwrap();
    
    assert_eq!(decoded.as_f64(), Some(42.0));
}

#[test]
fn test_decode_number_float() {
    let original = json!(3.14);
    let encoded = encode_key(&original);
    let decoded = decode_key(&encoded).unwrap();
    
    let diff = (decoded.as_f64().unwrap() - 3.14).abs();
    assert!(diff < 0.0001);
}

#[test]
fn test_decode_number_negative() {
    let original = json!(-999);
    let encoded = encode_key(&original);
    let decoded = decode_key(&encoded).unwrap();
    
    assert_eq!(decoded.as_f64(), Some(-999.0));
}

#[test]
fn test_decode_string() {
    let original = json!("hello world");
    let encoded = encode_key(&original);
    let decoded = decode_key(&encoded).unwrap();
    
    assert_eq!(decoded, original);
}

#[test]
fn test_decode_empty_bytes() {
    let decoded = decode_key(&[]);
    assert!(decoded.is_none());
}

// ============================================================================
// Codec Tests - Sort Order Preservation
// ============================================================================

#[test]
fn test_sort_order_null_before_bool() {
    let null_key = encode_key(&json!(null));
    let bool_key = encode_key(&json!(false));
    
    assert!(null_key < bool_key, "Null should sort before Bool");
}

#[test]
fn test_sort_order_bool_before_number() {
    let bool_key = encode_key(&json!(true));
    let num_key = encode_key(&json!(0));
    
    assert!(bool_key < num_key, "Bool should sort before Number");
}

#[test]
fn test_sort_order_number_before_string() {
    let num_key = encode_key(&json!(999999));
    let str_key = encode_key(&json!("a"));
    
    assert!(num_key < str_key, "Number should sort before String");
}

#[test]
fn test_sort_order_numbers_ascending() {
    let key1 = encode_key(&json!(-100));
    let key2 = encode_key(&json!(0));
    let key3 = encode_key(&json!(100));
    
    assert!(key1 < key2, "Negative should sort before zero");
    assert!(key2 < key3, "Zero should sort before positive");
}

#[test]
fn test_sort_order_strings_lexicographic() {
    let key_a = encode_key(&json!("apple"));
    let key_b = encode_key(&json!("banana"));
    let key_c = encode_key(&json!("cherry"));
    
    assert!(key_a < key_b);
    assert!(key_b < key_c);
}

// ============================================================================
// Database Operations Tests
// ============================================================================

#[test]
fn test_database_create() {
    let (engine, _tmp) = create_test_engine();
    
    let result = engine.create_database("testdb".to_string());
    assert!(result.is_ok());
}

#[test]
fn test_database_get() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_database("mydb".to_string()).unwrap();
    
    let result = engine.get_database("mydb");
    assert!(result.is_ok());
}

#[test]
fn test_database_list() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_database("db1".to_string()).unwrap();
    engine.create_database("db2".to_string()).unwrap();
    engine.create_database("db3".to_string()).unwrap();
    
    let databases = engine.list_databases();
    assert!(databases.len() >= 3); // At least our 3 + maybe _system
}

#[test]
fn test_database_delete() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_database("to_delete".to_string()).unwrap();
    assert!(engine.get_database("to_delete").is_ok());
    
    engine.delete_database("to_delete").unwrap();
    assert!(engine.get_database("to_delete").is_err());
}

#[test]
fn test_database_with_collections() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_database("mydb".to_string()).unwrap();
    let db = engine.get_database("mydb").unwrap();
    
    // Create collections in the database
    db.create_collection("users".to_string(), None).unwrap();
    db.create_collection("products".to_string(), None).unwrap();
    
    let collections = db.list_collections();
    assert_eq!(collections.len(), 2);
}

// ============================================================================
// Collection Statistics Tests
// ============================================================================

#[test]
fn test_collection_count() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("items".to_string(), None).unwrap();
    let col = engine.get_collection("items").unwrap();
    
    assert_eq!(col.count(), 0);
    
    col.insert(json!({"name": "item1"})).unwrap();
    col.insert(json!({"name": "item2"})).unwrap();
    col.insert(json!({"name": "item3"})).unwrap();
    
    assert_eq!(col.count(), 3);
}

#[test]
fn test_collection_count_after_delete() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("items".to_string(), None).unwrap();
    let col = engine.get_collection("items").unwrap();
    
    let doc = col.insert(json!({"name": "item1"})).unwrap();
    col.insert(json!({"name": "item2"})).unwrap();
    
    assert_eq!(col.count(), 2);
    
    col.delete(&doc.key).unwrap();
    
    assert_eq!(col.count(), 1);
}

#[test]
fn test_collection_all_documents() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    col.insert(json!({"value": 1})).unwrap();
    col.insert(json!({"value": 2})).unwrap();
    col.insert(json!({"value": 3})).unwrap();
    
    let all = col.all();
    assert_eq!(all.len(), 3);
}

#[test]
fn test_collection_truncate() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("temp".to_string(), None).unwrap();
    let col = engine.get_collection("temp").unwrap();
    
    for i in 0..10 {
        col.insert(json!({"index": i})).unwrap();
    }
    
    assert_eq!(col.count(), 10);
    
    col.truncate().unwrap();
    
    assert_eq!(col.count(), 0);
}

// ============================================================================
// Batch Operations Tests
// ============================================================================

#[test]
fn test_multiple_inserts() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("batch".to_string(), None).unwrap();
    let col = engine.get_collection("batch").unwrap();
    
    // Insert multiple documents one by one
    for i in 0..5 {
        col.insert(json!({"name": format!("doc{}", i)})).unwrap();
    }
    
    assert_eq!(col.count(), 5);
}

#[test]
fn test_multiple_inserts_large() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("large_batch".to_string(), None).unwrap();
    let col = engine.get_collection("large_batch").unwrap();
    
    // Insert 100 documents
    for i in 0..100 {
        col.insert(json!({"index": i, "value": format!("item_{}", i)})).unwrap();
    }
    
    assert_eq!(col.count(), 100);
}

// ============================================================================
// Collection Type Tests
// ============================================================================

#[test]
fn test_edge_collection_creation() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("normal".to_string(), None).unwrap();
    engine.create_collection("edges".to_string(), Some("edge".to_string())).unwrap();
    
    let normal = engine.get_collection("normal").unwrap();
    let edges = engine.get_collection("edges").unwrap();
    
    // Normal collection doesn't require _from/_to
    assert!(normal.insert(json!({"data": "test"})).is_ok());
    
    // Edge collection requires _from and _to
    assert!(edges.insert(json!({"_from": "a/1", "_to": "b/2"})).is_ok());
}

#[test]
fn test_collection_field_access() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("my_collection".to_string(), None).unwrap();
    let col = engine.get_collection("my_collection").unwrap();
    
    // Access the name field directly
    assert_eq!(col.name, "my_collection");
}

// ============================================================================
// Storage Engine Persistence Tests
// ============================================================================

#[test]
fn test_data_survives_flush() {
    let tmp_dir = TempDir::new().unwrap();
    let path = tmp_dir.path().to_str().unwrap();
    
    {
        let engine = StorageEngine::new(path).unwrap();
        engine.create_collection("persistent".to_string(), None).unwrap();
        let col = engine.get_collection("persistent").unwrap();
        col.insert(json!({"_key": "test", "data": "value"})).unwrap();
        
        // Engine drops here, should flush
    }
    
    // Reopen
    {
        let engine = StorageEngine::new(path).unwrap();
        let col = engine.get_collection("persistent").unwrap();
        let doc = col.get("test").unwrap();
        assert_eq!(doc.get("data"), Some(json!("value")));
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_unicode_key() {
    let encoded = encode_key(&json!("日本語"));
    let decoded = decode_key(&encoded).unwrap();
    assert_eq!(decoded, json!("日本語"));
}

#[test]
fn test_special_characters_in_key() {
    let encoded = encode_key(&json!("hello\nworld\ttab"));
    let decoded = decode_key(&encoded).unwrap();
    assert_eq!(decoded, json!("hello\nworld\ttab"));
}

#[test]
fn test_very_large_number() {
    let large = 1e100;
    let encoded = encode_key(&json!(large));
    let decoded = decode_key(&encoded).unwrap();
    
    let diff = (decoded.as_f64().unwrap() - large).abs() / large;
    assert!(diff < 1e-10, "Large numbers should roundtrip");
}

#[test]
fn test_very_small_number() {
    let small = 1e-100;
    let encoded = encode_key(&json!(small));
    let decoded = decode_key(&encoded).unwrap();
    
    let diff = (decoded.as_f64().unwrap() - small).abs() / small;
    assert!(diff < 1e-10, "Small numbers should roundtrip");
}

#[test]
fn test_number_zero() {
    let encoded = encode_key(&json!(0));
    let decoded = decode_key(&encoded).unwrap();
    assert_eq!(decoded.as_f64(), Some(0.0));
}

#[test]
fn test_nested_object_encoding() {
    let obj = json!({"a": {"b": {"c": 1}}});
    let encoded = encode_key(&obj);
    let decoded = decode_key(&encoded).unwrap();
    
    // Complex types may not roundtrip exactly but shouldn't panic
    assert!(decoded.is_object() || decoded.is_string());
}
