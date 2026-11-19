//! Vector Index Tests
//!
//! Tests for vector similarity search functionality including:
//! - Vector index creation and management
//! - Vector similarity search
//! - SDBQL vector functions

mod common;
use common::{create_test_engine, execute_query};
use serde_json::json;
use solidb::storage::{StorageEngine, VectorIndexConfig, VectorMetric};
use tempfile::TempDir;

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
fn test_vector_index_create_with_auto_embedding_source() {
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("articles".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("articles").unwrap();

    let config = VectorIndexConfig::new("content_emb".to_string(), "embedding".to_string(), 3)
        .with_embedding_source("content".to_string())
        .with_embedding_provider("openai".to_string())
        .with_embedding_model("text-embedding-3-small".to_string());

    let result = collection.create_vector_index(config);
    assert!(
        result.is_ok(),
        "Should create vector index with embedding_source"
    );

    let stats = result.unwrap();
    assert_eq!(stats.name, "content_emb");
    assert_eq!(stats.field, "embedding");

    // Verify config is persisted with new fields
    let configs = collection.get_all_vector_index_configs();
    let found = configs.iter().find(|c| c.name == "content_emb");
    assert!(found.is_some());
    let found = found.unwrap();
    assert_eq!(found.embedding_source, Some("content".to_string()));
    assert_eq!(found.embedding_provider, Some("openai".to_string()));
    assert_eq!(
        found.embedding_model,
        Some("text-embedding-3-small".to_string())
    );
}

#[test]
fn test_auto_embed_skips_when_no_vector_and_source_but_no_llm() {
    // Without LLM env, auto-embed should gracefully not add vector (no crash, no garbage)
    let (engine, _tmp) = create_test_engine();

    engine.create_collection("docs".to_string(), None).unwrap();
    let collection = engine.get_collection("docs").unwrap();

    let config = VectorIndexConfig::new("emb".to_string(), "embedding".to_string(), 4)
        .with_embedding_source("text".to_string());
    collection.create_vector_index(config).unwrap();

    // Insert doc with source text but no vector
    let res = collection.insert(json!({
        "_key": "d1",
        "text": "hello world for embedding"
        // no "embedding" field
    }));
    assert!(res.is_ok());

    let doc = collection.get("d1").unwrap();
    // Should not have added a (wrong) vector: the storage path never embeds
    // (embedding is done in the async API layer), so no vector field appears.
    assert!(doc
        .get("embedding")
        .is_none_or(|v| v.as_array().is_none_or(|a| a.is_empty())));
}

#[test]
fn test_auto_embed_enqueues_and_clears_pending_markers() {
    // The write path never embeds; instead it records a cheap "pending" marker
    // for the async embedding worker. Verify the enqueue/clear lifecycle.
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("docs".to_string(), None).unwrap();
    let collection = engine.get_collection("docs").unwrap();

    let config = VectorIndexConfig::new("emb".to_string(), "embedding".to_string(), 4)
        .with_embedding_source("text".to_string());
    collection.create_vector_index(config).unwrap();

    // 1. Insert text-only doc → one pending marker enqueued.
    collection
        .insert(json!({"_key": "d1", "text": "hello world"}))
        .unwrap();
    assert_eq!(collection.count_embed_pending(), 1);
    assert_eq!(
        collection.list_embed_pending("emb", 10),
        vec!["d1".to_string()]
    );

    // 2. Insert a doc that already carries a valid vector → no marker.
    collection
        .insert(json!({"_key": "d2", "embedding": [0.1, 0.2, 0.3, 0.4]}))
        .unwrap();
    assert_eq!(collection.count_embed_pending(), 1); // still just d1

    // 3. Supply the vector for d1 → its marker is cleared.
    collection
        .update(
            "d1",
            json!({"_key": "d1", "text": "hello world", "embedding": [0.5, 0.6, 0.7, 0.8]}),
        )
        .unwrap();
    assert_eq!(collection.count_embed_pending(), 0);
    assert!(collection.list_embed_pending("emb", 10).is_empty());

    // 4. Deleting a still-pending doc also clears its marker.
    collection
        .insert(json!({"_key": "d3", "text": "another one"}))
        .unwrap();
    assert_eq!(collection.count_embed_pending(), 1);
    collection.delete("d3").unwrap();
    assert_eq!(collection.count_embed_pending(), 0);
}

#[test]
fn test_vector_search_with_metadata_filter() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let coll = engine.get_collection("items").unwrap();
    coll.create_vector_index(VectorIndexConfig::new(
        "emb".to_string(),
        "embedding".to_string(),
        3,
    ))
    .unwrap();

    coll.insert(json!({"_key": "d1", "embedding": [1.0, 0.0, 0.0], "category": "a"}))
        .unwrap();
    // d2 is the 2nd-nearest to the query but is category "b".
    coll.insert(json!({"_key": "d2", "embedding": [0.9, 0.1, 0.0], "category": "b"}))
        .unwrap();
    coll.insert(json!({"_key": "d3", "embedding": [0.8, 0.0, 0.2], "category": "a"}))
        .unwrap();

    // Unfiltered top-3 includes the category-"b" doc.
    let res = execute_query(
        &engine,
        r#"RETURN VECTOR_SEARCH("items", "emb", [1.0, 0.0, 0.0], 3)"#,
    );
    assert_eq!(res[0].as_array().unwrap().len(), 3);

    // Filtered to category "a": the closer d2 is excluded; d1 and d3 remain.
    let res = execute_query(
        &engine,
        r#"RETURN VECTOR_SEARCH("items", "emb", [1.0, 0.0, 0.0], 2, {filter: {category: "a"}})"#,
    );
    let arr = res[0].as_array().unwrap();
    assert!(!arr.is_empty());
    let keys: Vec<&str> = arr
        .iter()
        .map(|h| h["doc"]["_key"].as_str().unwrap())
        .collect();
    assert!(arr
        .iter()
        .all(|h| h["doc"]["category"].as_str() == Some("a")));
    assert!(keys.contains(&"d1"));
    assert!(keys.contains(&"d3"));
    assert!(!keys.contains(&"d2"));
}

