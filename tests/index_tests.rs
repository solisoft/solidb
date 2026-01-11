//! Index and Fulltext Search Tests
//!
//! Tests for index functionality including:
//! - Hash indexes
//! - Persistent indexes  
//! - Fulltext indexes
//! - Geo indexes
//! - Compound indexes

use serde_json::json;
use solidb::storage::{IndexType, StorageEngine};
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

// ============================================================================
// Hash Index Tests
// ============================================================================

#[test]
fn test_hash_index_create() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    let result = users.create_index(
        "email_idx".to_string(),
        vec!["email".to_string()],
        IndexType::Hash,
        false,
    );

    assert!(
        result.is_ok(),
        "Should create hash index: {:?}",
        result.err()
    );
}

#[test]
fn test_hash_index_list() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    users
        .create_index(
            "idx1".to_string(),
            vec!["field1".to_string()],
            IndexType::Hash,
            false,
        )
        .unwrap();
    users
        .create_index(
            "idx2".to_string(),
            vec!["field2".to_string()],
            IndexType::Hash,
            false,
        )
        .unwrap();

    let indexes = users.list_indexes();
    assert_eq!(indexes.len(), 2);
}

#[test]
fn test_hash_index_unique() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    let result = users.create_index(
        "email_unique".to_string(),
        vec!["email".to_string()],
        IndexType::Hash,
        true, // unique
    );

    assert!(result.is_ok());
    let stats = result.unwrap();
    assert!(stats.unique);
}

// ============================================================================
// Persistent Index Tests
// ============================================================================

#[test]
fn test_persistent_index_create() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();

    let result = products.create_index(
        "price_idx".to_string(),
        vec!["price".to_string()],
        IndexType::Persistent,
        false,
    );

    assert!(result.is_ok());
}

#[test]
fn test_persistent_index_with_data() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();

    // Insert data first
    products
        .insert(json!({"_key": "p1", "name": "Laptop", "price": 999}))
        .unwrap();
    products
        .insert(json!({"_key": "p2", "name": "Phone", "price": 599}))
        .unwrap();
    products
        .insert(json!({"_key": "p3", "name": "Tablet", "price": 399}))
        .unwrap();

    // Create index on existing data
    let result = products.create_index(
        "price_idx".to_string(),
        vec!["price".to_string()],
        IndexType::Persistent,
        false,
    );

    assert!(result.is_ok());
    let stats = result.unwrap();
    assert_eq!(stats.indexed_documents, 3);
}

// ============================================================================
// Compound Index Tests
// ============================================================================

#[test]
fn test_compound_index_create() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    // Create compound index on multiple fields
    let result = users.create_index(
        "city_age_idx".to_string(),
        vec!["city".to_string(), "age".to_string()],
        IndexType::Persistent,
        false,
    );

    assert!(result.is_ok());
    let stats = result.unwrap();
    assert_eq!(stats.fields.len(), 2);
}

#[test]
fn test_compound_index_with_data() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();

    users
        .insert(json!({"_key": "u1", "city": "Paris", "age": 30}))
        .unwrap();
    users
        .insert(json!({"_key": "u2", "city": "London", "age": 25}))
        .unwrap();
    users
        .insert(json!({"_key": "u3", "city": "Paris", "age": 35}))
        .unwrap();

    let result = users.create_index(
        "compound_idx".to_string(),
        vec!["city".to_string(), "age".to_string()],
        IndexType::Hash,
        false,
    );

    assert!(result.is_ok());
    let stats = result.unwrap();
    assert_eq!(stats.indexed_documents, 3);
}

// ============================================================================
// Fulltext Index Tests
// ============================================================================

#[test]
fn test_fulltext_index_create() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("articles".to_string(), None)
        .unwrap();
    let articles = engine.get_collection("articles").unwrap();

    let result = articles.create_fulltext_index(
        "content_ft".to_string(),
        vec!["content".to_string()],
        Some(3), // min_length
    );

    assert!(result.is_ok());
}

#[test]
fn test_fulltext_index_with_data() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("articles".to_string(), None)
        .unwrap();
    let articles = engine.get_collection("articles").unwrap();

    articles
        .insert(json!({
            "_key": "a1",
            "title": "Introduction to Rust",
            "content": "Rust is a systems programming language focused on safety and performance."
        }))
        .unwrap();

    articles
        .insert(json!({
            "_key": "a2",
            "title": "Database Design",
            "content": "Learn about database design patterns and best practices."
        }))
        .unwrap();

    let result = articles.create_fulltext_index(
        "content_ft".to_string(),
        vec!["content".to_string()],
        Some(3),
    );

    assert!(result.is_ok());
}

#[test]
fn test_fulltext_search() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("articles".to_string(), None)
        .unwrap();
    let articles = engine.get_collection("articles").unwrap();

    articles
        .insert(json!({
            "_key": "a1",
            "content": "The quick brown fox jumps over the lazy dog"
        }))
        .unwrap();

    articles
        .insert(json!({
            "_key": "a2",
            "content": "A lazy cat sleeps all day"
        }))
        .unwrap();

    articles
        .create_fulltext_index(
            "content_ft".to_string(),
            vec!["content".to_string()],
            Some(3),
        )
        .unwrap();

    // Search for "lazy" with max_distance 0 (exact match)
    let results = articles.fulltext_search("content", "lazy", 0);
    assert!(results.is_some());
    let matches = results.unwrap();
    assert_eq!(matches.len(), 2); // Both documents contain "lazy"
}

