//! Multivector indexes (card #1005): a field holding a list of vectors (one
//! a token for ColBERT, one a patch for ColPali), each a node of the graph,
//! searched with a list of query vectors and scored by MaxSim, the sum over
//! the query vectors of the best match among the document's.

use rand::{Rng, SeedableRng};
use serde_json::{json, Value};
use solidb::storage::{StorageEngine, VectorIndexConfig};
use std::collections::HashMap;
use tempfile::TempDir;

mod common;
use common::{create_test_engine, execute_with_binds};

fn normalized(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (na * nb)
}

/// The exact MaxSim of `query` against `doc` (cosine).
fn max_sim(query: &[Vec<f32>], doc: &[Vec<f32>]) -> f32 {
    query
        .iter()
        .map(|q| doc.iter().map(|d| cosine(q, d)).fold(f32::MIN, f32::max))
        .sum()
}

/// `docs` documents of 6 to 18 token vectors each, drawn around two of 60
/// topics, and `queries` queries of 4 token vectors around one topic.
#[allow(clippy::type_complexity)]
fn corpus(docs: usize, queries: usize, dim: usize) -> (Vec<Vec<Vec<f32>>>, Vec<Vec<Vec<f32>>>) {
    let mut rng = rand::rngs::StdRng::seed_from_u64(1005);
    let topics: Vec<Vec<f32>> = (0..60)
        .map(|_| normalized((0..dim).map(|_| rng.gen_range(-1.0f32..1.0)).collect()))
        .collect();
    let token = |rng: &mut rand::rngs::StdRng, topic: usize| -> Vec<f32> {
        normalized(
            topics[topic]
                .iter()
                .map(|x| x + rng.gen_range(-0.35f32..0.35))
                .collect(),
        )
    };
    let documents = (0..docs)
        .map(|_| {
            let (a, b) = (rng.gen_range(0..60), rng.gen_range(0..60));
            let n = rng.gen_range(6..=18);
            (0..n)
                .map(|i| token(&mut rng, if i % 3 == 0 { b } else { a }))
                .collect()
        })
        .collect();
    let questions = (0..queries)
        .map(|_| {
            let topic = rng.gen_range(0..60);
            (0..4).map(|_| token(&mut rng, topic)).collect()
        })
        .collect();
    (documents, questions)
}

fn load(engine: &StorageEngine, name: &str, docs: &[Vec<Vec<f32>>]) {
    engine.create_collection(name.to_string(), None).unwrap();
    let collection = engine.get_collection(name).unwrap();
    let batch: Vec<Value> = docs
        .iter()
        .enumerate()
        .map(|(i, tokens)| json!({"_key": format!("d{i}"), "tokens": tokens, "n": i % 2}))
        .collect();
    collection.insert_batch(batch).unwrap();
}

fn multivector_config(dim: usize) -> VectorIndexConfig {
    VectorIndexConfig::new("mv".to_string(), "tokens".to_string(), dim).with_multivector(true)
}

/// On 1,500 documents (17,800 token vectors, so a graph), the top 10 by
/// MaxSim are found against an exhaustive MaxSim, with exact scores.
#[test]
fn multivector_search_finds_the_maxsim_top_10() {
    let (engine, _tmp) = create_test_engine();
    let dim = 32;
    let (docs, queries) = corpus(1_500, 20, dim);
    load(&engine, "pages", &docs);
    let collection = engine.get_collection("pages").unwrap();
    let stats = collection
        .create_vector_index(multivector_config(dim))
        .unwrap();
    let tokens: usize = docs.iter().map(Vec::len).sum();
    assert_eq!(stats.indexed_vectors, tokens, "one node a token vector");
    assert!(tokens > 10_000, "the graph is built: {tokens}");

    let mut hits = 0;
    for query in &queries {
        let mut truth: Vec<(usize, f32)> = docs
            .iter()
            .enumerate()
            .map(|(i, d)| (i, max_sim(query, d)))
            .collect();
        truth.sort_by(|a, b| b.1.total_cmp(&a.1));
        let truth: Vec<String> = truth[..10].iter().map(|(i, _)| format!("d{i}")).collect();

        let flat: Vec<f32> = query.concat();
        let found = collection.vector_search("mv", &flat, 10, None).unwrap();
        assert_eq!(found.len(), 10);
        for r in &found {
            let i: usize = r.doc_key[1..].parse().unwrap();
            let exact = max_sim(query, &docs[i]);
            assert!((r.score - exact).abs() < 1e-4, "{} vs {exact}", r.score);
        }
        hits += found.iter().filter(|r| truth.contains(&r.doc_key)).count();
    }
    let recall = hits as f64 / (10 * queries.len()) as f64;
    assert!(recall >= 0.95, "recall@10 {recall}");
}

