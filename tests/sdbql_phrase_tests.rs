//! `FULLTEXT(..., { phrase: N })` keeps a hit only when the query words occur
//! in order, with at most N extra tokens inside the match.
//!
//! Order is checked while the document is scored. Postings stay term keys, so
//! an index built before the option still answers it. This is not the
//! in-memory `PHRASE()` boolean.

use serde_json::{json, Value};
use solidb::storage::{Collection, StorageEngine};
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn engine_with(docs: &[(&str, &str)]) -> (StorageEngine, Collection, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_collection("pages".into(), None).unwrap();
    let pages = engine.get_collection("pages").unwrap();
    pages
        .create_fulltext_index("ft_body".into(), vec!["body".into()], None)
        .unwrap();
    for (key, body) in docs {
        pages.insert(json!({"_key": key, "body": body})).unwrap();
    }
    (engine, pages, tmp)
}

fn engine_folded(docs: &[(&str, &str)]) -> (StorageEngine, Collection, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_collection("pages".into(), None).unwrap();
    let pages = engine.get_collection("pages").unwrap();
    pages
        .create_fulltext_index_with("ft_body".into(), vec!["body".into()], None, true)
        .unwrap();
    for (key, body) in docs {
        pages.insert(json!({"_key": key, "body": body})).unwrap();
    }
    (engine, pages, tmp)
}

fn keys(engine: &StorageEngine, q: &str) -> Vec<String> {
    let out = QueryExecutor::new(engine)
        .execute(&parse(q).unwrap())
        .unwrap_or_else(|e| panic!("{q}: {e}"));
    let mut keys: Vec<String> = out
        .into_iter()
        .map(|v| {
            v.as_str()
                .unwrap_or_else(|| panic!("{q}: row {v}"))
                .to_string()
        })
        .collect();
    keys.sort();
    keys
}

fn run(engine: &StorageEngine, q: &str) -> Value {
    let out = QueryExecutor::new(engine)
        .execute(&parse(q).unwrap())
        .unwrap_or_else(|e| panic!("{q}: {e}"));
    out.into_iter().next().unwrap()
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars().skip(start).take(end - start).collect()
}

fn span_at(hit: &Value, i: usize) -> (usize, usize) {
    let span = &hit["spans"][i];
    (
        span[0].as_u64().unwrap() as usize,
        span[1].as_u64().unwrap() as usize,
    )
}

#[test]
fn phrase_zero_keeps_the_adjacent_pair() {
    let (engine, _pages, _tmp) = engine_with(&[
        ("adj", "la réplication asynchrone des journaux"),
        ("rev", "la asynchrone réplication des journaux"),
        ("far", "réplication un deux trois quatre asynchrone"),
    ]);
    let ordered = r#"FOR h IN FULLTEXT("pages", "body", "réplication asynchrone", { phrase: 0, distance: 0 })
        RETURN h.doc._key"#;
    assert_eq!(keys(&engine, ordered), vec!["adj".to_string()]);

    // The same words, unordered, still find the distant document.
    let bag = r#"FOR h IN FULLTEXT("pages", "body", "réplication asynchrone", { distance: 0 })
        RETURN h.doc._key"#;
    assert_eq!(
        keys(&engine, bag),
        vec!["adj".to_string(), "far".to_string(), "rev".to_string()]
    );
}

#[test]
fn phrase_one_allows_a_single_filler() {
    let (engine, _pages, _tmp) = engine_with(&[
        ("adj", "alpha beta"),
        ("one", "alpha xx beta"),
        ("two", "alpha xx yy beta"),
    ]);
    let tight = r#"FOR h IN FULLTEXT("pages", "body", "alpha beta", { phrase: 0, distance: 0 })
        RETURN h.doc._key"#;
    assert_eq!(keys(&engine, tight), vec!["adj".to_string()]);
    let loose = r#"FOR h IN FULLTEXT("pages", "body", "alpha beta", { phrase: 1, distance: 0 })
        RETURN h.doc._key"#;
    assert_eq!(
        keys(&engine, loose),
        vec!["adj".to_string(), "one".to_string()]
    );
}

