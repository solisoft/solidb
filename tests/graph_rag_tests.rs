//! Graph RAG Integration Tests
//!
//! End-to-end tests for NEIGHBORS, GRAPH_RAG, COMMUNITY_SEARCH SDBQL
//! functions and the community-build HTTP endpoints.

mod common;

use common::{
    create_test_engine, execute_query, execute_query_expect_err, execute_single,
    execute_with_binds, execute_with_db_and_binds,
};
use serde_json::json;
use solidb::storage::{StorageEngine, VectorIndexConfig, VectorMetric};
use solidb::BindVars;

/// Small knowledge graph matching the docs example:
///   docs/a ──► docs/b
///   seeds: a (1.0), d (0.994) for query vector [1,0,0,0]
fn setup_graph_rag_corpus(engine: &StorageEngine) {
    engine.create_collection("docs".to_string(), None).unwrap();
    let docs = engine.get_collection("docs").unwrap();

    let vector_config = VectorIndexConfig::new("emb".to_string(), "embedding".to_string(), 4)
        .with_metric(VectorMetric::Cosine);
    docs.create_vector_index(vector_config).unwrap();

    docs.insert(json!({
        "_key": "a",
        "title": "Doc A",
        "embedding": [1.0, 0.0, 0.0, 0.0]
    }))
    .unwrap();
    docs.insert(json!({
        "_key": "b",
        "title": "Doc B",
        "embedding": [0.5, 0.5, 0.0, 0.0]
    }))
    .unwrap();
    docs.insert(json!({
        "_key": "d",
        "title": "Doc D",
        "embedding": [0.994, 0.006, 0.0, 0.0]
    }))
    .unwrap();

    engine
        .create_collection("links".to_string(), Some("edge".to_string()))
        .unwrap();
    let links = engine.get_collection("links").unwrap();
    links
        .insert(json!({"_from": "docs/a", "_to": "docs/b"}))
        .unwrap();
}

/// The same corpus, but inside a named database.
///
/// This is the shape every HTTP / driver request takes, and it is *not*
/// equivalent to the engine-level setup above: a database-scoped collection's
/// column family — and therefore every document's `_id` — is prefixed with the
/// database name (`"grdb:docs/a"`), while edges reference vertices unprefixed
/// (`"docs/a"`). Graph RAG must key off `doc_key`, never off `_id`.
const GRAPH_DB: &str = "grdb";

fn setup_graph_rag_db(engine: &StorageEngine) {
    engine.create_database(GRAPH_DB.to_string()).unwrap();
    let db = engine.get_database(GRAPH_DB).unwrap();

    db.create_collection("docs".to_string(), None).unwrap();
    let docs = db.get_collection("docs").unwrap();
    docs.create_vector_index(
        VectorIndexConfig::new("emb".to_string(), "embedding".to_string(), 4)
            .with_metric(VectorMetric::Cosine),
    )
    .unwrap();
    docs.create_fulltext_index("text_ft".to_string(), vec!["text".to_string()], Some(3))
        .unwrap();

    docs.insert(json!({
        "_key": "a", "title": "Doc A",
        "embedding": [1.0, 0.0, 0.0, 0.0],
        "text": "vector database introduction"
    }))
    .unwrap();
    docs.insert(json!({
        "_key": "b", "title": "Doc B",
        "embedding": [0.5, 0.5, 0.0, 0.0]
    }))
    .unwrap();
    docs.insert(json!({
        "_key": "d", "title": "Doc D",
        "embedding": [0.994, 0.006, 0.0, 0.0],
        "text": "embedding search with vector database"
    }))
    .unwrap();

    db.create_collection("links".to_string(), Some("edge".to_string()))
        .unwrap();
    db.get_collection("links")
        .unwrap()
        .insert(json!({"_from": "docs/a", "_to": "docs/b"}))
        .unwrap();
}

fn setup_community_summaries(engine: &StorageEngine) {
    engine
        .create_collection("_community_summaries".to_string(), None)
        .unwrap();
    let summaries = engine.get_collection("_community_summaries").unwrap();
    summaries
        .insert(json!({
            "_key": "run1:0",
            "run_id": "run1",
            "edge_collection": "links",
            "community_id": 0,
            "title": "Vector DB",
            "summary": "A community about vector database and embedding search",
            "keywords": ["vector", "database", "embedding"],
            "size": 3
        }))
        .unwrap();
    summaries
        .insert(json!({
            "_key": "run1:1",
            "run_id": "run1",
            "edge_collection": "links",
            "community_id": 1,
            "title": "Unrelated",
            "summary": "Cooking recipes and kitchen tips",
            "keywords": ["cooking", "recipes"],
            "size": 2
        }))
        .unwrap();

    engine
        .create_collection("_graph_runs".to_string(), None)
        .unwrap();
    let runs = engine.get_collection("_graph_runs").unwrap();
    runs.insert(json!({
        "_key": "links",
        "latest_run_id": "run1",
        "communities_found": 2
    }))
    .unwrap();
}

