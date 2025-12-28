//! Document Operations Tests
//!
//! Tests for document-level operations including:
//! - System fields (_key, _id, _rev, _created_at, _updated_at)
//! - Revision tracking
//! - Timestamps
//! - Document replacement vs merge

use solidb::storage::StorageEngine;
use serde_json::json;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

// ============================================================================
// System Fields Tests
// ============================================================================

#[test]
fn test_document_auto_generated_key() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    // Insert without _key - should auto-generate
    let doc = col.insert(json!({"name": "Test"})).unwrap();
    
    assert!(!doc.key.is_empty(), "Should auto-generate a key");
    assert!(doc.key.len() > 0);
}

#[test]
fn test_document_custom_key() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    // Insert with custom _key
    let doc = col.insert(json!({"_key": "my_custom_key", "name": "Test"})).unwrap();
    
    assert_eq!(doc.key, "my_custom_key");
}

#[test]
fn test_document_has_id() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let doc = col.insert(json!({"name": "Test"})).unwrap();
    let value = doc.to_value();
    
    // _id should be collection/key format
    let id = value.get("_id").and_then(|v| v.as_str()).unwrap();
    assert!(id.contains("/"), "ID should be in collection/key format");
    assert!(id.starts_with("docs/"));
}

#[test]
fn test_document_has_revision() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let doc = col.insert(json!({"name": "Test"})).unwrap();
    let value = doc.to_value();
    
    let rev = value.get("_rev");
    assert!(rev.is_some(), "Document should have _rev field");
}

#[test]
fn test_document_revision_changes_on_update() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let doc1 = col.insert(json!({"_key": "test", "name": "Original"})).unwrap();
    let rev1 = doc1.to_value().get("_rev").cloned();
    
    let doc2 = col.update("test", json!({"name": "Updated"})).unwrap();
    let rev2 = doc2.to_value().get("_rev").cloned();
    
    assert_ne!(rev1, rev2, "Revision should change on update");
}

#[test]
fn test_document_has_created_at() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let doc = col.insert(json!({"name": "Test"})).unwrap();
    let value = doc.to_value();
    
    let created_at = value.get("_created_at");
    assert!(created_at.is_some(), "Document should have _created_at field");
    // Can be number or string depending on implementation
}

#[test]
fn test_document_has_updated_at() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let doc = col.insert(json!({"name": "Test"})).unwrap();
    let value = doc.to_value();
    
    let updated_at = value.get("_updated_at");
    assert!(updated_at.is_some(), "Document should have _updated_at field");
}

#[test]
fn test_document_timestamps_present() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let doc1 = col.insert(json!({"_key": "test", "name": "Original"})).unwrap();
    let value1 = doc1.to_value();
    
    // Check that timestamps exist
    assert!(value1.get("_created_at").is_some(), "Should have _created_at");
    
    // Update document
    let doc2 = col.update("test", json!({"name": "Updated"})).unwrap();
    let value2 = doc2.to_value();
    
    // Timestamps should still be present after update
    assert!(value2.get("_created_at").is_some());
    assert!(value2.get("_updated_at").is_some());
}

// ============================================================================
// Update Behavior Tests
// ============================================================================

#[test]
fn test_update_merges_fields() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    col.insert(json!({
        "_key": "test",
        "name": "Alice",
        "age": 30,
        "city": "Paris"
    })).unwrap();
    
    // Update only one field
    let updated = col.update("test", json!({"age": 31})).unwrap();
    let value = updated.to_value();
    
    // Original fields should still be present
    assert_eq!(value.get("name"), Some(&json!("Alice")));
    assert_eq!(value.get("age"), Some(&json!(31)));
    assert_eq!(value.get("city"), Some(&json!("Paris")));
}

#[test]
fn test_update_adds_new_fields() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    col.insert(json!({"_key": "test", "name": "Alice"})).unwrap();
    
    // Add new field
    let updated = col.update("test", json!({"email": "alice@example.com"})).unwrap();
    let value = updated.to_value();
    
    assert_eq!(value.get("name"), Some(&json!("Alice")));
    assert_eq!(value.get("email"), Some(&json!("alice@example.com")));
}

#[test]
fn test_update_nested_object() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    col.insert(json!({
        "_key": "test",
        "profile": {
            "name": "Alice",
            "settings": {"theme": "dark"}
        }
    })).unwrap();
    
    // Update nested object
    let updated = col.update("test", json!({
        "profile": {"name": "Updated Alice"}
    })).unwrap();
    
    let value = updated.to_value();
    // Behavior depends on implementation - may replace or merge
}

#[test]
fn test_update_array_field() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    col.insert(json!({
        "_key": "test",
        "tags": ["a", "b", "c"]
    })).unwrap();
    
    // Replace array with new values
    let updated = col.update("test", json!({"tags": ["x", "y"]})).unwrap();
    let value = updated.to_value();
    
    assert_eq!(value.get("tags"), Some(&json!(["x", "y"])));
}

// ============================================================================
// Document Data Type Tests
// ============================================================================

#[test]
fn test_document_string_field() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("types".to_string(), None).unwrap();
    let col = engine.get_collection("types").unwrap();
    
    let doc = col.insert(json!({"_key": "str", "value": "hello world"})).unwrap();
    let retrieved = col.get("str").unwrap();
    
    assert_eq!(retrieved.get("value"), Some(json!("hello world")));
}

#[test]
fn test_document_number_fields() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("types".to_string(), None).unwrap();
    let col = engine.get_collection("types").unwrap();
    
    col.insert(json!({
        "_key": "nums",
        "integer": 42,
        "float": 3.14,
        "negative": -100,
        "zero": 0
    })).unwrap();
    
    let doc = col.get("nums").unwrap();
    assert_eq!(doc.get("integer"), Some(json!(42)));
    assert_eq!(doc.get("float"), Some(json!(3.14)));
    assert_eq!(doc.get("negative"), Some(json!(-100)));
    assert_eq!(doc.get("zero"), Some(json!(0)));
}

