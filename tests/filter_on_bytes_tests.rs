//! An unindexed scan checks its leading FILTERs on the stored bytes and
//! decodes only the documents they keep (card #942,
//! `block_scan::block_pruned_docs`). Every query here is run that way and
//! with the typed reads off, which decodes every document and leaves the
//! FILTER clauses to drop them: the rows, their order and the errors must
//! be the same.

use serde_json::{json, Value};
use solidb::sdbql::{parse, QueryExecutor};
use solidb::storage::StorageEngine;
use tempfile::TempDir;

fn engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).expect("engine");
    engine
        .create_collection("things".to_string(), None)
        .unwrap();
    let things = engine.get_collection("things").unwrap();
    let kinds = ["a", "b", "c", ""];
    let mut docs: Vec<Value> = (0..600)
        .map(|i| {
            json!({
                "_key": format!("k{i:04}"),
                "n": i % 50,
                "f": (i % 7) as f64 * 1.5,
                "kind": kinds[i % 4],
                "on": i % 3 == 0,
                "tags": [i % 5, "x"],
                "in": {"depth": i % 9, "name": format!("n{}", i % 6)},
            })
        })
        .collect();
    // Documents that do not look like the others: a field missing, null,
    // of another kind, a number held as a float, a string that reads as one.
    docs.extend([
        json!({"_key": "odd-missing"}),
        json!({"_key": "odd-null", "n": null, "kind": null, "in": null}),
        json!({"_key": "odd-string", "n": "7", "kind": 7, "on": "true"}),
        json!({"_key": "odd-float", "n": 7.0, "f": 3, "kind": "a"}),
        json!({"_key": "odd-array", "n": [7], "kind": ["a"], "in": [1, 2]}),
        json!({"_key": "odd-object", "n": {"v": 7}, "in": {"depth": "3"}}),
    ]);
    things.insert_batch(docs).unwrap();
    (engine, tmp)
}

fn run_with(engine: &StorageEngine, q: &str, typed: bool) -> Result<Vec<Value>, String> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse failed for {q}: {e:?}"));
    let binds = [
        ("seven".to_string(), json!(7)),
        ("kind".to_string(), json!("a")),
        ("list".to_string(), json!([1, 7, "7", null])),
    ]
    .into_iter()
    .collect();
    QueryExecutor::with_bind_vars(engine, binds)
        .with_typed_collect(typed)
        .execute(&query)
        .map_err(|e| e.to_string())
}

const FILTERS: &[&str] = &[
    "FILTER d.n == 7",
    "FILTER d.n == @seven",
    "FILTER d.n != 7",
    "FILTER d.n < 3",
    "FILTER d.n <= 3 FILTER d.kind == @kind",
    "FILTER d.n > 47 OR d.kind == ''",
    "FILTER NOT (d.n >= 2)",
    "FILTER d.n IN @list",
    "FILTER d.n NOT IN [1, 2, 3] FILTER d.f > 7.4",
    "FILTER d.n == null",
    "FILTER d.missing == null FILTER d.n == 7",
    "FILTER d.kind == 'a' AND d.on == true AND d.f >= 3",
    "FILTER d.kind < 'b'",
    "FILTER d.on",
    "FILTER !d.on FILTER d.n == 1",
    "FILTER d.in.depth == 3",
    "FILTER d.in.name == 'n2' FILTER d.in.depth > 6",
    "FILTER d._key >= 'k0590'",
    "FILTER d._key == 'odd-float'",
    "FILTER d.f == 3",
    "FILTER d.n == '7'",
    "FILTER d.tags == [2, 'x'] FILTER d.n == 2",
    // Behind a FILTER that is not read from the bytes, and in front of one.
    "FILTER LENGTH(d.tags) == 2 FILTER d.n == 7",
    "FILTER d.n == 7 FILTER LENGTH(d.kind) == 1",
    "FILTER d.n == 7 FILTER d.kind == CONCAT('', 'a')",
    // Nothing kept.
    "FILTER d.n == 7 FILTER d.n == 8",
    "FILTER d.kind == 'nope'",
];

const TAILS: &[&str] = &[
    "RETURN d",
    "RETURN d._key",
    "RETURN {k: d._key, n: d.n, deep: d.in.name}",
    "SORT d.f DESC, d._key LIMIT 5 RETURN [d._key, d.f]",
    "LET twice = d.n * 2 FILTER twice != 14 RETURN [d._key, twice]",
    "COLLECT kind = d.kind INTO rows = d._key RETURN {kind, rows}",
    "LIMIT 3 RETURN d._key",
];

#[test]
fn a_filter_checked_on_the_stored_bytes_keeps_what_the_filter_clause_keeps() {
    let (engine, _tmp) = engine();
    let mut kept_some = 0;
    for filter in FILTERS {
        for tail in TAILS {
            let q = format!("FOR d IN things {filter} {tail}");
            let on_bytes = run_with(&engine, &q, true);
            let decoded = run_with(&engine, &q, false);
            assert_eq!(on_bytes, decoded, "{q}");
            assert!(decoded.is_ok(), "{q}: {decoded:?}");
            kept_some += usize::from(decoded.is_ok_and(|rows| !rows.is_empty()));
        }
    }
    assert!(kept_some > FILTERS.len() * TAILS.len() / 2, "{kept_some}");

    // An inner loop reads the outer row's value: not a constant, the same
    // rows all the same.
    let q = "FOR o IN things FILTER o.n == 49 FOR d IN things FILTER d.n == o.n - 42 \
             FILTER d.kind == o.kind RETURN [o._key, d._key]";
    assert_eq!(run_with(&engine, q, true), run_with(&engine, q, false));
    assert!(!run_with(&engine, q, true).unwrap().is_empty());
}

#[test]
fn a_mistake_in_a_filter_is_the_same_mistake() {
    let (engine, _tmp) = engine();
    for q in [
        "FOR d IN things FILTER d.n == 7 FILTER NOPE(d.n) RETURN d._key",
        "FOR d IN things FILTER NOPE(d.n) FILTER d.n == 7 RETURN d._key",
        "FOR d IN things FILTER d.n == 7 FILTER d.kind == @unbound RETURN d._key",
        "FOR d IN things FILTER d.n == @unbound RETURN d._key",
        // The FILTER read from the bytes keeps nothing: the one behind it
        // meets no row, either way.
        "FOR d IN things FILTER d.kind == 'nope' FILTER NOPE(d.n) RETURN d._key",
    ] {
        let on_bytes = run_with(&engine, q, true);
        let decoded = run_with(&engine, q, false);
        assert_eq!(on_bytes, decoded, "{q}");
    }
    let failed = run_with(
        &engine,
        "FOR d IN things FILTER d.n == 7 FILTER NOPE(d.n) RETURN d._key",
        true,
    );
    assert!(failed.unwrap_err().contains("NOPE"));
}

#[test]
fn explain_says_the_filter_was_checked_on_the_stored_documents() {
    let (engine, _tmp) = engine();
    let query = parse("FOR d IN things FILTER d.n == 7 RETURN d._key").unwrap();
    let explain = QueryExecutor::new(&engine).explain(&query).unwrap();
    let text = serde_json::to_string(&explain).unwrap();
    assert!(text.contains("checked on the stored documents"), "{text}");
    // Every document was read; twelve are returned, and one that holds 7.0.
    assert!(text.contains("606"), "{text}");
}