// ============================================================================
// NEIGHBORS
// ============================================================================

#[test]
fn test_neighbors_expands_from_seeds() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_corpus(&engine);

    let results = execute_query(
        &engine,
        r#"
        LET res = NEIGHBORS("links", ["docs/a"], { hops: 1, direction: "outbound" })
        FOR r IN res
          RETURN { id: r.id, hops: r.hops, seed: r.seed }
        "#,
    );

    assert_eq!(results.len(), 2);
    let ids: Vec<&str> = results
        .iter()
        .filter_map(|v| v.get("id").and_then(|x| x.as_str()))
        .collect();
    assert!(ids.contains(&"docs/a"));
    assert!(ids.contains(&"docs/b"));

    let seed_hit = results
        .iter()
        .find(|v| v.get("id").and_then(|x| x.as_str()) == Some("docs/a"))
        .unwrap();
    assert_eq!(seed_hit.get("hops").and_then(|v| v.as_u64()), Some(0));
    assert_eq!(seed_hit.get("seed").and_then(|v| v.as_bool()), Some(true));

    let neighbor = results
        .iter()
        .find(|v| v.get("id").and_then(|x| x.as_str()) == Some("docs/b"))
        .unwrap();
    assert_eq!(neighbor.get("hops").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(neighbor.get("seed").and_then(|v| v.as_bool()), Some(false));
}

#[test]
fn test_neighbors_hydrates_documents() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_corpus(&engine);

    let result = execute_single(
        &engine,
        r#"
        LET res = NEIGHBORS("links", ["docs/a"], { hops: 1, limit: 5 })
        FOR r IN res
          FILTER r.id == "docs/b"
          RETURN r.doc.title
        "#,
    );
    assert_eq!(result, json!("Doc B"));
}

/// `NEIGHBORS` takes a collection *name* from the query and SDBQL functions
/// carry only Read permission, so it must never persist `_from`/`_to` indexes
/// on a collection that isn't an edge collection.
#[test]
fn test_neighbors_does_not_index_non_edge_collections() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("plain".to_string(), None).unwrap();
    let plain = engine.get_collection("plain").unwrap();
    plain.insert(json!({"_key": "x", "a": 1})).unwrap();

    let _ = execute_query(
        &engine,
        r#"RETURN LENGTH(NEIGHBORS("plain", ["plain/x"], { hops: 1, direction: "any" }))"#,
    );

    assert!(
        engine
            .get_collection("plain")
            .unwrap()
            .list_indexes()
            .is_empty(),
        "a read-only NEIGHBORS() call persisted indexes on a document collection"
    );
}

/// An edge collection still gets its traversal indexes lazily.
#[test]
fn test_neighbors_auto_indexes_edge_collections() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_corpus(&engine);

    let _ = execute_query(
        &engine,
        r#"RETURN LENGTH(NEIGHBORS("links", ["docs/a"], { hops: 1 }))"#,
    );

    let names: Vec<String> = engine
        .get_collection("links")
        .unwrap()
        .list_indexes()
        .into_iter()
        .map(|i| i.name)
        .collect();
    assert!(names.contains(&"_edge_from_idx".to_string()), "{:?}", names);
}

/// `limit` bounds the documents actually returned, so a dangling edge target
/// must not silently consume one of the slots.
#[test]
fn test_neighbors_limit_counts_hydrated_hits() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("v".to_string(), None).unwrap();
    let v = engine.get_collection("v").unwrap();
    v.insert(json!({"_key": "a"})).unwrap();
    v.insert(json!({"_key": "b"})).unwrap();
    engine
        .create_collection("e".to_string(), Some("edge".to_string()))
        .unwrap();
    let e = engine.get_collection("e").unwrap();
    // "v/aaa" has no document; it sorts before "v/b" at the same score.
    e.insert(json!({"_from": "v/a", "_to": "v/aaa"})).unwrap();
    e.insert(json!({"_from": "v/a", "_to": "v/b"})).unwrap();

    let results = execute_query(
        &engine,
        r#"
        LET res = NEIGHBORS("e", ["v/a"], { hops: 1, limit: 2 })
        FOR r IN res
          RETURN r.id
        "#,
    );
    assert_eq!(results, vec![json!("v/a"), json!("v/b")]);
}

/// `include_seeds: false` drops seeds entirely — including a seed that another
/// seed happens to reach.
#[test]
fn test_neighbors_exclude_seeds_drops_cross_reached_seeds() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_corpus(&engine);

    let results = execute_query(
        &engine,
        r#"
        LET res = NEIGHBORS("links", ["docs/a", "docs/b"], { hops: 1, include_seeds: false })
        FOR r IN res
          RETURN r.id
        "#,
    );
    // docs/b is a seed and is reached from docs/a; neither may appear.
    assert!(results.is_empty(), "got {:?}", results);
}