#[test]
fn test_vector_index_build_with_auto_embed_on_populated_collection() {
    // Tests the batched embedding path during index creation on existing data
    let (engine, _tmp) = create_test_engine();

    engine
        .create_collection("articles".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("articles").unwrap();

    // Populate first
    for i in 0..5 {
        collection
            .insert(json!({
                "_key": format!("a{}", i),
                "content": format!("test document number {}", i)
            }))
            .unwrap();
    }

    // Now create auto-embed index; this exercises the collect + batch_blocking path
    let config = VectorIndexConfig::new("content_emb".to_string(), "embedding".to_string(), 3)
        .with_embedding_source("content".to_string());

    let result = collection.create_vector_index(config);
    // Will "fail" to embed (no LLM) but should succeed without panic and index created
    assert!(result.is_ok());

    // No vectors should have been added
    let stats = result.unwrap();
    assert_eq!(stats.indexed_vectors, 0);
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

// ============================================================================
// Throttled-persist durability (batch write path -> shutdown flush -> reopen)
// ============================================================================

#[test]
fn test_vector_index_batch_write_survives_flush_and_reopen() {
    // The batch write paths now persist the vector index on a throttle rather
    // than after every batch (avoiding a full re-serialize per chunk on a bulk
    // load). Durability of the deferred window is provided by the shutdown
    // flush. This exercises that whole contract end to end.
    let tmp = TempDir::new().expect("temp dir");
    let path = tmp.path().to_str().unwrap().to_string();

    {
        let engine = StorageEngine::new(&path).expect("engine");
        engine
            .create_collection("products".to_string(), None)
            .unwrap();
        let collection = engine.get_collection("products").unwrap();
        collection
            .create_vector_index(VectorIndexConfig::new(
                "embedding_idx".to_string(),
                "embedding".to_string(),
                3,
            ))
            .unwrap();

        // Bulk write path: marks the index dirty and defers the disk persist.
        collection
            .insert_batch(vec![
                json!({"_key": "d1", "embedding": [1.0, 0.0, 0.0]}),
                json!({"_key": "d2", "embedding": [0.0, 1.0, 0.0]}),
                json!({"_key": "d3", "embedding": [0.0, 0.0, 1.0]}),
            ])
            .unwrap();

        // In-memory index must be correct immediately after the batch, even if
        // the disk persist was throttled out.
        let live = collection
            .vector_search("embedding_idx", &[1.0, 0.0, 0.0], 1, None)
            .unwrap();
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].doc_key, "d1");

        // The shutdown flush makes the deferred state durable.
        collection.flush_vector_indexes();
    }

    // Reopen at the same path: the index must load from disk with the vectors.
    {
        let engine = StorageEngine::new(&path).expect("reopen engine");
        let collection = engine.get_collection("products").unwrap();
        let hits = collection
            .vector_search("embedding_idx", &[0.0, 1.0, 0.0], 1, None)
            .expect("search after reopen");
        assert_eq!(hits.len(), 1, "index should have survived reopen");
        assert_eq!(hits[0].doc_key, "d2");
    }
}

