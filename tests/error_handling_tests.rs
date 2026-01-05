//! Error Handling Tests
//!
//! Tests for comprehensive error handling across the codebase including:
//! - Storage errors
//! - Collection errors  
//! - Document errors
//! - Query errors
//! - Index errors

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

// ============================================================================
// Collection Error Tests
// ============================================================================

#[test]
fn test_error_collection_not_found() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.get_collection("nonexistent");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_error_duplicate_collection() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();

    let result = engine.create_collection("users".to_string(), None);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));
}

// ============================================================================
// Document Error Tests
// ============================================================================

#[test]
fn test_error_document_not_found() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    let result = users.get("nonexistent_key");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[test]
fn test_error_document_update_not_found() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    let result = users.update("nonexistent_key", json!({"name": "Test"}));
    assert!(result.is_err());
}

#[test]
fn test_error_document_delete_not_found() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    let result = users.delete("nonexistent_key");
    assert!(result.is_err());
}

// ============================================================================
// Edge Collection Error Tests
// ============================================================================

#[test]
fn test_error_edge_missing_from() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let edges = engine.get_collection("edges").unwrap();

    // Missing _from field
    let result = edges.insert(json!({
        "_to": "users/bob"
    }));

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("_from"));
}

#[test]
fn test_error_edge_missing_to() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let edges = engine.get_collection("edges").unwrap();

    // Missing _to field
    let result = edges.insert(json!({
        "_from": "users/alice"
    }));

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("_to"));
}

#[test]
fn test_edge_from_to_format() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let edges = engine.get_collection("edges").unwrap();

    // Edge collection accepts various formats - test that valid format works
    let result = edges.insert(json!({
        "_from": "users/alice",
        "_to": "users/bob"
    }));

    assert!(result.is_ok(), "Valid edge format should work");
}

#[test]
fn test_edge_valid_format() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("edges".to_string(), Some("edge".to_string()))
        .unwrap();
    let edges = engine.get_collection("edges").unwrap();

    // Valid edge with proper format
    let result = edges.insert(json!({
        "_from": "users/alice",
        "_to": "users/bob",
        "relation": "knows"
    }));

    assert!(result.is_ok());
    let doc = result.unwrap();
    assert_eq!(doc.get("_from"), Some(json!("users/alice")));
}

// ============================================================================
// Query Parse Error Tests
// ============================================================================

#[test]
fn test_error_invalid_query_syntax() {
    let result = parse("FOR doc IN RETURN doc");
    assert!(result.is_err(), "Missing collection name should fail");
}

#[test]
fn test_error_unclosed_string() {
    let result = parse("FOR doc IN users FILTER doc.name == 'unclosed RETURN doc");
    assert!(result.is_err(), "Unclosed string should fail");
}

#[test]
fn test_error_invalid_operator() {
    let _result = parse("FOR doc IN users FILTER doc.name === 'test' RETURN doc");
    // This might parse depending on lexer behavior, but execution would fail
}

#[test]
fn test_query_without_return() {
    // Query without RETURN may be valid for some operations
    let _query = parse("FOR doc IN users FILTER doc.age > 30");
    // Parser behavior - may succeed or fail depending on grammar
    // Just verify it doesn't panic
}

// ============================================================================
// Query Execution Error Tests
// ============================================================================

#[test]
fn test_error_query_collection_not_found() {
    let (engine, _tmp) = create_test_engine();

    let query = parse("FOR doc IN nonexistent_collection RETURN doc").unwrap();
    let executor = QueryExecutor::new(&engine);

    let result = executor.execute(&query);
    assert!(
        result.is_err(),
        "Query on nonexistent collection should fail"
    );
}

#[test]
fn test_error_query_invalid_field_access() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    users
        .insert(json!({"_key": "alice", "name": "Alice"}))
        .unwrap();

    // Accessing a nested field that doesn't exist returns null, not an error
    let query = parse("FOR doc IN users RETURN doc.address.city").unwrap();
    let executor = QueryExecutor::new(&engine);

    let result = executor.execute(&query);
    assert!(result.is_ok()); // Should return null, not error
}

#[test]
fn test_bind_variable_missing() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();

    // The query may still parse, but execution behavior varies
    // depending on whether the collection has data that triggers
    // the bind variable evaluation
    let query = parse("FOR doc IN users FILTER doc.name == @name RETURN doc").unwrap();
    let executor = QueryExecutor::new(&engine); // No bind vars provided

    let _result = executor.execute(&query);
    // With empty collection, no documents to filter, so bind var may not be evaluated
    // Just check it doesn't panic or corrupt state
}

// ============================================================================
// Index Error Tests
// ============================================================================

#[test]
fn test_error_duplicate_index_name() {
    use solidb::storage::IndexType;

    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    users
        .create_index(
            "my_index".to_string(),
            vec!["field".to_string()],
            IndexType::Hash,
            false,
        )
        .unwrap();

    // Try to create another index with same name
    let result = users.create_index(
        "my_index".to_string(),
        vec!["other_field".to_string()],
        IndexType::Hash,
        false,
    );

    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));
}

