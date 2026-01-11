//! JSON Schema Validation Tests
//!
//! Tests for optional JSON Schema enforcement on collections:
//! - Setting and retrieving schemas
//! - Validation modes (off, strict, lenient)
//! - Document validation on insert/update
//! - Invalid schema handling
//! - Schema removal

use serde_json::json;
use solidb::storage::StorageEngine;
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine
        .initialize()
        .expect("Failed to initialize storage engine");
    (engine, tmp_dir)
}

#[test]
fn test_collection_without_schema() {
    let (engine, _tmp_dir) = create_test_engine();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("users".to_string(), None).unwrap();

    let collection = db.get_collection("users").unwrap();

    // No schema should be set
    assert!(collection.get_json_schema().is_none());

    // Should be able to insert any document
    let doc = json!({
        "name": "Alice",
        "age": 30,
        "unexpected": "field"
    });
    let result = collection.insert(doc.clone());
    assert!(result.is_ok());
}

#[test]
fn test_set_schema_strict() {
    let (engine, _tmp_dir) = create_test_engine();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("users".to_string(), None).unwrap();

    let collection = db.get_collection("users").unwrap();

    // Set a strict schema
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "number", "minimum": 0 }
        },
        "required": ["name", "age"],
        "additionalProperties": false
    });

    use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
    collection
        .set_json_schema(CollectionSchema::new(
            "default".to_string(),
            schema,
            SchemaValidationMode::Strict,
        ))
        .unwrap();

    // Schema should be retrievable
    let retrieved_schema = collection.get_json_schema();
    assert!(retrieved_schema.is_some());
    let retrieved = retrieved_schema.unwrap();
    assert_eq!(retrieved.name, "default");
    assert_eq!(retrieved.validation_mode, SchemaValidationMode::Strict);

    // Valid document should insert successfully
    let valid_doc = json!({
        "name": "Alice",
        "age": 30
    });
    let result = collection.insert(valid_doc);
    assert!(result.is_ok());
}

#[test]
fn test_schema_rejects_invalid_document() {
    let (engine, _tmp_dir) = create_test_engine();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("users".to_string(), None).unwrap();

    let collection = db.get_collection("users").unwrap();

    // Set a strict schema
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "number" }
        },
        "required": ["name", "age"]
    });

    use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
    collection
        .set_json_schema(CollectionSchema::new(
            "default".to_string(),
            schema,
            SchemaValidationMode::Strict,
        ))
        .unwrap();

    // Invalid document (missing required field, wrong type)
    let invalid_doc = json!({
        "name": "Bob",
        "age": "thirty" // Should be number
    });
    let result = collection.insert(invalid_doc);
    assert!(result.is_err());

    // Error should mention schema validation
    if let Err(e) = result {
        let error_msg = e.to_string();
        assert!(
            error_msg.contains("schema") || error_msg.contains("Schema"),
            "Error should mention schema validation: {}",
            error_msg
        );
    }
}

#[test]
fn test_schema_lenient_mode() {
    let (engine, _tmp_dir) = create_test_engine();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("users".to_string(), None).unwrap();

    let collection = db.get_collection("users").unwrap();

    // Set a lenient schema
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "number" }
        },
        "required": ["name"]
    });

    use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
    collection
        .set_json_schema(CollectionSchema::new(
            "default".to_string(),
            schema,
            SchemaValidationMode::Lenient,
        ))
        .unwrap();

    // Invalid document should still be accepted in lenient mode
    let doc = json!({
        "name": "Charlie",
        "age": "not_a_number" // Wrong type but lenient
    });
    let result = collection.insert(doc);
    assert!(
        result.is_ok(),
        "Document should be accepted in lenient mode"
    );
}

#[test]
fn test_schema_validation_on_update() {
    let (engine, _tmp_dir) = create_test_engine();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("products".to_string(), None).unwrap();

    let collection = db.get_collection("products").unwrap();

    // Set schema
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "price": { "type": "number", "minimum": 0 }
        },
        "required": ["name", "price"]
    });

    use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
    collection
        .set_json_schema(CollectionSchema::new(
            "default".to_string(),
            schema,
            SchemaValidationMode::Strict,
        ))
        .unwrap();

    // Insert valid document with explicit key
    let doc = json!({
        "_key": "widget-1",
        "name": "Widget",
        "price": 10.99
    });
    collection.insert(doc).unwrap();

    // Update to valid document should succeed
    let update = json!({
        "price": 15.99
    });
    let result = collection.update("widget-1", update);
    assert!(result.is_ok());

    // Update to invalid document should fail
    let invalid_update = json!({
        "price": "free" // Should be number
    });
    let result = collection.update("widget-1", invalid_update);
    assert!(result.is_err());
}