#[test]
fn test_vector_update_skips_reindex_when_embedding_unchanged() {
    // A document UPDATE that leaves the embedding untouched must not disturb the
    // vector index (the fast path that avoids HNSW churn), while an UPDATE that
    // changes the embedding must re-index correctly.
    let (engine, _tmp) = create_test_engine();
    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let coll = engine.get_collection("products").unwrap();
    coll.create_vector_index(VectorIndexConfig::new(
        "embedding_idx".to_string(),
        "embedding".to_string(),
        3,
    ))
    .unwrap();

    coll.insert(json!({"_key": "d1", "embedding": [1.0, 0.0, 0.0], "label": "a"}))
        .unwrap();
    coll.insert(json!({"_key": "d2", "embedding": [0.0, 1.0, 0.0], "label": "b"}))
        .unwrap();

    // Non-embedding field change: vector unchanged, index must still hold it.
    coll.update("d1", json!({"label": "a2"})).unwrap();
    let hits = coll
        .vector_search("embedding_idx", &[1.0, 0.0, 0.0], 1, None)
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].doc_key, "d1",
        "unchanged-embedding update must not drop the vector"
    );

    // Embedding change: index must reflect the new vector.
    coll.update("d1", json!({"embedding": [0.0, 0.0, 1.0]}))
        .unwrap();
    let z = coll
        .vector_search("embedding_idx", &[0.0, 0.0, 1.0], 1, None)
        .unwrap();
    assert_eq!(z[0].doc_key, "d1", "changed embedding must be re-indexed");
    let x = coll
        .vector_search("embedding_idx", &[1.0, 0.0, 0.0], 2, None)
        .unwrap();
    let d1_score = x
        .iter()
        .find(|r| r.doc_key == "d1")
        .map(|r| r.score)
        .unwrap();
    assert!(
        d1_score < 0.5,
        "d1 should have moved off its old direction; score={d1_score}"
    );
}

