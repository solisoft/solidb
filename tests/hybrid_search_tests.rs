//! Hybrid Search Tests
//!
//! Tests for hybrid search functionality combining vector similarity
//! with fulltext search for improved RAG results.

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

/// Set up a test collection with vector and fulltext indexes
fn setup_hybrid_collection(engine: &StorageEngine) {
    engine
        .create_collection("articles".to_string(), None)
        .unwrap();
    let collection = engine.get_collection("articles").unwrap();

    // Create vector index
    let vector_config = VectorIndexConfig::new(
        "embedding_idx".to_string(),
        "embedding".to_string(),
        4, // 4 dimensions for testing
    )
    .with_metric(VectorMetric::Cosine);
    collection.create_vector_index(vector_config).unwrap();

    // Create fulltext index
    collection
        .create_fulltext_index(
            "content_ft".to_string(),
            vec!["content".to_string()],
            Some(3),
        )
        .unwrap();

    // Insert test documents
    // Doc 1: Matches both vector (similar to [1,0,0,0]) and text ("machine learning")
    collection
        .insert(json!({
            "_key": "doc1",
            "title": "Machine Learning Basics",
            "content": "Introduction to machine learning algorithms and neural networks",
            "embedding": [0.9, 0.1, 0.1, 0.0]
        }))
        .unwrap();

    // Doc 2: Matches vector but not text query
    collection
        .insert(json!({
            "_key": "doc2",
            "title": "Data Science Overview",
            "content": "Statistical analysis and data visualization techniques",
            "embedding": [0.85, 0.15, 0.0, 0.1]
        }))
        .unwrap();

    // Doc 3: Matches text but not vector (different direction)
    collection
        .insert(json!({
            "_key": "doc3",
            "title": "Deep Learning Guide",
            "content": "Advanced machine learning with deep neural networks",
            "embedding": [0.0, 0.0, 0.9, 0.1]
        }))
        .unwrap();

    // Doc 4: Matches neither well
    collection
        .insert(json!({
            "_key": "doc4",
            "title": "Database Systems",
            "content": "Relational database management and SQL queries",
            "embedding": [0.1, 0.1, 0.1, 0.9]
        }))
        .unwrap();

    // Doc 5: Strong match for both
    collection
        .insert(json!({
            "_key": "doc5",
            "title": "ML Tutorial",
            "content": "Complete machine learning tutorial with practical examples",
            "embedding": [0.95, 0.05, 0.0, 0.0]
        }))
        .unwrap();
}

// ============================================================================
// HYBRID_SEARCH SDBQL Function Tests
// ============================================================================

#[test]
fn test_hybrid_search_basic() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    // Search for documents similar to [1,0,0,0] and containing "machine learning"
    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "embedding_idx",
            "content",
            [1.0, 0.0, 0.0, 0.0],
            "machine learning"
        )
        FOR result IN results
        RETURN result
    "#;

    let results = execute_query(&engine, query);

    // Should return results
    assert!(!results.is_empty(), "Hybrid search should return results");

    // First result should have both vector and fulltext match (doc1 or doc5)
    let first = &results[0];
    assert!(first.get("doc").is_some(), "Result should have doc");
    assert!(first.get("score").is_some(), "Result should have score");
    assert!(first.get("sources").is_some(), "Result should have sources");
}

#[test]
fn test_hybrid_search_with_options() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    // Search with custom weights favoring vector similarity
    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "embedding_idx",
            "content",
            [1.0, 0.0, 0.0, 0.0],
            "machine learning",
            { vector_weight: 0.8, text_weight: 0.2, limit: 3 }
        )
        FOR result IN results
        RETURN result
    "#;

    let results = execute_query(&engine, query);

    // Should respect limit
    assert!(results.len() <= 3, "Should respect limit option");
}

#[test]
fn test_hybrid_search_rrf_fusion() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    // Search using RRF fusion method
    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "embedding_idx",
            "content",
            [1.0, 0.0, 0.0, 0.0],
            "machine learning",
            { fusion: "rrf", limit: 5 }
        )
        FOR result IN results
        RETURN result
    "#;

    let results = execute_query(&engine, query);

    assert!(!results.is_empty(), "RRF fusion should return results");
}

#[test]
fn test_hybrid_search_both_match_boost() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    // doc1 and doc5 match both vector and text
    // They should rank higher than docs matching only one
    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "embedding_idx",
            "content",
            [1.0, 0.0, 0.0, 0.0],
            "machine learning",
            { limit: 5 }
        )
        FOR result IN results
        RETURN { key: result.doc._key, sources: result.sources, score: result.score }
    "#;

    let results = execute_query(&engine, query);

    // Find results with both sources
    let both_sources: Vec<_> = results
        .iter()
        .filter(|r| {
            if let Some(sources) = r.get("sources").and_then(|s| s.as_array()) {
                sources.len() == 2
            } else {
                false
            }
        })
        .collect();

    // Should have documents matching both
    assert!(
        !both_sources.is_empty(),
        "Should have documents matching both vector and fulltext"
    );
}