// ============================================================================
// Option validation
// ============================================================================

#[test]
fn test_unknown_option_is_rejected() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_corpus(&engine);
    let err = execute_query_expect_err(
        &engine,
        r#"RETURN NEIGHBORS("links", ["docs/a"], { seedLimit: 5 })"#,
    );
    assert!(err.contains("unknown option 'seedLimit'"), "got: {}", err);
}

#[test]
fn test_wrongly_typed_option_is_rejected() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_corpus(&engine);
    let err = execute_query_expect_err(
        &engine,
        r#"RETURN NEIGHBORS("links", ["docs/a"], { hops: "2" })"#,
    );
    assert!(err.contains("'hops' must be"), "got: {}", err);
}

#[test]
fn test_out_of_range_decay_is_rejected() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_corpus(&engine);
    let err = execute_query_expect_err(
        &engine,
        r#"RETURN NEIGHBORS("links", ["docs/a"], { decay: 0 })"#,
    );
    assert!(err.contains("decay must be in (0, 1]"), "got: {}", err);
}

// ============================================================================
// GRAPH_RAG
// ============================================================================

#[test]
fn test_graph_rag_vector_seeds_and_expansion() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_corpus(&engine);

    let mut binds = BindVars::new();
    binds.insert("query_vector".to_string(), json!([1.0, 0.0, 0.0, 0.0]));

    let results = execute_with_binds(
        &engine,
        r#"
        LET res = GRAPH_RAG("docs", "emb", "links", @query_vector,
                            { hops: 1, seed_limit: 2, direction: "outbound" })
        FOR r IN res
          RETURN { id: r.id, hops: r.hops, seed: r.seed, score: r.score }
        "#,
        binds,
    );

    assert!(results.len() >= 2);
    let ids: Vec<&str> = results
        .iter()
        .filter_map(|v| v.get("id").and_then(|x| x.as_str()))
        .collect();
    assert!(ids.contains(&"docs/a"));
    assert!(ids.contains(&"docs/d"));
    assert!(ids.contains(&"docs/b"));

    let seeds: Vec<_> = results
        .iter()
        .filter(|v| v.get("seed").and_then(|x| x.as_bool()) == Some(true))
        .collect();
    assert_eq!(seeds.len(), 2);
}

#[test]
fn test_graph_rag_empty_seeds_returns_empty() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_corpus(&engine);

    let results = execute_query(
        &engine,
        r#"
        LET res = GRAPH_RAG("docs", "emb", "links", [0.0, 0.0, 1.0, 0.0],
                            { seed_limit: 0, hops: 1 })
        RETURN LENGTH(res)
        "#,
    );
    assert_eq!(results, vec![json!(0)]);
}

/// Runs under a database context, where `_id` carries the `"grdb:"` column
/// family prefix. Seeding off `_id` here yields vertex ids that match no edge
/// and hydrate to nothing, so the whole result collapses to `[]`.
#[test]
fn test_graph_rag_hybrid_seed_mode() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_db(&engine);

    let mut binds = BindVars::new();
    binds.insert("query_vector".to_string(), json!([1.0, 0.0, 0.0, 0.0]));

    let results = execute_with_db_and_binds(
        &engine,
        GRAPH_DB,
        r#"
        LET res = GRAPH_RAG("docs", "emb", "links", @query_vector, {
          seed_mode: "hybrid",
          fulltext_field: "text",
          text_query: "vector database",
          seed_limit: 5,
          hops: 1
        })
        FOR r IN res
          RETURN r.id
        "#,
        binds,
    );

    let ids: Vec<&str> = results.iter().filter_map(|v| v.as_str()).collect();
    assert!(ids.contains(&"docs/a"), "seeds missing, got {:?}", ids);
    assert!(ids.contains(&"docs/d"), "seeds missing, got {:?}", ids);
    // docs/b is not a seed (no `text` field) but is one hop from docs/a.
    assert!(ids.contains(&"docs/b"), "expansion missing, got {:?}", ids);
}

/// The vector path must work the same way under a database context.
#[test]
fn test_graph_rag_vector_seed_mode_with_database_context() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_db(&engine);

    let mut binds = BindVars::new();
    binds.insert("query_vector".to_string(), json!([1.0, 0.0, 0.0, 0.0]));

    let results = execute_with_db_and_binds(
        &engine,
        GRAPH_DB,
        r#"
        LET res = GRAPH_RAG("docs", "emb", "links", @query_vector, { hops: 1, seed_limit: 2 })
        FOR r IN res
          RETURN r.id
        "#,
        binds,
    );

    let ids: Vec<&str> = results.iter().filter_map(|v| v.as_str()).collect();
    assert!(ids.contains(&"docs/a"), "got {:?}", ids);
    assert!(ids.contains(&"docs/b"), "got {:?}", ids);
}