/// Card #984: a filtered search over a graph follows the filter in the
/// graph's walk and checks what the filter names on projections kept
/// between queries. Its answer must be the exact one on easy data, and a
/// document written since must be read as it is now, by an update one at a
/// time as by a batch.
#[test]
fn a_filtered_search_over_a_graph_sees_writes() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let coll = engine.get_collection("items").unwrap();
    // 12,000 points on a circle: past the graph threshold, and the nearest
    // of a key are its neighbours by number.
    let n = 12_000;
    let point = |i: usize| {
        let a = i as f64 / n as f64 * std::f64::consts::TAU;
        vec![a.cos(), a.sin(), 0.5]
    };
    let docs: Vec<_> = (0..n)
        .map(|i| json!({"_key": format!("p{i}"), "embedding": point(i), "cat": i % 10}))
        .collect();
    coll.insert_batch(docs).unwrap();
    coll.create_vector_index(VectorIndexConfig::new(
        "emb".to_string(),
        "embedding".to_string(),
        3,
    ))
    .unwrap();
    let query: Vec<f32> = point(5_000).iter().map(|x| *x as f32).collect();
    let filter = |cat: i64| {
        let mut f = serde_json::Map::new();
        f.insert("cat".to_string(), json!(cat));
        f
    };
    let keys = |found: &[(serde_json::Value, f32)]| -> Vec<String> {
        found
            .iter()
            .map(|(doc, _)| doc["_key"].as_str().unwrap().to_string())
            .collect()
    };
    let found = coll
        .vector_search_filtered("emb", &query, 3, 4, Some(40), &filter(3))
        .unwrap();
    let mut got = keys(&found);
    got.sort();
    assert_eq!(got, vec!["p4993", "p5003", "p5013"]);
    assert!(found.iter().all(|(doc, _)| doc["cat"] == json!(3)));

    // p5000 itself now passes, p5003 no longer does.
    coll.update("p5000", json!({"cat": 3})).unwrap();
    coll.update("p5003", json!({"cat": 4})).unwrap();
    let mut got = keys(
        &coll
            .vector_search_filtered("emb", &query, 3, 4, Some(40), &filter(3))
            .unwrap(),
    );
    got.sort();
    assert_eq!(got, vec!["p4993", "p5000", "p5013"]);

    // The same through a batch update.
    coll.update_batch(&[
        ("p5001".to_string(), json!({"cat": 3})),
        ("p5013".to_string(), json!({"cat": 7})),
    ])
    .unwrap();
    let mut got = keys(
        &coll
            .vector_search_filtered("emb", &query, 3, 4, Some(40), &filter(3))
            .unwrap(),
    );
    got.sort();
    assert_eq!(got, vec!["p4993", "p5000", "p5001"]);

    // And a delete.
    coll.delete("p5000").unwrap();
    let mut got = keys(
        &coll
            .vector_search_filtered("emb", &query, 3, 4, Some(40), &filter(3))
            .unwrap(),
    );
    got.sort();
    assert_eq!(got, vec!["p4983", "p4993", "p5001"]);
}

// ============================================================================
// Quantized graph (card #996)
// ============================================================================

/// An index created with `quantization: scalar` quantizes at creation and,
/// past the graph threshold, searches the graph's codes; what a search
/// returns is re-scored against the vectors in the documents, so the
/// scores are exact. The filtered walk is re-scored the same way.
#[test]
fn test_quantized_index_rescored_from_documents() {
    use rand::{Rng, SeedableRng};
    use solidb::storage::VectorQuantization;

    let (engine, _tmp) = create_test_engine();
    engine.create_collection("emb".to_string(), None).unwrap();
    let collection = engine.get_collection("emb").unwrap();

    let dim = 16;
    let mut rng = rand::rngs::StdRng::seed_from_u64(11);
    let vectors: Vec<Vec<f32>> = (0..10_500)
        .map(|_| (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect())
        .collect();
    let docs: Vec<serde_json::Value> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| json!({"_key": format!("d{i}"), "embedding": v, "group": i % 2}))
        .collect();
    collection.insert_batch(docs).unwrap();

    let config = VectorIndexConfig::new("idx".to_string(), "embedding".to_string(), dim)
        .with_quantization(VectorQuantization::Scalar);
    let stats = collection.create_vector_index(config).unwrap();
    assert_eq!(stats.quantization, VectorQuantization::Scalar);
    assert_eq!(stats.indexed_vectors, 10_500);
    assert!((stats.compression_ratio - 4.0).abs() < 1e-6, "{stats:?}");

    let cosine = |a: &[f32], b: &[f32]| -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (na * nb)
    };
    for i in [0usize, 17, 4_242, 10_499] {
        let found = collection
            .vector_search("idx", &vectors[i], 5, None)
            .unwrap();
        assert_eq!(found.len(), 5);
        assert_eq!(
            found[0].doc_key,
            format!("d{i}"),
            "a vector finds itself first"
        );
        for r in &found {
            let j: usize = r.doc_key[1..].parse().unwrap();
            let exact = cosine(&vectors[i], &vectors[j]);
            assert!((r.score - exact).abs() < 1e-5, "{} vs {exact}", r.score);
        }
        assert!(found.windows(2).all(|w| w[0].score >= w[1].score));
    }

    let mut filter = serde_json::Map::new();
    filter.insert("group".to_string(), json!(1));
    let found = collection
        .vector_search_filtered("idx", &vectors[17], 5, 4, None, &filter)
        .unwrap();
    assert_eq!(found.len(), 5);
    assert_eq!(found[0].0["_key"], json!("d17"));
    for (doc, score) in &found {
        assert_eq!(doc["group"], json!(1));
        let j: usize = doc["_key"].as_str().unwrap()[1..].parse().unwrap();
        assert!((score - cosine(&vectors[17], &vectors[j])).abs() < 1e-5);
    }
}

