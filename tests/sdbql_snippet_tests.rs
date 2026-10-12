//! Passages cut around the tokens a fulltext hit actually matched.
//!
//! Offsets are recorded while the document is scored. The postings stay
//! term keys, so an index built before snippets still answers them.

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

fn run(engine: &StorageEngine, q: &str) -> Value {
    let out = QueryExecutor::new(engine)
        .execute(&parse(q).unwrap())
        .unwrap_or_else(|e| panic!("{q}: {e}"));
    out.into_iter().next().unwrap()
}

fn slice_chars(text: &str, start: usize, end: usize) -> String {
    text.chars().skip(start).take(end - start).collect()
}

#[test]
fn snippet_is_the_passage_around_the_match() {
    // The marker words sit far from the match: a 40-character window around
    // "réplication" stays in the filler.
    let body = format!(
        "PREAMBLE {}la réplication asynchrone des journaux {}",
        "filler ".repeat(40),
        "ENDING ".repeat(40)
    );
    let (engine, pages, _tmp) = engine_with(&[("p", &body)]);

    let hits = pages.fulltext_search("réplication", None, 5).unwrap();
    assert_eq!(hits[0].field, "body");
    assert_eq!(hits.len(), 1);
    let (start, end) = hits[0].spans[0];
    assert_eq!(slice_chars(&body, start, end), "réplication");

    let passage = run(
        &engine,
        r#"FOR h IN FULLTEXT("pages", "body", "réplication")
             RETURN SNIPPET(h, 40)"#,
    );
    let passage = passage.as_str().unwrap();
    assert!(passage.contains("réplication"), "{passage}");
    assert!(!passage.contains("PREAMBLE"), "{passage}");
    assert!(!passage.contains("ENDING"), "{passage}");
    assert!(passage.contains('\u{2026}'), "{passage}");
}

#[test]
fn fuzzy_snippet_shows_the_stored_word() {
    let body = format!(
        "{}configurer la réplication du cluster{}",
        "PREAMBLE ".repeat(20),
        " ENDING".repeat(20)
    );
    let (engine, _pages, _tmp) = engine_with(&[("p", &body)]);
    let hit = run(
        &engine,
        r#"RETURN FIRST(FULLTEXT("pages", "body", "réplicaton cluster", 2))"#,
    );
    let matched = hit["matched"].as_array().unwrap();
    assert!(matched.iter().any(|t| t == "réplicaton"), "{hit}");
    assert!(!matched.iter().any(|t| t == "réplication"), "{hit}");

    let passage = run(
        &engine,
        r#"RETURN SNIPPET(FIRST(FULLTEXT("pages", "body", "réplicaton cluster", 2)), 80)"#,
    );
    let passage = passage.as_str().unwrap();
    assert!(passage.contains("réplication"), "{passage}");
    assert!(!passage.contains("réplicaton"), "{passage}");
}

#[test]
fn fold_accents_snippet_keeps_the_accented_spelling() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_collection("pages".into(), None).unwrap();
    let pages = engine.get_collection("pages").unwrap();
    pages
        .create_fulltext_index_with("ft_body".into(), vec!["body".into()], None, true)
        .unwrap();
    let body = "PREAMBLE Mme Hélène tient la permanence ENDING";
    pages.insert(json!({"_key": "p", "body": body})).unwrap();

    let hits = pages.fulltext_search("helene", None, 5).unwrap();
    assert_eq!(hits[0].matched_terms, vec!["helene".to_string()]);
    let (start, end) = hits[0].spans[0];
    assert_eq!(slice_chars(body, start, end), "Hélène");

    let passage = run(
        &engine,
        r#"RETURN SNIPPET(FIRST(FULLTEXT("pages", "body", "helene")), 40)"#,
    );
    let passage = passage.as_str().unwrap();
    assert!(passage.contains("Hélène"), "{passage}");
}

#[test]
fn search_index_hits_carry_the_same_spans() {
    let body = format!(
        "{}un mot rare au milieu{}",
        "pad ".repeat(20),
        " pad".repeat(20)
    );
    let (engine, _pages, _tmp) = engine_with(&[("p", &body)]);
    let hit = run(
        &engine,
        r#"RETURN FIRST(SEARCH_INDEX("pages", "body", "rare"))"#,
    );
    assert_eq!(hit["field"], json!("body"));
    assert!(hit.get("terms").is_some(), "{hit}");
    let start = hit["spans"][0][0].as_u64().unwrap() as usize;
    let end = hit["spans"][0][1].as_u64().unwrap() as usize;
    assert_eq!(slice_chars(&body, start, end), "rare");

    let passage = run(
        &engine,
        r#"RETURN SNIPPET(FIRST(SEARCH_INDEX("pages", "body", "rare")), 24)"#,
    );
    let passage = passage.as_str().unwrap();
    assert!(passage.contains("rare"), "{passage}");
    assert!(!passage.starts_with("pad"), "{passage}");
}

#[test]
fn the_winning_field_is_the_one_on_the_hit() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_collection("pages".into(), None).unwrap();
    let pages = engine.get_collection("pages").unwrap();
    pages
        .create_fulltext_index("ft".into(), vec!["title".into(), "body".into()], None)
        .unwrap();
    pages
        .insert(json!({
            "_key": "p",
            "title": "réplication",
            "body": format!("réplication {}", "générales ".repeat(40)),
        }))
        .unwrap();

    let hits = pages.fulltext_search("réplication", None, 5).unwrap();
    assert_eq!(hits[0].field, "title", "{hits:?}");
    let passage = run(
        &engine,
        r#"RETURN SNIPPET(FIRST(FULLTEXT("pages", "title", "réplication")), 160)"#,
    );
    assert_eq!(passage, json!("réplication"));
}

#[test]
fn snippet_rejects_a_bad_width() {
    let (engine, _pages, _tmp) = engine_with(&[("p", "rien ici")]);
    let err = QueryExecutor::new(&engine)
        .execute(&parse(r#"RETURN SNIPPET({doc: {body: "x"}, field: "body"}, 1.5)"#).unwrap())
        .unwrap_err();
    assert!(err.to_string().contains("integer"), "{err}");
}