#[test]
fn snippet_quotes_the_phrase_not_an_earlier_word() {
    // The lone "réplication" sits far from the phrase, so a window around the
    // pair cannot reach it. Spans must be that pair only.
    let body = format!(
        "LONE réplication {}la réplication asynchrone des journaux {}",
        "filler ".repeat(40),
        "ENDING ".repeat(40)
    );
    let (engine, _pages, _tmp) = engine_with(&[("p", &body)]);
    let hit = run(
        &engine,
        r#"RETURN FIRST(FULLTEXT("pages", "body", "réplication asynchrone", { phrase: 0, distance: 0 }))"#,
    );
    let spans = hit["spans"].as_array().unwrap();
    assert_eq!(spans.len(), 2, "{hit}");
    let (start, end) = span_at(&hit, 0);
    let lone_end = "LONE réplication".chars().count();
    assert!(start > lone_end, "span covers the early word: {hit}");
    assert_eq!(slice_chars(&body, start, end), "réplication");
    let (start, end) = span_at(&hit, 1);
    assert_eq!(slice_chars(&body, start, end), "asynchrone");

    let passage = run(
        &engine,
        r#"RETURN SNIPPET(FIRST(FULLTEXT("pages", "body", "réplication asynchrone", { phrase: 0, distance: 0 })), 32)"#,
    );
    let passage = passage.as_str().unwrap();
    assert!(passage.contains("réplication"), "{passage}");
    assert!(passage.contains("asynchrone"), "{passage}");
    assert!(!passage.contains("LONE"), "{passage}");
    assert!(!passage.contains("ENDING"), "{passage}");
}

#[test]
fn fold_accents_phrase_shows_the_stored_spelling() {
    let body = "Mme Hélène Dupré tient la permanence";
    let (engine, _pages, _tmp) = engine_folded(&[("p", body)]);
    let hit = run(
        &engine,
        r#"RETURN FIRST(FULLTEXT("pages", "body", "helene dupre", { phrase: 0, distance: 0 }))"#,
    );
    assert_eq!(span_slice(body, &hit, 0), "Hélène");
    assert_eq!(span_slice(body, &hit, 1), "Dupré");
    let passage = run(
        &engine,
        r#"RETURN SNIPPET(FIRST(FULLTEXT("pages", "body", "helene dupre", { phrase: 0, distance: 0 })), 160)"#,
    );
    let passage = passage.as_str().unwrap();
    assert!(passage.contains("Hélène"), "{passage}");
    assert!(passage.contains("Dupré"), "{passage}");
}

fn span_slice(body: &str, hit: &Value, i: usize) -> String {
    let (start, end) = span_at(hit, i);
    slice_chars(body, start, end)
}

#[test]
fn a_short_word_stays_in_the_phrase() {
    let (engine, _pages, _tmp) = engine_with(&[
        ("exact", "la base de données principale"),
        ("gap", "la base xx données principale"),
    ]);
    let ordered = r#"FOR h IN FULLTEXT("pages", "body", "base de données", { phrase: 0, distance: 0 })
        RETURN h.doc._key"#;
    assert_eq!(keys(&engine, ordered), vec!["exact".to_string()]);
    // Without the phrase, "de" is below min_length and both documents match.
    let bag = r#"FOR h IN FULLTEXT("pages", "body", "base de données", { distance: 0 })
        RETURN h.doc._key"#;
    assert_eq!(
        keys(&engine, bag),
        vec!["exact".to_string(), "gap".to_string()]
    );
    // Nothing but short words: no postings, so no candidates.
    let none = r#"FOR h IN FULLTEXT("pages", "body", "de la", { phrase: 0, distance: 0 })
        RETURN h.doc._key"#;
    assert!(keys(&engine, none).is_empty());
}

#[test]
fn distance_zero_rejects_the_typo_that_distance_one_accepts() {
    let body = "la réplication cluster est prête";
    let (engine, _pages, _tmp) =
        engine_folded(&[("near", body), ("rev", "le cluster réplication tourne")]);
    let exact = r#"FOR h IN FULLTEXT("pages", "body", "replicaton cluster", { phrase: 0, distance: 0 })
        RETURN h.doc._key"#;
    assert!(keys(&engine, exact).is_empty());
    let fuzzy = r#"FOR h IN FULLTEXT("pages", "body", "replicaton cluster", { phrase: 0, distance: 1 })
        RETURN h.doc._key"#;
    assert_eq!(keys(&engine, fuzzy), vec!["near".to_string()]);
    let hit = run(
        &engine,
        r#"RETURN FIRST(FULLTEXT("pages", "body", "replicaton cluster", { phrase: 0, distance: 1 }))"#,
    );
    assert_eq!(span_slice(body, &hit, 0), "réplication");
    let matched = hit["matched"].as_array().unwrap();
    assert!(matched.iter().any(|t| t == "replicaton"), "{hit}");
    assert!(!matched.iter().any(|t| t == "réplication"), "{hit}");
}