#[test]
fn test_remove_schema() {
    let (engine, _tmp_dir) = create_test_engine();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("items".to_string(), None).unwrap();

    let collection = db.get_collection("items").unwrap();

    // Set schema
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        }
    });

    use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
    collection
        .set_json_schema(CollectionSchema::new(
            "default".to_string(),
            schema,
            SchemaValidationMode::Strict,
        ))
        .unwrap();

    // Schema should be present
    assert!(collection.get_json_schema().is_some());

    // Remove schema
    collection.remove_json_schema().unwrap();

    // Schema should be gone
    assert!(collection.get_json_schema().is_none());

    // Should be able to insert any document now
    let any_doc = json!({
        "arbitrary": "data",
        "nested": { "structure": true }
    });
    let result = collection.insert(any_doc);
    assert!(result.is_ok());
}

#[test]
fn test_schema_additional_properties_false() {
    let (engine, _tmp_dir) = create_test_engine();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("users".to_string(), None).unwrap();

    let collection = db.get_collection("users").unwrap();

    // Set schema with additionalProperties: false
    let schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "email": { "type": "string" }
        },
        "required": ["name"],
        "additionalProperties": false
    });

    use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
    collection
        .set_json_schema(CollectionSchema::new(
            "default".to_string(),
            schema,
            SchemaValidationMode::Strict,
        ))
        .unwrap();

    // Document with only allowed fields should work
    let valid_doc = json!({
        "name": "Alice",
        "email": "alice@example.com"
    });
    assert!(collection.insert(valid_doc).is_ok());

    // Document with extra fields should fail
    let invalid_doc = json!({
        "name": "Bob",
        "email": "bob@example.com",
        "extra_field": "not allowed" // Should fail with additionalProperties: false
    });
    let result = collection.insert(invalid_doc);
    assert!(result.is_err());
}

#[test]
fn test_schema_with_nested_objects() {
    let (engine, _tmp_dir) = create_test_engine();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("orders".to_string(), None).unwrap();

    let collection = db.get_collection("orders").unwrap();

    // Set schema with nested objects
    let schema = json!({
        "type": "object",
        "properties": {
            "orderId": { "type": "string" },
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "productId": { "type": "string" },
                        "quantity": { "type": "number", "minimum": 1 }
                    },
                    "required": ["productId", "quantity"]
                }
            }
        },
        "required": ["orderId", "items"]
    });

    use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
    collection
        .set_json_schema(CollectionSchema::new(
            "default".to_string(),
            schema,
            SchemaValidationMode::Strict,
        ))
        .unwrap();

    // Valid nested document should work
    let valid_doc = json!({
        "orderId": "ORD-123",
        "items": [
            { "productId": "P1", "quantity": 2 },
            { "productId": "P2", "quantity": 1 }
        ]
    });
    let result = collection.insert(valid_doc);
    assert!(result.is_ok(), "Should accept valid nested document");
}

#[test]
fn test_invalid_schema_rejection() {
    let (engine, _tmp_dir) = create_test_engine();
    let db = engine.get_database("_system").unwrap();
    db.create_collection("test".to_string(), None).unwrap();

    let collection = db.get_collection("test").unwrap();

    // Try to set an invalid JSON Schema
    let invalid_schema = json!({
        "type": "invalid_type",
        "minLength": -1 // Negative minLength is invalid
    });

    use solidb::storage::schema::{CollectionSchema, SchemaValidationMode};
    let result = collection.set_json_schema(CollectionSchema::new(
        "default".to_string(),
        invalid_schema,
        SchemaValidationMode::Strict,
    ));

    // Should fail to set invalid schema
    assert!(result.is_err());
    if let Err(e) = result {
        let error_msg = e.to_string();
        assert!(
            error_msg.contains("schema") || error_msg.contains("Schema"),
            "Error should mention schema: {}",
            error_msg
        );
    }
}
