//! Vector Index Tests
//!
//! Tests for vector similarity search functionality including:
//! - Vector index creation and management
//! - Vector similarity search
//! - SDBQL vector functions

use serde_json::json;
use solidb::storage::{StorageEngine, VectorIndexConfig, VectorMetric};
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

/// Helper to execute a query and get all results
fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<serde_json::Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .execute(&query)
        .expect(&format!("Query failed: {}", query_str))
}

// ============================================================================
// Vector Index CRUD Tests
// ============================================================================

#[test]
fn test_vector_index_create() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("products").unwrap();

    let config = VectorIndexConfig::new(
        "embedding_idx".to_string(),
        "embedding".to_string(),
        3, // 3 dimensions for easy testing
    );

    let result = collection.create_vector_index(config);
    assert!(
        result.is_ok(),
        "Should create vector index: {:?}",
        result.err()
    );

    let stats = result.unwrap();
    assert_eq!(stats.name, "embedding_idx");
    assert_eq!(stats.field, "embedding");
    assert_eq!(stats.dimension, 3);
}

#[test]
fn test_vector_index_create_with_metric() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("products").unwrap();

    let config = VectorIndexConfig::new("embedding_idx".to_string(), "embedding".to_string(), 3)
        .with_metric(VectorMetric::Euclidean);

    let result = collection.create_vector_index(config);
    assert!(result.is_ok());

    let stats = result.unwrap();
    assert_eq!(stats.metric, VectorMetric::Euclidean);
}

#[test]
fn test_vector_index_list() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("products").unwrap();

    let config1 = VectorIndexConfig::new("idx1".to_string(), "vec1".to_string(), 3);
    let config2 = VectorIndexConfig::new("idx2".to_string(), "vec2".to_string(), 5);

    collection.create_vector_index(config1).unwrap();
    collection.create_vector_index(config2).unwrap();

    let indexes = collection.list_vector_indexes();
    assert_eq!(indexes.len(), 2);
}

#[test]
fn test_vector_index_drop() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("products").unwrap();

    let config = VectorIndexConfig::new("embedding_idx".to_string(), "embedding".to_string(), 3);
    collection.create_vector_index(config).unwrap();

    let result = collection.drop_vector_index("embedding_idx");
    assert!(result.is_ok());

    let indexes = collection.list_vector_indexes();
    assert_eq!(indexes.len(), 0);
}

#[test]
fn test_vector_index_duplicate_name() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("products").unwrap();

    let config1 = VectorIndexConfig::new("embedding_idx".to_string(), "embedding".to_string(), 3);
    let config2 = VectorIndexConfig::new("embedding_idx".to_string(), "other".to_string(), 3);

    collection.create_vector_index(config1).unwrap();
    let result = collection.create_vector_index(config2);

    assert!(result.is_err(), "Should fail with duplicate index name");
}

// ============================================================================
// Vector Search Tests
// ============================================================================

#[test]
fn test_vector_search_basic() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("products").unwrap();

    // Insert documents with vectors
    collection
        .insert(json!({
            "_key": "p1",
            "name": "Product A",
            "embedding": [1.0, 0.0, 0.0]
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "p2",
            "name": "Product B",
            "embedding": [0.0, 1.0, 0.0]
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "p3",
            "name": "Product C",
            "embedding": [0.9, 0.1, 0.0]
        }))
        .unwrap();

    // Create vector index
    let config = VectorIndexConfig::new("embedding_idx".to_string(), "embedding".to_string(), 3);
    collection.create_vector_index(config).unwrap();

    // Search for similar vectors to [1, 0, 0]
    let results = collection
        .vector_search("embedding_idx", &[1.0, 0.0, 0.0], 2, None)
        .unwrap();

    assert_eq!(results.len(), 2);
    // p1 should be most similar (exact match)
    assert_eq!(results[0].doc_key, "p1");
    assert!((results[0].score - 1.0).abs() < 0.001);
}

#[test]
fn test_vector_search_euclidean() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("products").unwrap();

    // Insert documents
    collection
        .insert(json!({
            "_key": "p1",
            "embedding": [0.0, 0.0, 0.0]
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "p2",
            "embedding": [1.0, 0.0, 0.0]
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "p3",
            "embedding": [10.0, 0.0, 0.0]
        }))
        .unwrap();

    // Create vector index with Euclidean metric
    let config = VectorIndexConfig::new("embedding_idx".to_string(), "embedding".to_string(), 3)
        .with_metric(VectorMetric::Euclidean);
    collection.create_vector_index(config).unwrap();

    // Search from origin - p1 should be closest (distance 0)
    let results = collection
        .vector_search("embedding_idx", &[0.0, 0.0, 0.0], 3, None)
        .unwrap();

    assert_eq!(results[0].doc_key, "p1");
    assert!(results[0].score < 0.001); // distance ~0
}

// ============================================================================
// SDBQL Vector Function Tests
// ============================================================================