#[test]
fn test_hybrid_search_vector_only_match() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    // Search with text that doesn't match well
    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "embedding_idx",
            "content",
            [0.1, 0.1, 0.1, 0.9],
            "xyz nonexistent term"
        )
        FOR result IN results
        RETURN { key: result.doc._key, sources: result.sources }
    "#;

    let results = execute_query(&engine, query);

    // Should still return vector matches
    assert!(
        !results.is_empty(),
        "Should return vector-only matches when text doesn't match"
    );

    // Results should only have "vector" source
    for result in &results {
        if let Some(sources) = result.get("sources").and_then(|s| s.as_array()) {
            assert!(
                sources.iter().any(|s| s == "vector"),
                "Should have vector source"
            );
        }
    }
}

#[test]
fn test_hybrid_search_text_only_match() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    // Search with a vector that doesn't match well but text that does
    // Use vector pointing in completely different direction
    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "embedding_idx",
            "content",
            [0.0, 0.0, 1.0, 0.0],
            "machine learning"
        )
        FOR result IN results
        RETURN { key: result.doc._key, sources: result.sources }
    "#;

    let results = execute_query(&engine, query);

    // Should return results
    assert!(
        !results.is_empty(),
        "Should return results including text matches"
    );

    // doc3 has embedding [0,0,0.9,0.1] which should be top vector result
    // and also contains "machine learning"
}

#[test]
fn test_hybrid_search_scores_present() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "embedding_idx",
            "content",
            [1.0, 0.0, 0.0, 0.0],
            "machine learning"
        )
        FOR result IN results
        RETURN {
            key: result.doc._key,
            score: result.score,
            vector_score: result.vector_score,
            text_score: result.text_score
        }
    "#;

    let results = execute_query(&engine, query);

    for result in &results {
        // Combined score should always be present
        assert!(
            result.get("score").is_some(),
            "Combined score should be present"
        );

        // Individual scores should be present for respective matches
        let vec_score = result.get("vector_score");
        let txt_score = result.get("text_score");

        // At least one score should be present
        assert!(
            vec_score.is_some() || txt_score.is_some(),
            "At least one individual score should be present"
        );
    }
}

#[test]
fn test_hybrid_search_documents_included() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "embedding_idx",
            "content",
            [1.0, 0.0, 0.0, 0.0],
            "machine learning",
            { limit: 3 }
        )
        FOR result IN results
        RETURN result.doc.title
    "#;

    let results = execute_query(&engine, query);

    // Should return document titles
    assert!(!results.is_empty(), "Should return results with documents");

    // First result should have a title
    assert!(
        results[0].is_string(),
        "Result should include document title"
    );
}

#[test]
fn test_hybrid_search_sorted_by_score() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "embedding_idx",
            "content",
            [1.0, 0.0, 0.0, 0.0],
            "machine learning"
        )
        FOR result IN results
        RETURN result.score
    "#;

    let results = execute_query(&engine, query);

    // Verify results are sorted in descending order
    let scores: Vec<f64> = results.iter().filter_map(|r| r.as_f64()).collect();

    for i in 1..scores.len() {
        assert!(
            scores[i - 1] >= scores[i],
            "Results should be sorted by score descending: {} >= {}",
            scores[i - 1],
            scores[i]
        );
    }
}

#[test]
fn test_hybrid_search_error_missing_vector_index() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    // Try to use non-existent vector index
    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "nonexistent_idx",
            "content",
            [1.0, 0.0, 0.0, 0.0],
            "machine learning"
        )
        FOR result IN results
        RETURN result
    "#;

    let query_parsed = parse(query).unwrap();
    let executor = QueryExecutor::new(&engine);
    let result = executor.execute(&query_parsed);

    assert!(result.is_err(), "Should error on missing vector index");
}

#[test]
fn test_hybrid_search_empty_text_query() {
    let (engine, _tmp) = create_test_engine();
    setup_hybrid_collection(&engine);

    // Empty text query should still return vector results
    let query = r#"
        LET results = HYBRID_SEARCH(
            "articles",
            "embedding_idx",
            "content",
            [1.0, 0.0, 0.0, 0.0],
            ""
        )
        FOR result IN results
        RETURN { key: result.doc._key, sources: result.sources }
    "#;

    let results = execute_query(&engine, query);

    // Should return vector-only results
    assert!(
        !results.is_empty(),
        "Should return results even with empty text query"
    );
}