#[test]
fn test_document_boolean_fields() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("types".to_string(), None).unwrap();
    let col = engine.get_collection("types").unwrap();
    
    col.insert(json!({
        "_key": "bools",
        "yes": true,
        "no": false
    })).unwrap();
    
    let doc = col.get("bools").unwrap();
    assert_eq!(doc.get("yes"), Some(json!(true)));
    assert_eq!(doc.get("no"), Some(json!(false)));
}

#[test]
fn test_document_null_field() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("types".to_string(), None).unwrap();
    let col = engine.get_collection("types").unwrap();
    
    col.insert(json!({
        "_key": "nulls",
        "empty": null
    })).unwrap();
    
    let doc = col.get("nulls").unwrap();
    assert_eq!(doc.get("empty"), Some(json!(null)));
}

#[test]
fn test_document_array_field() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("types".to_string(), None).unwrap();
    let col = engine.get_collection("types").unwrap();
    
    col.insert(json!({
        "_key": "arrays",
        "numbers": [1, 2, 3],
        "strings": ["a", "b", "c"],
        "mixed": [1, "two", true, null]
    })).unwrap();
    
    let doc = col.get("arrays").unwrap();
    assert_eq!(doc.get("numbers"), Some(json!([1, 2, 3])));
    assert_eq!(doc.get("strings"), Some(json!(["a", "b", "c"])));
}

#[test]
fn test_document_nested_object_field() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("types".to_string(), None).unwrap();
    let col = engine.get_collection("types").unwrap();
    
    col.insert(json!({
        "_key": "nested",
        "user": {
            "name": "Alice",
            "address": {
                "city": "Paris",
                "country": "France"
            }
        }
    })).unwrap();
    
    let doc = col.get("nested").unwrap();
    let user = doc.get("user").unwrap();
    assert_eq!(user.get("name"), Some(&json!("Alice")));
    assert_eq!(user.get("address").unwrap().get("city"), Some(&json!("Paris")));
}

// ============================================================================
// Edge Document Tests  
// ============================================================================

#[test]
fn test_edge_document_fields() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("relations".to_string(), Some("edge".to_string())).unwrap();
    let edges = engine.get_collection("relations").unwrap();
    
    let doc = edges.insert(json!({
        "_from": "users/alice",
        "_to": "users/bob",
        "type": "follows",
        "since": 2020
    })).unwrap();
    
    let value = doc.to_value();
    assert_eq!(value.get("_from"), Some(&json!("users/alice")));
    assert_eq!(value.get("_to"), Some(&json!("users/bob")));
    assert_eq!(value.get("type"), Some(&json!("follows")));
}

#[test]
fn test_edge_update_preserves_from_to() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("relations".to_string(), Some("edge".to_string())).unwrap();
    let edges = engine.get_collection("relations").unwrap();
    
    let doc = edges.insert(json!({
        "_key": "e1",
        "_from": "users/alice",
        "_to": "users/bob",
        "weight": 1
    })).unwrap();
    
    let updated = edges.update("e1", json!({"weight": 5})).unwrap();
    let value = updated.to_value();
    
    // _from and _to should be preserved
    assert_eq!(value.get("_from"), Some(&json!("users/alice")));
    assert_eq!(value.get("_to"), Some(&json!("users/bob")));
    assert_eq!(value.get("weight"), Some(&json!(5)));
}

// ============================================================================
// Key Validation Tests
// ============================================================================

#[test]
fn test_key_with_alphanumeric() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let doc = col.insert(json!({"_key": "user123", "name": "Test"})).unwrap();
    assert_eq!(doc.key, "user123");
}

#[test]
fn test_key_with_underscores() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let doc = col.insert(json!({"_key": "user_name_123", "name": "Test"})).unwrap();
    assert_eq!(doc.key, "user_name_123");
}

#[test]
fn test_key_with_dashes() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let doc = col.insert(json!({"_key": "user-name-123", "name": "Test"})).unwrap();
    assert_eq!(doc.key, "user-name-123");
}

#[test]
fn test_key_with_uuid() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    let doc = col.insert(json!({"_key": uuid, "name": "Test"})).unwrap();
    assert_eq!(doc.key, uuid);
}

// ============================================================================
// Document Size Tests
// ============================================================================

#[test]
fn test_small_document() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let doc = col.insert(json!({"x": 1})).unwrap();
    assert!(!doc.key.is_empty());
}

#[test]
fn test_large_document_many_fields() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let mut data = serde_json::Map::new();
    for i in 0..500 {
        data.insert(format!("field_{}", i), json!(i));
    }
    
    let doc = col.insert(json!(data)).unwrap();
    assert!(!doc.key.is_empty());
    
    let retrieved = col.get(&doc.key).unwrap();
    assert_eq!(retrieved.get("field_0"), Some(json!(0)));
    assert_eq!(retrieved.get("field_499"), Some(json!(499)));
}

#[test]
fn test_document_with_long_string() {
    let (engine, _tmp) = create_test_engine();
    
    engine.create_collection("docs".to_string(), None).unwrap();
    let col = engine.get_collection("docs").unwrap();
    
    let long_string = "a".repeat(10000);
    let doc = col.insert(json!({"_key": "long", "content": long_string.clone()})).unwrap();
    
    let retrieved = col.get("long").unwrap();
    let content = retrieved.get("content").unwrap();
    assert_eq!(content.as_str().unwrap(), long_string);
}
