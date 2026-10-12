//! Fulltext ranking: BM25 over the index's own statistics.
//!
//! Up to 2.3.1 a hit scored +10 per occurrence of a query word and +5 per
//! word within two edits of one, with no weight for how rare a word is, so a
//! chunk full of "les", "des", "pour" outranked the one holding the word the
//! query was about. These tests pin the ranking, not the score's scale.

use serde_json::json;
use solidb::storage::{Collection, StorageEngine};
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn engine_with(docs: &[(&str, &str)]) -> (StorageEngine, Collection, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_collection("chunks".into(), None).unwrap();
    let chunks = engine.get_collection("chunks").unwrap();
    chunks
        .create_fulltext_index("ft_text".into(), vec!["text".into()], None)
        .unwrap();
    for (key, text) in docs {
        chunks.insert(json!({"_key": key, "text": text})).unwrap();
    }
    (engine, chunks, tmp)
}

fn keys(engine: &StorageEngine, q: &str) -> Vec<String> {
    let out = QueryExecutor::new(engine)
        .execute(&parse(q).unwrap())
        .unwrap_or_else(|e| panic!("{q}: {e}"));
    out[0]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["doc"]["_key"].as_str().unwrap().to_string())
        .collect()
}

/// Common words, however often they occur, weigh less than the rare word.
#[test]
fn rare_word_outranks_repeated_common_words() {
    let mut docs: Vec<(String, String)> = (0..40)
        .map(|i| {
            (
                format!("filler{i}"),
                "les notes des réunions pour les équipes".to_string(),
            )
        })
        .collect();
    docs.push((
        "common".into(),
        "les réunions des équipes pour les équipes des réunions pour les notes".into(),
    ));
    docs.push((
        "rare".into(),
        "réplication asynchrone entre deux nœuds".into(),
    ));
    let docs: Vec<(&str, &str)> = docs.iter().map(|(k, t)| (k.as_str(), t.as_str())).collect();
    let (engine, chunks, _tmp) = engine_with(&docs);

    let hits = chunks
        .fulltext_search("les réunions de réplication pour les équipes", None, 5)
        .unwrap();
    assert_eq!(hits[0].doc_key, "rare", "{hits:?}");
    assert_eq!(hits[0].matched_terms, vec!["réplication".to_string()]);
    assert!(hits.windows(2).all(|w| w[0].score >= w[1].score));

    let via_sdbql = keys(
        &engine,
        r#"RETURN FULLTEXT("chunks", "text", "les réunions de réplication pour les équipes")"#,
    );
    assert_eq!(via_sdbql[0], "rare");
}

/// Only the best candidates are loaded and scored. They used to be picked by
/// how many query words they held, so 200 chunks holding three common words
/// filled the pool and the one holding the rare word was never scored.
#[test]
fn candidate_pool_keeps_the_rare_word() {
    let mut docs: Vec<(String, String)> = (0..200)
        .map(|i| (format!("f{i:03}"), "les notes pour les équipes".to_string()))
        .collect();
    docs.push(("rare".into(), "procédure de réplication".into()));
    let docs: Vec<(&str, &str)> = docs.iter().map(|(k, t)| (k.as_str(), t.as_str())).collect();
    let (_engine, chunks, _tmp) = engine_with(&docs);

    let hits = chunks
        .fulltext_search("réplication pour les équipes notes", None, 10)
        .unwrap();
    assert_eq!(hits[0].doc_key, "rare", "{hits:?}");
}

/// A longer field with the same occurrences ranks lower.
#[test]
fn shorter_field_ranks_higher_for_same_occurrences() {
    let (_engine, chunks, _tmp) = engine_with(&[
        ("short", "sauvegarde incrémentale"),
        (
            "long",
            "sauvegarde complète du serveur avec rotation quotidienne hebdomadaire \
             mensuelle et vérification des archives",
        ),
    ]);
    let hits = chunks.fulltext_search("sauvegarde", None, 10).unwrap();
    assert_eq!(hits[0].doc_key, "short", "{hits:?}");
    assert!(hits[0].score > hits[1].score);
}

/// The distance lets a misspelt query word score, as half an occurrence,
/// in documents found through another word.
#[test]
fn fuzzy_match_counts_within_the_distance() {
    let (engine, _chunks, _tmp) = engine_with(&[
        ("typo", "configurer la réplication du cluster"),
        ("plain", "configurer le cluster"),
    ]);
    // "replicaton" has no postings: candidates come from "cluster".
    let exact = keys(
        &engine,
        r#"RETURN FULLTEXT("chunks", "text", "réplicaton cluster", 0)"#,
    );
    assert_eq!(exact.len(), 2);
    let fuzzy = keys(
        &engine,
        r#"RETURN FULLTEXT("chunks", "text", "réplicaton cluster", 2)"#,
    );
    assert_eq!(fuzzy[0], "typo", "{fuzzy:?}");
}

/// The allowed edits shrink with the word: at distance 2 a three-letter word
/// gets one edit, so "dix" (two edits from "des") no longer matches it.
#[test]
fn short_words_get_fewer_edits() {
    let (engine, _chunks, _tmp) = engine_with(&[("far", "cluster dix")]);
    let hits = QueryExecutor::new(&engine)
        .execute(&parse(r#"RETURN FULLTEXT("chunks", "text", "cluster des", 2)"#).unwrap())
        .unwrap();
    let hits = hits[0].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["matched"], json!(["cluster"]), "{hits:?}");
}