#[test]
fn test_sdbql_vector_similarity() {
    let (engine, _tmp) = create_test_engine();

    // Test cosine similarity of identical vectors
    let result = execute_query(
        &engine,
        "RETURN VECTOR_SIMILARITY([1.0, 0.0, 0.0], [1.0, 0.0, 0.0])",
    );

    let value = &result[0];
    let score = value.as_f64().unwrap();
    assert!(
        (score - 1.0).abs() < 0.001,
        "Identical vectors should have similarity 1.0"
    );
}

#[test]
fn test_sdbql_vector_similarity_orthogonal() {
    let (engine, _tmp) = create_test_engine();

    // Test cosine similarity of orthogonal vectors
    let result = execute_query(
        &engine,
        "RETURN VECTOR_SIMILARITY([1.0, 0.0, 0.0], [0.0, 1.0, 0.0])",
    );

    let value = &result[0];
    let score = value.as_f64().unwrap();
    assert!(
        score.abs() < 0.001,
        "Orthogonal vectors should have similarity ~0"
    );
}

#[test]
fn test_sdbql_vector_distance() {
    let (engine, _tmp) = create_test_engine();

    // Test Euclidean distance
    let result = execute_query(
        &engine,
        "RETURN VECTOR_DISTANCE([0.0, 0.0], [3.0, 4.0], 'euclidean')",
    );

    let value = &result[0];
    let distance = value.as_f64().unwrap();
    assert!(
        (distance - 5.0).abs() < 0.001,
        "Distance should be 5.0 (3-4-5 triangle)"
    );
}

#[test]
fn test_sdbql_vector_normalize() {
    let (engine, _tmp) = create_test_engine();

    // Test vector normalization
    let result = execute_query(&engine, "RETURN VECTOR_NORMALIZE([3.0, 4.0, 0.0])");

    let normalized = result[0].as_array().unwrap();
    assert_eq!(normalized.len(), 3);

    // Magnitude should be 1
    let x = normalized[0].as_f64().unwrap();
    let y = normalized[1].as_f64().unwrap();
    let z = normalized[2].as_f64().unwrap();
    let magnitude = (x * x + y * y + z * z).sqrt();
    assert!(
        (magnitude - 1.0).abs() < 0.001,
        "Normalized vector should have magnitude 1.0"
    );
}

#[test]
fn test_sdbql_vector_in_filter() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let collection = engine.get_collection("items").unwrap();

    collection
        .insert(json!({
            "_key": "item1",
            "embedding": [1.0, 0.0, 0.0]
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "item2",
            "embedding": [0.9, 0.1, 0.0]
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "item3",
            "embedding": [0.0, 1.0, 0.0]
        }))
        .unwrap();

    // Find items with high similarity to [1, 0, 0]
    let query = r#"
        FOR doc IN items
        LET sim = VECTOR_SIMILARITY(doc.embedding, [1.0, 0.0, 0.0])
        FILTER sim > 0.8
        RETURN {_key: doc._key, similarity: sim}
    "#;

    let result = execute_query(&engine, query);
    assert_eq!(result.len(), 2); // item1 and item2 should match
}

#[test]
fn test_sdbql_vector_sort() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let collection = engine.get_collection("items").unwrap();

    collection
        .insert(json!({
            "_key": "item1",
            "embedding": [1.0, 0.0, 0.0]
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "item2",
            "embedding": [0.5, 0.5, 0.0]
        }))
        .unwrap();

    collection
        .insert(json!({
            "_key": "item3",
            "embedding": [0.0, 1.0, 0.0]
        }))
        .unwrap();

    // Sort by similarity to [1, 0, 0]
    let query = r#"
        FOR doc IN items
        LET sim = VECTOR_SIMILARITY(doc.embedding, [1.0, 0.0, 0.0])
        SORT sim DESC
        LIMIT 2
        RETURN doc._key
    "#;

    let result = execute_query(&engine, query);
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].as_str().unwrap(), "item1"); // Most similar
}

// ============================================================================
// Vector Index Persistence Tests
// ============================================================================

#[test]
fn test_vector_index_persistence() {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = tmp_dir.path().to_str().unwrap().to_string();

    // Create engine, collection, and index
    {
        let engine = StorageEngine::new(&path).expect("Failed to create storage engine");
        engine
            .create_collection("products".to_string(), None)
            .unwrap();
        let collection = engine.get_collection("products").unwrap();

        collection
            .insert(json!({
                "_key": "p1",
                "embedding": [1.0, 0.0, 0.0]
            }))
            .unwrap();

        let config =
            VectorIndexConfig::new("embedding_idx".to_string(), "embedding".to_string(), 3);
        collection.create_vector_index(config).unwrap();
    }

    // Reopen and verify index exists
    {
        let engine = StorageEngine::new(&path).expect("Failed to reopen storage engine");
        let collection = engine.get_collection("products").unwrap();

        let config = collection.get_vector_index("embedding_idx");
        assert!(
            config.is_ok(),
            "Vector index should persist across restarts"
        );
    }
}