#[test]
fn a_repeated_query_word_needs_two_document_words() {
    let (engine, _pages, _tmp) = engine_with(&[
        ("pair", "un cluster cluster local"),
        ("gap", "un cluster du cluster local"),
        ("once", "un seul cluster local"),
    ]);
    let tight = r#"FOR h IN FULLTEXT("pages", "body", "cluster cluster", { phrase: 0, distance: 0 })
        RETURN h.doc._key"#;
    assert_eq!(keys(&engine, tight), vec!["pair".to_string()]);
    let loose = r#"FOR h IN FULLTEXT("pages", "body", "cluster cluster", { phrase: 1, distance: 0 })
        RETURN h.doc._key"#;
    assert_eq!(
        keys(&engine, loose),
        vec!["gap".to_string(), "pair".to_string()]
    );
}

#[test]
fn one_word_phrase_does_not_filter() {
    let (engine, _pages, _tmp) = engine_with(&[("p", "la réplication des journaux")]);
    let hit = run(
        &engine,
        r#"RETURN FIRST(FULLTEXT("pages", "body", "réplication", { phrase: 0, distance: 0 }))"#,
    );
    assert_eq!(hit["doc"]["_key"], "p");
    assert_eq!(hit["spans"].as_array().unwrap().len(), 1);
}

#[test]
fn a_failing_field_does_not_outrank_the_phrase() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_collection("pages".into(), None).unwrap();
    let pages = engine.get_collection("pages").unwrap();
    pages
        .create_fulltext_index("ft".into(), vec!["title".into(), "body".into()], None)
        .unwrap();
    // The body repeats both words with a filler between them, so phrase 0
    // fails it, but its term frequency would outrank the short title.
    pages
        .insert(json!({
            "_key": "p",
            "title": "réplication asynchrone",
            "body": "réplication xx asynchrone ".repeat(15),
        }))
        .unwrap();

    let bag = pages
        .fulltext_search_fuzzy("réplication asynchrone", None, 5, 0, None)
        .unwrap();
    assert_eq!(bag[0].field, "body", "{bag:?}");
    let ordered = pages
        .fulltext_search_fuzzy("réplication asynchrone", None, 5, 0, Some(0))
        .unwrap();
    assert_eq!(ordered.len(), 1, "{ordered:?}");
    assert_eq!(ordered[0].field, "title", "{ordered:?}");
    assert_eq!(ordered[0].spans.len(), 2, "{ordered:?}");
}

#[test]
fn phrase_rejects_a_bad_value() {
    let (engine, _pages, _tmp) = engine_with(&[("p", "rien ici")]);
    let err = |q: &str| {
        QueryExecutor::new(&engine)
            .execute(&parse(q).unwrap())
            .unwrap_err()
            .to_string()
    };
    let bad = err(r#"RETURN FULLTEXT("pages", "body", "rien", { phrase: 1.5 })"#);
    assert!(bad.contains("integer"), "{bad}");
    let negative = err(r#"RETURN FULLTEXT("pages", "body", "rien", { phrase: -1 })"#);
    assert!(negative.contains("integer"), "{negative}");
    let unknown = err(r#"RETURN FULLTEXT("pages", "body", "rien", { nope: 1 })"#);
    assert!(unknown.contains("phrase"), "{unknown}");
    assert!(unknown.contains("nope"), "{unknown}");
}

/// Only a bounded number of candidates are scored, picked by the weight of
/// the words they hold. A thousand documents holding every word of the
/// phrase, apart, weigh exactly what the one holding the phrase weighs; they
/// used to fill the pool before the order was checked, and the phrase was
/// never found with a small `limit`.
fn crowded() -> (StorageEngine, Collection, TempDir) {
    let mut docs: Vec<(String, String)> = (0..1000)
        .map(|i| {
            (
                format!("f{i:04}"),
                "la base reste loin des données".to_string(),
            )
        })
        .collect();
    docs.push(("phrase".into(), "une base de données".into()));
    let docs: Vec<(&str, &str)> = docs.iter().map(|(k, t)| (k.as_str(), t.as_str())).collect();
    engine_with(&docs)
}

#[test]
fn phrase_is_found_among_many_unordered_candidates() {
    let (engine, _pages, _tmp) = crowded();
    let hits = keys(
        &engine,
        r#"FOR h IN FULLTEXT("pages", "body", "base de données", {phrase: 0, distance: 0, limit: 10}) RETURN h.doc._key"#,
    );
    assert_eq!(hits, vec!["phrase".to_string()]);
}

/// Same with the default distance, where no word can be required from the
/// postings because a near-miss may stand in for it.
#[test]
fn fuzzy_phrase_is_found_among_many_unordered_candidates() {
    let (engine, _pages, _tmp) = crowded();
    let hits = keys(
        &engine,
        r#"FOR h IN FULLTEXT("pages", "body", "base de données", {phrase: 0, limit: 10}) RETURN h.doc._key"#,
    );
    assert_eq!(hits, vec!["phrase".to_string()]);
}