/// Binary quantization is withdrawn (card #1008): it lost too many of the
/// nearest neighbours on a large collection. Creating an index with it, or
/// quantizing one as it, is refused and says what to use instead.
#[test]
fn test_binary_quantization_is_refused() {
    use solidb::storage::VectorQuantization;

    let (engine, _tmp) = create_test_engine();
    engine.create_collection("bin".to_string(), None).unwrap();
    let collection = engine.get_collection("bin").unwrap();
    collection
        .insert(json!({"_key": "a", "embedding": [1.0, 0.0, 0.0]}))
        .unwrap();
    let config = VectorIndexConfig::new("idx".to_string(), "embedding".to_string(), 3)
        .with_quantization(VectorQuantization::Binary);
    let err = collection
        .create_vector_index(config)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("binary quantization is not available"),
        "{err}"
    );
    assert!(err.contains("'pq'"), "{err}");
    assert!(
        collection.get_vector_index("idx").is_err(),
        "nothing was created"
    );

    collection
        .create_vector_index(VectorIndexConfig::new(
            "plain".to_string(),
            "embedding".to_string(),
            3,
        ))
        .unwrap();
    let err = collection
        .quantize_vector_index("plain", VectorQuantization::Binary)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("binary quantization is not available"),
        "{err}"
    );
}

/// Product quantization (card #1000): created with `quantization: pq`, the
/// graph keeps a byte a sub-vector of 4 dimensions; searches re-scored
/// from the documents report exact scores and find each vector first.
#[test]
fn test_pq_index_rescored_from_documents() {
    use rand::{Rng, SeedableRng};
    use solidb::storage::VectorQuantization;

    let (engine, _tmp) = create_test_engine();
    engine.create_collection("pq".to_string(), None).unwrap();
    let collection = engine.get_collection("pq").unwrap();
    let dim = 64;
    let mut rng = rand::rngs::StdRng::seed_from_u64(9);
    let vectors: Vec<Vec<f32>> = (0..10_500)
        .map(|_| (0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect())
        .collect();
    let docs: Vec<serde_json::Value> = vectors
        .iter()
        .enumerate()
        .map(|(i, v)| json!({"_key": format!("d{i}"), "embedding": v}))
        .collect();
    collection.insert_batch(docs).unwrap();
    let config = VectorIndexConfig::new("idx".to_string(), "embedding".to_string(), dim)
        .with_quantization(VectorQuantization::Pq);
    let stats = collection.create_vector_index(config).unwrap();
    assert_eq!(stats.quantization, VectorQuantization::Pq);
    assert!((stats.compression_ratio - 16.0).abs() < 1e-6, "{stats:?}");
    let cosine = |a: &[f32], b: &[f32]| -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        dot / (na * nb)
    };
    for i in [0usize, 1_234, 9_876] {
        let found = collection
            .vector_search("idx", &vectors[i], 5, None)
            .unwrap();
        assert_eq!(found[0].doc_key, format!("d{i}"));
        for r in &found {
            let j: usize = r.doc_key[1..].parse().unwrap();
            assert!((r.score - cosine(&vectors[i], &vectors[j])).abs() < 1e-5);
        }
    }
}