/// `vector_search` reports an L2 *distance* for a euclidean index — smaller is
/// closer. Used as a seed weight verbatim it would rank the farthest document
/// first.
#[test]
fn test_graph_rag_euclidean_ranks_nearest_seed_first() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("euc".to_string(), None).unwrap();
    let euc = engine.get_collection("euc").unwrap();
    euc.create_vector_index(
        VectorIndexConfig::new("emb".to_string(), "embedding".to_string(), 2)
            .with_metric(VectorMetric::Euclidean),
    )
    .unwrap();
    euc.insert(json!({"_key": "near", "embedding": [0.0, 0.0]}))
        .unwrap();
    euc.insert(json!({"_key": "far", "embedding": [10.0, 0.0]}))
        .unwrap();
    engine
        .create_collection("euc_links".to_string(), Some("edge".to_string()))
        .unwrap();

    let results = execute_query(
        &engine,
        r#"
        LET res = GRAPH_RAG("euc", "emb", "euc_links", [0.0, 0.0], { hops: 0, seed_limit: 2 })
        FOR r IN res
          RETURN r.id
        "#,
    );
    assert_eq!(results[0], json!("euc/near"));
    assert_eq!(results[1], json!("euc/far"));
}

/// A negative cosine similarity is a real signal, not a zero: two seeds must
/// not collapse to the same score just because one points away from the query.
#[test]
fn test_graph_rag_negative_cosine_scores_are_ordered() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("cos".to_string(), None).unwrap();
    let cos = engine.get_collection("cos").unwrap();
    cos.create_vector_index(
        VectorIndexConfig::new("emb".to_string(), "embedding".to_string(), 2)
            .with_metric(VectorMetric::Cosine),
    )
    .unwrap();
    cos.insert(json!({"_key": "opposite", "embedding": [-1.0, 0.0]}))
        .unwrap();
    cos.insert(json!({"_key": "ortho", "embedding": [0.0, 1.0]}))
        .unwrap();
    engine
        .create_collection("cos_links".to_string(), Some("edge".to_string()))
        .unwrap();

    let results = execute_query(
        &engine,
        r#"
        LET res = GRAPH_RAG("cos", "emb", "cos_links", [1.0, 0.0], { hops: 0, seed_limit: 2 })
        FOR r IN res
          RETURN { id: r.id, score: r.score }
        "#,
    );
    assert_eq!(results[0].get("id").unwrap(), &json!("cos/ortho"));
    assert_eq!(results[1].get("id").unwrap(), &json!("cos/opposite"));
    let ortho = results[0].get("score").and_then(|v| v.as_f64()).unwrap();
    let opposite = results[1].get("score").and_then(|v| v.as_f64()).unwrap();
    assert!(
        ortho > opposite,
        "orthogonal must outrank opposite: {} vs {}",
        ortho,
        opposite
    );
}

#[test]
fn test_graph_rag_unknown_vector_index_errors() {
    let (engine, _tmp) = create_test_engine();
    setup_graph_rag_corpus(&engine);
    let err = execute_query_expect_err(
        &engine,
        r#"RETURN GRAPH_RAG("docs", "nope", "links", [1.0, 0.0, 0.0, 0.0], {})"#,
    );
    assert!(err.contains("unknown vector index"), "got: {}", err);
}

// ============================================================================
// COMMUNITY_SEARCH
// ============================================================================

#[test]
fn test_community_search_ranks_relevant_summaries() {
    let (engine, _tmp) = create_test_engine();
    setup_community_summaries(&engine);

    let results = execute_query(
        &engine,
        r#"
        LET res = COMMUNITY_SEARCH("vector database", { edge_collection: "links", limit: 3 })
        FOR c IN res
          RETURN { community_id: c.community_id, title: c.title, score: c.score }
        "#,
    );

    assert!(!results.is_empty());
    let top = &results[0];
    assert_eq!(top.get("title").and_then(|v| v.as_str()), Some("Vector DB"));
    let score = top.get("score").and_then(|v| v.as_f64()).unwrap_or(0.0);
    assert!(score > 0.0);

    if results.len() >= 2 {
        let first_score = results[0]
            .get("score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let second_score = results[1]
            .get("score")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        assert!(first_score >= second_score);
    }
}

#[test]
fn test_community_search_empty_when_no_summaries() {
    let (engine, _tmp) = create_test_engine();

    let results = execute_query(
        &engine,
        r#"
        RETURN LENGTH(COMMUNITY_SEARCH("anything", {}))
        "#,
    );
    assert_eq!(results, vec![json!(0)]);
}