/// A document's vectors follow its writes: replaced whole on update (it
/// may hold fewer), gone on delete; a filtered search filters documents.
#[test]
fn multivector_index_follows_writes_and_filters() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("c".to_string(), None).unwrap();
    let collection = engine.get_collection("c").unwrap();
    collection
        .create_vector_index(multivector_config(3))
        .unwrap();
    collection
        .insert(json!({"_key": "a", "tag": "x", "tokens": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]}))
        .unwrap();
    collection
        .insert(json!({"_key": "b", "tag": "y", "tokens": [[0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]]}))
        .unwrap();
    // A single vector, or none, is not a multivector field.
    collection
        .insert(json!({"_key": "c", "tokens": [1.0, 0.0, 0.0]}))
        .unwrap();
    let index = collection.get_vector_index("mv").unwrap();
    assert_eq!(index.len(), 5);

    let query = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
    let found = collection.vector_search("mv", &query, 5, None).unwrap();
    let keys: Vec<&str> = found.iter().map(|r| r.doc_key.as_str()).collect();
    assert_eq!(keys, ["b", "a"], "{found:?}");
    // b: best for [1,0,0] is [1,1,0] (0.707), for [0,0,1] itself (1.0).
    assert!((found[0].score - (1.0 + std::f32::consts::FRAC_1_SQRT_2)).abs() < 1e-5);

    // Fewer vectors after an update: the old ones are gone.
    collection
        .update("b", json!({"tokens": [[0.0, 1.0, 0.0]]}))
        .unwrap();
    assert_eq!(index.len(), 3);
    let found = collection.vector_search("mv", &query, 5, None).unwrap();
    assert_eq!(found[0].doc_key, "a");
    assert!((found[0].score - 1.0).abs() < 1e-5, "{found:?}");
    // An update that leaves the vectors alone keeps them.
    collection.update("b", json!({"tag": "z"})).unwrap();
    assert_eq!(index.len(), 3);

    let filter = serde_json::Map::from_iter([("tag".to_string(), json!("z"))]);
    let filtered = collection
        .vector_search_filtered("mv", &query, 5, 1, None, &filter)
        .unwrap();
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].0["_key"], json!("b"));

    collection.delete("a").unwrap();
    assert_eq!(index.len(), 1);
    let found = collection.vector_search("mv", &query, 5, None).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].doc_key, "b");

    // A query whose length is not a whole number of vectors.
    let err = collection
        .vector_search("mv", &[1.0, 0.0], 5, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("query vectors of 3 dimensions"), "{err}");
}

/// SDBQL takes a list of query vectors; HTTP's `vectors` likewise (both
/// end to end into the same search).
#[test]
fn vector_search_in_sdbql_takes_a_list_of_query_vectors() {
    let (engine, _tmp) = create_test_engine();
    engine.create_collection("c".to_string(), None).unwrap();
    let collection = engine.get_collection("c").unwrap();
    collection
        .create_vector_index(multivector_config(3))
        .unwrap();
    collection
        .insert(json!({"_key": "a", "tokens": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]}))
        .unwrap();
    collection
        .insert(json!({"_key": "b", "tokens": [[0.0, 0.0, 1.0]]}))
        .unwrap();
    let binds = HashMap::from([("q".to_string(), json!([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]))]);
    let rows = execute_with_binds(
        &engine,
        "FOR r IN VECTOR_SEARCH('c', 'mv', @q, 2) RETURN {key: r.doc._key, score: r.score}",
        binds,
    );
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0]["key"], json!("a"));
    assert!((rows[0]["score"].as_f64().unwrap() - 2.0).abs() < 1e-5);
}

/// The index says it is multivector after a restart (V6 keeps the whole
/// config), and refuses what it cannot answer.
#[test]
fn multivector_survives_a_restart_and_refuses_texts() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().to_str().unwrap().to_string();
    {
        let engine = StorageEngine::new(&path).unwrap();
        engine.create_collection("c".to_string(), None).unwrap();
        let collection = engine.get_collection("c").unwrap();
        collection
            .create_vector_index(multivector_config(3))
            .unwrap();
        collection
            .insert(json!({"_key": "a", "tokens": [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]}))
            .unwrap();
        collection.flush_vector_indexes();

        // Text in, one vector out: not what a multivector index holds.
        let err = collection
            .create_vector_index(
                VectorIndexConfig::new("auto".to_string(), "t".to_string(), 3)
                    .with_multivector(true)
                    .with_embedding_source("text".to_string()),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("multivector"), "{err}");
        let config = collection
            .get_all_vector_index_configs()
            .into_iter()
            .find(|c| c.name == "mv")
            .unwrap();
        let err = solidb::storage::collection::vector::embed_query_text(
            &engine, "_system", &config, "hi",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("is multivector"), "{err}");
    }
    let engine = StorageEngine::new(&path).unwrap();
    let collection = engine.get_collection("c").unwrap();
    let index = collection.get_vector_index("mv").unwrap();
    assert!(index.config().multivector);
    assert_eq!(index.len(), 2);
    let found = collection
        .vector_search("mv", &[0.0, 1.0, 0.0], 1, None)
        .unwrap();
    assert_eq!(found[0].doc_key, "a");
    assert!((found[0].score - 1.0).abs() < 1e-5);
}
