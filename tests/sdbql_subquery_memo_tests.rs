//! Uncorrelated subqueries run once per query (`executor/subquery_memo.rs`),
//! and everything that must still run per row does.

use serde_json::{json, Value};
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn run(engine: &StorageEngine, q: &str) -> Vec<Value> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    QueryExecutor::new(engine)
        .execute(&query)
        .unwrap_or_else(|e| panic!("execute {q}: {e}"))
}

fn engine() -> (StorageEngine, TempDir) {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
    (engine, dir)
}

/// The live card board's shape: comments IN (cards IN (columns)). It used to
/// run the inner FORs once per comment, nested, and took minutes in
/// production on 2k comments / 1k cards / 168 columns.
#[test]
fn nested_uncorrelated_in_subqueries_match_hoisted_form() {
    let (engine, _dir) = engine();
    for c in ["card_columns", "cards", "comments"] {
        engine.create_collection(c.to_string(), None).unwrap();
    }
    let cols = engine.get_collection("card_columns").unwrap();
    for i in 0..40 {
        let project = if i % 2 == 0 { "p1" } else { "p2" };
        cols.insert(json!({"_key": format!("col{i}"), "project_id": project}))
            .unwrap();
    }
    let cards = engine.get_collection("cards").unwrap();
    for i in 0..400 {
        cards
            .insert(json!({"_key": format!("card{i}"), "column_id": format!("col{}", i % 40)}))
            .unwrap();
    }
    let comments = engine.get_collection("comments").unwrap();
    for i in 0..800 {
        let kind = if i % 5 == 0 { "doc" } else { "card" };
        comments
            .insert(json!({
                "_key": format!("m{i:04}"),
                "commentable_type": kind,
                "commentable_id": format!("card{}", (i * 7) % 450),
            }))
            .unwrap();
    }

    let col_ids = r#"(FOR col IN card_columns FILTER col.project_id == "p1" RETURN col._key)"#;
    let card_ids = format!("(FOR c IN cards FILTER c.column_id IN {col_ids} RETURN c._key)");
    let nested = format!(
        r#"FOR m IN comments FILTER m.commentable_type == "card" AND m.commentable_id IN {card_ids} SORT m._key RETURN m._key"#
    );
    let hoisted = format!(
        r#"LET ids = {card_ids} FOR m IN comments FILTER m.commentable_type == "card" AND m.commentable_id IN ids SORT m._key RETURN m._key"#
    );

    let started = std::time::Instant::now();
    let got = run(&engine, &nested);
    let elapsed = started.elapsed();
    let want = run(&engine, &hoisted);
    assert!(!want.is_empty());
    assert_eq!(got, want);
    // Per-row evaluation is 800 × (400 + 400 × 40) rows; once is ~1k.
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "nested IN subqueries took {elapsed:?}"
    );

    // NOT IN takes the same path.
    let not_in = run(
        &engine,
        &format!(
            r#"FOR m IN comments FILTER m.commentable_type == "card" AND m.commentable_id NOT IN {card_ids} RETURN 1"#
        ),
    );
    assert_eq!(not_in.len() + want.len(), 640);
}

#[test]
fn correlated_subquery_still_runs_per_row() {
    let (engine, _dir) = engine();
    engine
        .create_collection("orders".to_string(), None)
        .unwrap();
    let orders = engine.get_collection("orders").unwrap();
    for (k, u) in [("o1", "a"), ("o2", "a"), ("o3", "b")] {
        orders.insert(json!({"_key": k, "user": u})).unwrap();
    }
    let got = run(
        &engine,
        r#"FOR u IN ["a", "b", "c"] RETURN LENGTH(FOR o IN orders FILTER o.user == u RETURN 1)"#,
    );
    assert_eq!(got, vec![json!(2), json!(1), json!(0)]);
}

/// `FOR t IN tags` names the row's variable without an Expression::Variable.
#[test]
fn subquery_iterating_a_row_variable_by_name_is_correlated() {
    let (engine, _dir) = engine();
    let got = run(
        &engine,
        r#"FOR row IN [[1, 2], [3], []] LET tags = row RETURN (FOR t IN tags RETURN t * 10)"#,
    );
    assert_eq!(got, vec![json!([10, 20]), json!([30]), json!([])]);
}

#[test]
fn nondeterministic_subquery_is_not_memoised() {
    let (engine, _dir) = engine();
    let got = run(
        &engine,
        "FOR i IN 1..20 RETURN FIRST(FOR x IN 1..1 RETURN RAND())",
    );
    let first = &got[0];
    assert!(
        got.iter().any(|v| v != first),
        "RAND() in a subquery returned the same value on all 20 rows"
    );
}

/// A writing query must see its own earlier writes in a later row.
#[test]
fn subquery_in_writing_query_reads_per_row() {
    let (engine, _dir) = engine();
    engine.create_collection("log".to_string(), None).unwrap();
    let got = run(
        &engine,
        "FOR i IN 1..3 INSERT {n: LENGTH(FOR d IN log RETURN 1)} INTO log RETURN NEW.n",
    );
    assert_eq!(got, vec![json!(0), json!(1), json!(2)]);
}

/// The memo is per query: a reused executor sees the data as it is now.
#[test]
fn memo_does_not_outlive_the_query() {
    let (engine, _dir) = engine();
    engine.create_collection("items".to_string(), None).unwrap();
    let items = engine.get_collection("items").unwrap();
    items.insert(json!({"_key": "a"})).unwrap();
    let query = parse("FOR i IN 1..2 RETURN LENGTH(FOR d IN items RETURN 1)").unwrap();
    let executor = QueryExecutor::new(&engine);
    assert_eq!(executor.execute(&query).unwrap(), vec![json!(1), json!(1)]);
    items.insert(json!({"_key": "b"})).unwrap();
    assert_eq!(executor.execute(&query).unwrap(), vec![json!(2), json!(2)]);
}