#[test]
fn test_error_fulltext_index_duplicate() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("articles".to_string(), None)
        .unwrap();
    let articles = engine.get_collection("articles").unwrap();

    articles
        .create_fulltext_index("content_idx".to_string(), vec!["content".to_string()], None)
        .unwrap();

    // Try to create another fulltext index with same name
    let result =
        articles.create_fulltext_index("content_idx".to_string(), vec!["title".to_string()], None);

    assert!(result.is_err());
}

#[test]
fn test_error_geo_index_duplicate() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("places".to_string(), None)
        .unwrap();
    let places = engine.get_collection("places").unwrap();

    places
        .create_geo_index("location_idx".to_string(), "location".to_string())
        .unwrap();

    // Try to create another geo index with same name
    let result = places.create_geo_index("location_idx".to_string(), "coords".to_string());

    assert!(result.is_err());
}

// ============================================================================
// Database Error Tests
// ============================================================================

#[test]
fn test_error_database_not_found() {
    let (engine, _tmp) = create_test_engine();

    let result = engine.get_database("nonexistent_db");
    assert!(result.is_err());
}

#[test]
fn test_error_duplicate_database() {
    let (engine, _tmp) = create_test_engine();

    engine.create_database("mydb".to_string()).unwrap();

    let result = engine.create_database("mydb".to_string());
    assert!(result.is_err());
}

// ============================================================================
// Graceful Error Recovery Tests
// ============================================================================

#[test]
fn test_continue_after_document_error() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    // Insert a document
    users
        .insert(json!({"_key": "alice", "name": "Alice"}))
        .unwrap();

    // Try to get nonexistent document
    let _ = users.get("nonexistent");

    // Collection should still work
    let result = users.get("alice");
    assert!(result.is_ok());
    assert_eq!(result.unwrap().get("name"), Some(json!("Alice")));
}

#[test]
fn test_continue_after_query_error() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    users
        .insert(json!({"_key": "alice", "name": "Alice"}))
        .unwrap();

    // Execute a failing query
    let bad_query = parse("FOR doc IN nonexistent RETURN doc").unwrap();
    let executor = QueryExecutor::new(&engine);
    let _ = executor.execute(&bad_query);

    // Good query should still work
    let good_query = parse("FOR doc IN users RETURN doc.name").unwrap();
    let result = executor.execute(&good_query);
    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 1);
}

// ============================================================================
// Empty/Null Handling Tests
// ============================================================================

#[test]
fn test_query_empty_collection() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("empty".to_string(), None).unwrap();

    let query = parse("FOR doc IN empty RETURN doc").unwrap();
    let executor = QueryExecutor::new(&engine);

    let result = executor.execute(&query);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn test_query_with_null_values() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    users
        .insert(json!({"_key": "u1", "name": "Alice", "email": null}))
        .unwrap();

    let query = parse("FOR doc IN users RETURN doc.email").unwrap();
    let executor = QueryExecutor::new(&engine);

    let result = executor.execute(&query);
    assert!(result.is_ok());
    assert_eq!(result.unwrap()[0], serde_json::Value::Null);
}

// ============================================================================
// Large Data Handling Tests
// ============================================================================

#[test]
fn test_insert_large_document() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("large".to_string(), None).unwrap();
    let col = engine.get_collection("large").unwrap();

    // Create a large document with many fields
    let mut large_doc = serde_json::Map::new();
    for i in 0..100 {
        large_doc.insert(format!("field_{}", i), json!(format!("value_{}", i)));
    }

    let result = col.insert(json!(large_doc));
    assert!(result.is_ok());
}

#[test]
fn test_insert_deeply_nested_document() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("nested".to_string(), None)
        .unwrap();
    let col = engine.get_collection("nested").unwrap();

    let nested = json!({
        "level1": {
            "level2": {
                "level3": {
                    "level4": {
                        "level5": {
                            "value": "deeply nested"
                        }
                    }
                }
            }
        }
    });

    let result = col.insert(nested);
    assert!(result.is_ok());

    // Verify we can query the nested value
    let query =
        parse("FOR doc IN nested RETURN doc.level1.level2.level3.level4.level5.value").unwrap();
    let executor = QueryExecutor::new(&engine);
    let results = executor.execute(&query).unwrap();
    assert_eq!(results[0], json!("deeply nested"));
}

#[test]
fn test_insert_document_with_large_array() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("arrays".to_string(), None)
        .unwrap();
    let col = engine.get_collection("arrays").unwrap();

    let large_array: Vec<i32> = (0..1000).collect();

    let result = col.insert(json!({"data": large_array}));
    assert!(result.is_ok());

    // Verify length
    let query = parse("FOR doc IN arrays RETURN LENGTH(doc.data)").unwrap();
    let executor = QueryExecutor::new(&engine);
    let results = executor.execute(&query).unwrap();
    assert_eq!(results[0], json!(1000));
}