#[test]
fn test_fulltext_list_indexes() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("articles".to_string(), None)
        .unwrap();
    let articles = engine.get_collection("articles").unwrap();

    articles
        .create_fulltext_index(
            "title_ft".to_string(),
            vec!["title".to_string()],
            None, // default min_length
        )
        .unwrap();

    articles
        .create_fulltext_index(
            "content_ft".to_string(),
            vec!["content".to_string()],
            Some(4),
        )
        .unwrap();

    let ft_indexes = articles.list_fulltext_indexes();
    assert_eq!(ft_indexes.len(), 2);
}

// ============================================================================
// Geo Index Tests
// ============================================================================

#[test]
fn test_geo_index_create() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("places".to_string(), None)
        .unwrap();
    let places = engine.get_collection("places").unwrap();

    let result = places.create_geo_index("location_geo".to_string(), "location".to_string());
    assert!(result.is_ok());
}

#[test]
fn test_geo_index_with_data() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("places".to_string(), None)
        .unwrap();
    let places = engine.get_collection("places").unwrap();

    // Insert places with geo coordinates
    places
        .insert(json!({
            "_key": "paris",
            "name": "Paris",
            "location": {"lat": 48.8566, "lon": 2.3522}
        }))
        .unwrap();

    places
        .insert(json!({
            "_key": "london",
            "name": "London",
            "location": {"lat": 51.5074, "lon": -0.1278}
        }))
        .unwrap();

    let result = places.create_geo_index("location_geo".to_string(), "location".to_string());
    assert!(result.is_ok());
}

#[test]
fn test_geo_near_search() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("places".to_string(), None)
        .unwrap();
    let places = engine.get_collection("places").unwrap();

    places
        .insert(json!({
            "_key": "paris",
            "name": "Paris",
            "location": {"lat": 48.8566, "lon": 2.3522}
        }))
        .unwrap();

    places
        .insert(json!({
            "_key": "london",
            "name": "London",
            "location": {"lat": 51.5074, "lon": -0.1278}
        }))
        .unwrap();

    places
        .insert(json!({
            "_key": "berlin",
            "name": "Berlin",
            "location": {"lat": 52.5200, "lon": 13.4050}
        }))
        .unwrap();

    places
        .create_geo_index("location_geo".to_string(), "location".to_string())
        .unwrap();

    // Search near Paris (should return Paris as closest)
    let results = places.geo_near("location", 48.8566, 2.3522, 10);
    assert!(results.is_some());
    let matches = results.unwrap();
    assert!(!matches.is_empty());

    // First result should be Paris (closest to search point) - returns (Document, distance)
    let (first_doc, _distance) = &matches[0];
    assert_eq!(first_doc.key, "paris");
}

#[test]
fn test_geo_list_indexes() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("places".to_string(), None)
        .unwrap();
    let places = engine.get_collection("places").unwrap();

    places
        .create_geo_index("geo1".to_string(), "location1".to_string())
        .unwrap();
    places
        .create_geo_index("geo2".to_string(), "location2".to_string())
        .unwrap();

    let geo_indexes = places.list_geo_indexes();
    assert_eq!(geo_indexes.len(), 2);
}

// ============================================================================
// Index Statistics Tests
// ============================================================================

#[test]
fn test_index_statistics() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("stats_test".to_string(), None)
        .unwrap();
    let col = engine.get_collection("stats_test").unwrap();

    // Insert data
    for i in 0..100 {
        col.insert(json!({"value": i})).unwrap();
    }

    // Create index
    let stats = col
        .create_index(
            "value_idx".to_string(),
            vec!["value".to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();

    assert_eq!(stats.indexed_documents, 100);
    assert_eq!(stats.name, "value_idx");
}

// ============================================================================
// Index Rebuild Tests
// ============================================================================

#[test]
fn test_rebuild_all_indexes() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("rebuild_test".to_string(), None)
        .unwrap();
    let col = engine.get_collection("rebuild_test").unwrap();

    // Create index
    col.create_index(
        "field_idx".to_string(),
        vec!["field".to_string()],
        IndexType::Hash,
        false,
    )
    .unwrap();

    // Insert data after index creation
    for i in 0..50 {
        col.insert(json!({"field": format!("value_{}", i)}))
            .unwrap();
    }

    // Rebuild should re-index all documents
    let result = col.rebuild_all_indexes();
    assert!(result.is_ok());
}

// ============================================================================
// Duplicate Index Tests
// ============================================================================

#[test]
fn test_duplicate_index_name_error() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("dup_test".to_string(), None)
        .unwrap();
    let col = engine.get_collection("dup_test").unwrap();

    // Create first index
    col.create_index(
        "my_idx".to_string(),
        vec!["field1".to_string()],
        IndexType::Hash,
        false,
    )
    .unwrap();

    // Try to create another with same name - should fail
    let result = col.create_index(
        "my_idx".to_string(),
        vec!["field2".to_string()],
        IndexType::Hash,
        false,
    );
    assert!(result.is_err());
}