// ============================================================================
// HNSW Search Tests
// ============================================================================

#[test]
fn test_vector_search_with_ef_search_parameter() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let collection = engine.get_collection("items").unwrap();

    // Insert test vectors
    for i in 0..100 {
        let x = (i as f32) / 100.0;
        collection
            .insert(json!({
                "_key": format!("item_{}", i),
                "embedding": [x, 1.0 - x, 0.0]
            }))
            .unwrap();
    }

    let config = VectorIndexConfig::new("embedding_idx".to_string(), "embedding".to_string(), 3);
    collection.create_vector_index(config).unwrap();

    // Search with different ef_search values
    let query = vec![0.5, 0.5, 0.0];

    // Low ef_search
    let results_low_ef = collection
        .vector_search("embedding_idx", &query, 5, Some(10))
        .unwrap();
    assert_eq!(results_low_ef.len(), 5, "Should return 5 results");

    // High ef_search
    let results_high_ef = collection
        .vector_search("embedding_idx", &query, 5, Some(100))
        .unwrap();
    assert_eq!(results_high_ef.len(), 5, "Should return 5 results");

    // Default ef_search (None)
    let results_default = collection
        .vector_search("embedding_idx", &query, 5, None)
        .unwrap();
    assert_eq!(results_default.len(), 5, "Should return 5 results");

    // All results should find similar items (around item_50)
    let first_key_low = &results_low_ef[0].doc_key;
    let first_key_high = &results_high_ef[0].doc_key;
    assert!(first_key_low.contains("item_"), "Should find item");
    assert!(first_key_high.contains("item_"), "Should find item");
}

#[test]
fn test_vector_index_serialization_v2() {
    // Test that vector index V2 format (with HNSW support) serializes correctly
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = tmp_dir.path().to_str().unwrap().to_string();

    // Create and populate index
    {
        let engine = StorageEngine::new(&path).expect("Failed to create storage engine");
        engine.create_collection("docs".to_string(), None).unwrap();
        let collection = engine.get_collection("docs").unwrap();

        // Insert some vectors with different directions (for cosine similarity)
        for i in 0..50 {
            let angle = (i as f32) * std::f32::consts::PI / 100.0; // 0 to ~pi/2
            collection
                .insert(json!({
                    "_key": format!("doc_{}", i),
                    "embedding": [angle.cos(), angle.sin(), 0.0]
                }))
                .unwrap();
        }

        let config = VectorIndexConfig::new("idx".to_string(), "embedding".to_string(), 3);
        collection.create_vector_index(config).unwrap();

        // Perform a search to verify it works
        let results = collection
            .vector_search("idx", &[1.0, 0.0, 0.0], 3, None)
            .unwrap();
        assert!(
            !results.is_empty(),
            "Search should return results before restart"
        );
    }

    // Reopen and verify search still works
    {
        let engine = StorageEngine::new(&path).expect("Failed to reopen storage engine");
        let collection = engine.get_collection("docs").unwrap();

        // Search should work after restart
        let results = collection
            .vector_search("idx", &[1.0, 0.0, 0.0], 3, None)
            .unwrap();
        assert!(!results.is_empty(), "Search should work after restart");

        // Verify best result is doc_0 (angle 0 = [1, 0, 0])
        assert_eq!(results[0].doc_key, "doc_0", "Should find closest document");
    }
}

#[test]
fn test_vector_index_delete_and_search() {
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("items".to_string(), None).unwrap();
    let collection = engine.get_collection("items").unwrap();

    // Insert test vectors
    collection
        .insert(json!({"_key": "a", "vec": [1.0, 0.0, 0.0]}))
        .unwrap();
    collection
        .insert(json!({"_key": "b", "vec": [0.9, 0.1, 0.0]}))
        .unwrap();
    collection
        .insert(json!({"_key": "c", "vec": [0.0, 1.0, 0.0]}))
        .unwrap();

    let config = VectorIndexConfig::new("idx".to_string(), "vec".to_string(), 3);
    collection.create_vector_index(config).unwrap();

    // Verify initial search
    let results = collection
        .vector_search("idx", &[1.0, 0.0, 0.0], 3, None)
        .unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].doc_key, "a", "Closest should be 'a'");

    // Delete the closest document
    collection.delete("a").unwrap();

    // Search again - 'a' should not appear
    let results_after = collection
        .vector_search("idx", &[1.0, 0.0, 0.0], 3, None)
        .unwrap();
    assert_eq!(results_after.len(), 2, "Should have 2 results after delete");
    assert!(
        !results_after.iter().any(|r| r.doc_key == "a"),
        "Deleted doc should not appear"
    );
    assert_eq!(
        results_after[0].doc_key, "b",
        "Second closest 'b' should now be first"
    );
}
