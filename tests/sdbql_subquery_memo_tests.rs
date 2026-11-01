//! A subquery's rows are reused while the bindings it can read are unchanged
//! (`executor/subquery_memo.rs`), and everything that must still run per row
//! does.
//!
//! Most of this file is one differential test: every query in [`SHAPES`] runs
//! with reuse on and with reuse off, on fresh executors, over a dataset with
//! the awkward values (1 vs 1.0, -0.0, null, missing fields, arrays, nested
//! objects, a dotted field name), without and then with indexes. Results,
//! order included, and errors must be the same. To cover a new shape, add its
//! query to [`SHAPES`]. The other tests assert how much work reuse saves,
//! through the executor's counters (never wall-clock time).

use serde_json::{json, Value};
use solidb::storage::{IndexType, StorageEngine};
use solidb::{parse, BindVars, QueryExecutor};
use tempfile::TempDir;

fn engine() -> (StorageEngine, TempDir) {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
    (engine, dir)
}

fn executor<'a>(engine: &'a StorageEngine, binds: &BindVars, reuse: bool) -> QueryExecutor<'a> {
    QueryExecutor::with_bind_vars(engine, binds.clone()).with_reuse_within_query(reuse)
}

fn run(engine: &StorageEngine, q: &str) -> Vec<Value> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    QueryExecutor::new(engine)
        .execute(&query)
        .unwrap_or_else(|e| panic!("execute {q}: {e}"))
}

/// The results, and the subquery runs and collection scans they took.
fn run_counted(engine: &StorageEngine, binds: &BindVars, q: &str) -> (Vec<Value>, u64, u64) {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    let executor = executor(engine, binds, true);
    let rows = executor
        .execute(&query)
        .unwrap_or_else(|e| panic!("execute {q}: {e}"));
    (rows, executor.subquery_runs(), executor.collection_scans())
}

fn outcome(
    engine: &StorageEngine,
    binds: &BindVars,
    q: &str,
    reuse: bool,
) -> Result<Vec<Value>, String> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    executor(engine, binds, reuse)
        .execute(&query)
        .map_err(|e| e.to_string())
}

/// Same results (or the same error) with reuse on and off. Only a query
/// that reads `undefinedVar` is expected to fail, so a shape that stops
/// parsing or executing cannot pass by failing the same way twice.
fn assert_same_with_and_without_reuse(engine: &StorageEngine, binds: &BindVars, q: &str) {
    let off = outcome(engine, binds, q, false);
    let on = outcome(engine, binds, q, true);
    assert_eq!(on, off, "reuse on vs off differ for: {q}");
    if let Err(e) = &off {
        assert!(q.contains("undefinedVar"), "{q} failed: {e}");
    }
}

fn binds() -> BindVars {
    let mut b = BindVars::new();
    b.insert("col".to_string(), json!("col001"));
    b
}

/// Columns, cards and comments shaped like the card board, plus edge
/// documents for the comparisons that hashing and addresses can get wrong.
fn seed(engine: &StorageEngine) {
    for c in [
        "cols",
        "cards",
        "comments",
        "orders",
        "products",
        "reactions",
    ] {
        engine.create_collection(c.to_string(), None).unwrap();
    }
    let cols = engine.get_collection("cols").unwrap();
    for i in 0..10 {
        let project = if i % 2 == 0 { "p1" } else { "p2" };
        cols.insert(json!({"_key": format!("col{i:03}"), "project_id": project, "n": i % 3}))
            .unwrap();
    }
    let cards = engine.get_collection("cards").unwrap();
    for i in 0..60 {
        cards
            .insert(json!({
                "_key": format!("card{i:02}"),
                "column_id": format!("col{:03}", i % 10),
                "n": i % 7,
                "items": [i % 3, i % 5],
                "meta": {"owner": format!("u{}", i % 4)},
            }))
            .unwrap();
    }
    for doc in [
        json!({"_key": "k1", "column_id": 1, "n": 1}),
        json!({"_key": "k2", "column_id": 1.0, "n": 1.0}),
        json!({"_key": "k3", "column_id": -0.0, "n": 0}),
        json!({"_key": "k4", "column_id": null}),
        json!({"_key": "k5"}),
        json!({"_key": "k6", "column_id": ["col001"], "a.b": "card02", "a": {"b": "card03"}}),
    ] {
        cards.insert(doc).unwrap();
    }
    let comments = engine.get_collection("comments").unwrap();
    for i in 0..150 {
        comments
            .insert(json!({
                "_key": format!("m{i:03}"),
                "card_id": format!("card{:02}", (i * 7) % 65),
                "n": i % 11,
                "body": format!("comment {i}"),
                "kind": if i % 5 == 0 { "doc" } else { "card" },
                "author": format!("u{}", i % 4),
            }))
            .unwrap();
    }
    for doc in [
        json!({"_key": "e1", "card_id": 1, "n": 1}),
        json!({"_key": "e2", "card_id": 1.0, "n": 2}),
        json!({"_key": "e3", "card_id": -0.0, "n": 3}),
        json!({"_key": "e4", "card_id": 0, "n": 4}),
        json!({"_key": "e5", "card_id": null, "n": 5}),
        json!({"_key": "e6", "n": 6, "a": {"b": "card05"}}),
        json!({"_key": "e7", "card_id": ["card01"], "n": 7}),
        json!({"_key": "e8", "card_id": {"x": 1}, "n": 8}),
        json!({"_key": "e9", "a.b": "card02", "a": {"b": "card03"}, "n": 9}),
    ] {
        comments.insert(doc).unwrap();
    }
    let reactions = engine.get_collection("reactions").unwrap();
    for i in 0..40 {
        reactions
            .insert(
                json!({"_key": format!("r{i:02}"), "comment_id": format!("m{:03}", (i * 3) % 150)}),
            )
            .unwrap();
    }
    let products = engine.get_collection("products").unwrap();
    for i in 0..8 {
        products
            .insert(json!({"_key": format!("p{i}"), "name": format!("product {i}")}))
            .unwrap();
    }
    let orders = engine.get_collection("orders").unwrap();
    for i in 0..12 {
        let items: Vec<Value> = (0..4)
            .map(|j| json!({"product": format!("p{}", (i + j / 2) % 9)}))
            .collect();
        orders
            .insert(json!({"_key": format!("o{i:02}"), "items": items}))
            .unwrap();
    }
}

/// The differential test's indexes. `comments.body` is a Bloom index, which
/// must never serve a subquery-valued key (its definite miss is not exact).
const INDEXES: &[(&str, &str, IndexType)] = &[
    ("comments", "card_id", IndexType::Persistent),
    ("comments", "n", IndexType::Hash),
    ("comments", "body", IndexType::Bloom),
    ("cards", "column_id", IndexType::Persistent),
    ("cards", "n", IndexType::Hash),
];

fn add_indexes(engine: &StorageEngine) {
    add_indexes_where(engine, |_, _| true);
}

fn add_indexes_where(engine: &StorageEngine, keep: impl Fn(&str, &str) -> bool) {
    for (coll, field, kind) in INDEXES
        .iter()
        .filter(|(coll, field, _)| keep(coll, field))
        .cloned()
    {
        engine
            .get_collection(coll)
            .unwrap()
            .create_index(
                format!("idx_{coll}_{field}"),
                vec![field.to_string()],
                kind,
                false,
            )
            .unwrap();
    }
}

/// Every shape reuse can apply to, and the ones it must not. Add new ones
/// here; `@col` is bound to "col001".
const SHAPES: &[&str] = &[
    // Uncorrelated, nested two deep (the board query).
    r#"FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id IN (FOR col IN cols FILTER col.project_id == "p1" RETURN col._key) RETURN c._key) RETURN m._key"#,
    r#"FOR m IN comments FILTER m.card_id NOT IN (FOR c IN cards FILTER c.column_id == "col001" RETURN c._key) RETURN m._key"#,
    // IN over a small and a large (hashed) memoised result.
    r#"FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == "col001" RETURN c._key) RETURN m._key"#,
    "FOR m IN comments FILTER m.card_id IN (FOR c IN cards RETURN c._key) RETURN m._key",
    "FOR m IN comments FILTER m.card_id IN (FOR c IN cards RETURN c._key) LIMIT 1 RETURN m._key",
    // Inner subquery reading only an outer loop's row.
    "FOR col IN cols LET ms = (FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == col._key RETURN c._key) RETURN m._key) RETURN {col: col._key, ms}",
    "FOR a IN cols FOR b IN [1, 2] RETURN (FOR c IN cards FILTER c.column_id == a._key AND c.n == b RETURN c._key)",
    // Truly per-row.
    "FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key RETURN m._key) RETURN {card: card._key, ms}",
    "FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key RETURN m) LET ks = (FOR x IN ms RETURN x._key) RETURN ks",
    "FOR m IN comments SORT m.card_id LET col = FIRST(FOR k IN cards FILTER k._key == m.card_id RETURN k.column_id) RETURN [m._key, col]",
    "FOR card IN cards RETURN LENGTH(FOR m IN comments FILTER m.card_id == card._key RETURN 1)",
    // A LET shared by every row, before and inside the FOR.
    "FOR m IN comments LET cs = (FOR c IN cards RETURN c) RETURN LENGTH(cs)",
    "LET cs = (FOR c IN cards RETURN c) FOR m IN comments RETURN LENGTH(cs)",
    "FOR m IN comments LET cs = (FOR c IN cards SORT c._key LIMIT 3 RETURN c) RETURN [m._key, cs[0]._key, cs[2].meta.owner]",
    "FOR m IN comments LET cs = (FOR c IN cards LIMIT 3 RETURN c._key) SORT m._key LIMIT 5 LET more = (FOR c IN cards FILTER c.n == m.n RETURN c._key) RETURN [cs, more]",
    // The ORM names every loop variable `doc`.
    "FOR doc IN comments FILTER doc.card_id IN (FOR doc IN cards FILTER doc.column_id == @col RETURN doc._key) RETURN doc._key",
    "FOR doc IN cards RETURN (FOR doc IN cards SORT doc._key LIMIT doc.n RETURN doc._key)",
    "FOR doc IN comments FILTER doc.n < 3 RETURN (FOR doc IN cards FILTER doc.n == 1 RETURN doc._key UNION FOR c IN cards FILTER c._key == doc.card_id RETURN c._key)",
    "FOR doc IN cards RETURN (FOR doc IN doc.items RETURN doc)",
    "FOR x IN [[1, 2], [3], []] LET doc = x RETURN (FOR doc IN doc RETURN doc)",
    "FOR doc IN cards LET r = (LET x = doc.n FOR doc IN cards FILTER doc.n == x RETURN doc._key) RETURN LENGTH(r)",
    // COLLECT INTO: every variable in scope, the outer row's included.
    "FOR x IN [1, 2, 3] RETURN (FOR y IN [10] COLLECT k = y INTO g RETURN g[0].x)",
    "FOR x IN [1, 2, 3] RETURN (FOR z IN [0] RETURN (FOR y IN [10] COLLECT k = y INTO g RETURN g[0].x))",
    "FOR x IN [1, 2, 3] RETURN (FOR y IN [10] COLLECT k = y INTO g KEEP y RETURN g)",
    "FOR x IN [1, 2, 3] RETURN (FOR y IN [10] COLLECT k = y INTO g = y RETURN g)",
    "FOR m IN comments LET cs = (FOR c IN cards LIMIT 3 RETURN c._key) COLLECT k = m.n INTO g KEEP cs RETURN {k, n: LENGTH(g), first: g[0].cs}",
    "FOR m IN comments LET cs = (FOR c IN cards LIMIT 3 RETURN c._key) COLLECT k = m.n INTO g KEEP m RETURN {k, keys: g[*].m._key}",
    // A backtick identifier binds the key a missing bind var falls back to.
    "FOR x IN [[1], [2]] LET `@ids` = x RETURN (FOR d IN 1..3 FILTER d IN @ids RETURN d)",
    // Lambdas whose body runs a subquery on the parameter.
    "RETURN MAP([1, 1, 2, 2, 3], x -> FIRST(FOR i IN [x] RETURN i))",
    "FOR o IN orders RETURN MAP(o.items, it -> FIRST(FOR p IN products FILTER p._key == it.product RETURN p.name))",
    "FOR o IN orders RETURN FILTER(o.items, it -> LENGTH(FOR p IN products FILTER p._key == it.product RETURN 1) > 0)",
    "FOR o IN orders RETURN SORT_BY(o.items, it -> FIRST(FOR p IN products FILTER p._key == it.product RETURN p.name))",
    "RETURN REDUCE([1, 2, 3, 4], (acc, x) -> acc + LENGTH(FOR i IN 1..acc RETURN i) + x, 0)",
    "FOR card IN cards LIMIT 5 RETURN card.items[* FILTER LENGTH(FOR c IN cards FILTER c.n == CURRENT RETURN 1) > 0]",
    // Numbers that are equal but not identical, null and missing fields.
    "FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == 1 RETURN c.column_id) RETURN m._key",
    "FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == 0 RETURN c.column_id) RETURN m._key",
    "FOR m IN comments FILTER m.card_id == FIRST(FOR c IN cards FILTER c.nope == null LIMIT 1 RETURN c.nope) RETURN m._key",
    "FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == null RETURN c.column_id) RETURN m._key",
    r#"FOR m IN comments FILTER m["a.b"] IN (FOR c IN cards RETURN c["a.b"]) RETURN m._key"#,
    "FOR m IN comments FILTER m.a.b IN (FOR c IN cards RETURN c.a.b) RETURN m._key",
    r#"FOR m IN comments FILTER (FOR c IN cards FILTER c._key == "card01" RETURN c._key) ANY == m.card_id[0] RETURN m._key"#,
    // A LET-bound array named like a collection.
    r#"LET cards = [{_key: "card01"}, {_key: "card02"}] FOR m IN comments FILTER m.card_id IN (FOR c IN cards RETURN c._key) RETURN m._key"#,
    r#"FOR m IN comments LET cards = [m.card_id] FILTER m.n == 1 RETURN (FOR c IN cards RETURN c)"#,
    // Errors are the same, whichever operand fails first.
    "FOR c IN cols RETURN undefinedVar IN (FOR k IN cards FILTER k.column_id == c._key RETURN k.x[true])",
    "FOR c IN cols RETURN undefinedVar IN (FOR k IN cards RETURN k.x[true])",
    // A subquery-valued key an index (or `_key` / `_id`) can serve: the
    // scan's rows in the scan's order, whatever order the subquery returns
    // its keys in, and LIMIT without SORT keeps the scan's first rows.
    r#"FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == "col001" SORT c._key DESC RETURN c._key) RETURN m._key"#,
    r#"FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == "col001" SORT c._key DESC RETURN c._key) LIMIT 1 RETURN m._key"#,
    r#"FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == "col001" SORT c._key DESC RETURN c._key) AND m.n > 5 RETURN m._key"#,
    "FOR m IN comments FILTER m.n > 5 AND m.card_id IN (FOR c IN cards SORT c._key DESC LIMIT 20 RETURN c._key) RETURN m._key",
    "FOR m IN comments FILTER m._key IN (FOR x IN comments FILTER x.n == 3 SORT x._key DESC RETURN x._key) RETURN m._key",
    "FOR m IN comments FILTER m._key IN (FOR x IN comments FILTER x.n == 3 SORT x._key DESC RETURN x._key) LIMIT 2 RETURN m._key",
    "FOR m IN comments FILTER m._id IN (FOR x IN comments FILTER x.n == 4 SORT x._key DESC RETURN x._id) RETURN m._key",
    "FOR m IN comments FILTER m.card_id == FIRST(FOR c IN cards FILTER c.n == 3 SORT c._key DESC RETURN c._key) RETURN m._key",
    "FOR m IN comments FILTER FIRST(FOR c IN cards FILTER c.n == 3 RETURN c._key) == m.card_id RETURN m._key",
    r#"FOR m IN comments FILTER m.body != null FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == "col002" RETURN c._key) RETURN m._key"#,
    "FOR col IN cols LET ms = (FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == col._key SORT c._key DESC RETURN c._key) RETURN m._key) RETURN {col: col._key, ms}",
    "FOR doc IN comments FILTER doc.card_id IN (FOR doc IN cards FILTER doc.column_id == @col SORT doc._key DESC RETURN doc._key) RETURN doc._key",
    "FOR card IN cards LIMIT 10 LET ms = (FOR m IN comments FILTER m.card_id == card._key AND m.n IN (FOR c IN cards FILTER c.n == 2 RETURN c.n) RETURN m._key) RETURN ms",
    // Keys equal but not identical (1 and 1.0), a numeric zero beside -0.0.
    r#"FOR m IN comments FILTER m.n == FIRST(FOR c IN cards FILTER c._key == "k2" RETURN c.n) RETURN m._key"#,
    r#"FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c._key IN ["k1", "k2"] RETURN c.column_id) RETURN m._key"#,
    r#"FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c._key IN ["k1", "k3"] RETURN c.column_id) RETURN m._key"#,
    // Not served: the subquery reads the loop variable, the index is Bloom,
    // the comparison is a range or a LIKE, the collection name is a LET.
    "FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.n == m.n RETURN c._key) RETURN m._key",
    r#"FOR m IN comments FILTER m.body IN (FOR c IN cards FILTER c.n == 2 RETURN CONCAT("comment ", c.n)) RETURN m._key"#,
    r#"FOR m IN comments FILTER m.n > FIRST(FOR c IN cards FILTER c._key == "card05" RETURN c.n) RETURN m._key"#,
    r#"FOR m IN comments FILTER m.card_id LIKE CONCAT(FIRST(FOR c IN cards FILTER c._key == "card05" RETURN SUBSTRING(c._key, 0, 5)), "%") RETURN m._key"#,
    r#"LET comments = [{_key: "z", card_id: "card01"}] FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == "col001" RETURN c._key) RETURN m._key"#,
    // A subquery in a branch, and one inside a CTE.
    "FOR c IN cols RETURN c.n == 0 ? (FOR k IN cards FILTER k.column_id == c._key RETURN k._key) : []",
    "WITH ks AS (FOR c IN cols RETURN (FOR k IN cards FILTER k.column_id == c._key RETURN k._key)) FOR x IN ks RETURN x",
    "WITH t AS (FOR x IN [1, 2, 3] RETURN (FOR y IN [10] COLLECT k = y INTO g RETURN g[0].x)) FOR r IN t RETURN r",
    // A correlated FILTER no index serves, answered from a per-query hash
    // (`execution/hash_probe.rs`): per run of a subquery, and per row of a
    // nested FOR.
    "FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key SORT m.n DESC LIMIT 2 RETURN m.n) RETURN ms",
    "FOR c IN cols FOR card IN cards FILTER card.column_id == c._key RETURN [c._key, card._key]",
    "FOR c IN cols FOR card IN cards FILTER c._key == card.column_id AND card.n > 2 RETURN [c._key, card._key]",
    r#"FOR c IN cols FOR m IN comments FILTER m.card_id == CONCAT("card0", c.n) RETURN m"#,
    r#"FOR c IN cols FOR m IN comments FILTER m.card_id == CONCAT("card0", c.n) RETURN {c: c._key, m: m.body, k: m._key}"#,
    // IN: an array with duplicates, an object (its keys), a string (never).
    r#"FOR c IN cols FOR m IN comments FILTER m.card_id IN [CONCAT("card0", c.n), CONCAT("card0", c.n), "card10", 1] RETURN [c._key, m._key]"#,
    r#"FOR c IN cols FOR m IN comments FILTER m.card_id IN (c.n == 1 ? {card01: 1, card07: 2} : {card14: true}) RETURN [c._key, m._key]"#,
    "FOR c IN cols FOR m IN comments FILTER m.card_id IN c._key RETURN [c._key, m._key]",
    // Equal but not identical numbers, null, and missing fields.
    "FOR x IN [1, 1.0, 0, -0.0, null, \"card01\"] FOR m IN comments FILTER m.card_id == x RETURN [x, m._key]",
    "FOR x IN [null, 1] FOR m IN comments FILTER m.nope == x RETURN [x, m._key]",
    "FOR x IN [[\"card01\"], {x: 1}, {x: 1.0}] FOR m IN comments FILTER m.card_id == x RETURN m._key",
    // `v.a.b` and `v["a.b"]` are different fields; a dotted segment with no
    // such key reads the nested path.
    r#"FOR x IN [{v: "card03", w: "card02"}, {v: "card03", w: "card02"}] FOR y IN comments FILTER y.a.b == x.v FOR z IN comments FILTER z["a.b"] == x.w RETURN [y._key, z._key]"#,
    r#"FOR x IN [{v: "card03", w: "card02"}, {v: "card03", w: "card02"}] LET ys = (FOR y IN comments FILTER y.a.b == x.v RETURN y._key) LET zs = (FOR z IN comments FILTER z["a.b"] == x.w RETURN z._key) RETURN [ys, zs]"#,
    r#"FOR x IN ["card05", "card05"] FOR z IN comments FILTER z["a.b"] == x RETURN z._key"#,
    // A key that fails: the FILTER fails for every document.
    "FOR c IN cols FOR m IN comments FILTER m.card_id == c.x[true] RETURN m._key",
    // A LET-bound array named like the collection is iterated instead.
    r#"LET comments = [{_key: "z", card_id: "card01"}] FOR c IN cards FOR m IN comments FILTER m.card_id == c._key RETURN [c._key, m._key]"#,
    // Several keys: each row probes the one with the fewest candidates.
    r#"FOR card IN cards LET ms = (FOR m IN comments FILTER m.kind == "card" AND m.card_id == card._key RETURN m._key) RETURN ms"#,
    r#"FOR card IN cards LET ms = (FOR m IN comments FILTER m.kind == "card" FILTER m.card_id == card._key RETURN m._key) RETURN ms"#,
    r#"FOR card IN cards LET ms = (FOR m IN comments FILTER m.n == card.n AND m._key IN ["m001", "m012", "m023", "e1"] RETURN m._key) RETURN ms"#,
    // A probe inside a probe on the same collection, and one reached from a
    // candidate's FILTER re-check.
    "FOR a IN cols FOR b IN cards FILTER b.column_id == FIRST(FOR z IN cards FILTER z.column_id == a._key RETURN z.column_id) RETURN b._key",
    "FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key FILTER LENGTH(FOR r IN reactions FILTER r.comment_id == m._key RETURN 1) > 0 RETURN m._key) RETURN ms",
    "FOR col IN cols FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == col._key RETURN c._key) RETURN [col._key, m._key]",
    "FOR col IN cols FOR c IN cards FILTER c.column_id == col._key FILTER LENGTH(FOR m IN comments FILTER m.card_id == c._key RETURN 1) > 0 RETURN c._key",
    "FOR col IN cols SORT col._key LET ms = (FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == col._key RETURN c._key) FILTER m.n > 2 SORT m._key RETURN m._key) RETURN {col: col._key, ms}",
    // COLLECT INTO over probe rows, with and without KEEP.
    r#"FOR c IN cols FOR m IN comments FILTER m.card_id IN [CONCAT("card0", c.n), "card10"] COLLECT k = c._key INTO g KEEP m RETURN {k, keys: g[*].m._key}"#,
    r#"FOR c IN cols FOR m IN comments FILTER m.card_id IN [CONCAT("card0", c.n), "card10"] COLLECT k = c.n INTO g RETURN {k, n: LENGTH(g), m: g[0].m.body}"#,
    // Two subqueries over one collection: whole documents twice, and two
    // projections, which must never be served from each other's documents.
    "FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key RETURN m) LET mine = (FOR m IN comments FILTER m.author == card.meta.owner RETURN m) RETURN {ms: ms[*]._key, mine: mine[*]._key}",
    "FOR card IN cards LET a = (FOR m IN comments FILTER m.card_id == card._key RETURN m._key) LET b = (FOR m IN comments FILTER m.card_id == card._key RETURN m.body) RETURN [a, b]",
    r#"LET a = (FOR t IN comments FILTER t.author == "u1" RETURN t._key) LET b = (FOR t IN comments FILTER t.author == "u2" RETURN t._key) RETURN [LENGTH(a), LENGTH(b)]"#,
    // Non-deterministic keys are not probed.
    "FOR c IN cols FOR m IN comments FILTER m.n == RAND_INT(0, 0) RETURN [c._key, m._key]",
    "FOR c IN cols FOR m IN comments FILTER m.n == RAND_INT(0, 0) AND m.card_id == CONCAT(\"card0\", c.n) RETURN [c._key, m._key]",
];

#[test]
fn reuse_never_changes_results() {
    let (engine, _dir) = engine();
    seed(&engine);
    let binds = binds();
    for q in SHAPES {
        assert_same_with_and_without_reuse(&engine, &binds, q);
    }
    add_indexes(&engine);
    for q in SHAPES {
        assert_same_with_and_without_reuse(&engine, &binds, q);
    }
}

/// A subquery-valued key is served by whichever index exists, and returns
/// the scan's rows in the scan's order either way.
#[test]
fn reuse_never_changes_results_with_any_one_index() {
    for field in ["card_id", "n"] {
        let (engine, _dir) = engine();
        seed(&engine);
        add_indexes_where(&engine, |coll, f| coll == "comments" && f == field);
        let binds = binds();
        for q in SHAPES {
            assert_same_with_and_without_reuse(&engine, &binds, q);
        }
    }
}

/// How the first FOR of `q` reads its collection, per EXPLAIN.
fn first_access(engine: &StorageEngine, q: &str, reuse: bool) -> (String, Option<String>) {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    let plan = executor(engine, &binds(), reuse)
        .explain(&query)
        .unwrap_or_else(|e| panic!("explain {q}: {e}"));
    let access = &plan.collections[0];
    (access.access_type.clone(), access.index_used.clone())
}

#[test]
fn explain_reports_a_subquery_valued_key_served_by_an_index() {
    let (engine, _dir) = engine();
    seed(&engine);
    add_indexes(&engine);
    let in_subquery = r#"FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == "col001" RETURN c._key) RETURN m"#;
    let by_card = (
        "index_lookup".to_string(),
        Some("idx_comments_card_id".to_string()),
    );
    assert_eq!(first_access(&engine, in_subquery, true), by_card);
    assert_eq!(
        first_access(
            &engine,
            "FOR m IN comments FILTER m.n == FIRST(FOR c IN cards FILTER c._key == \"card05\" RETURN c.n) RETURN m",
            true
        ),
        (
            "index_lookup".to_string(),
            Some("idx_comments_n".to_string())
        )
    );
    assert_eq!(
        first_access(
            &engine,
            "FOR m IN comments FILTER m._key IN (FOR x IN comments FILTER x.n == 3 RETURN x._key) RETURN m",
            true
        )
        .0,
        "index_lookup"
    );

    // Execution reads both collections from indexes: no scan, one run.
    let (rows, runs, scans) = run_counted(&engine, &BindVars::new(), in_subquery);
    assert!(!rows.is_empty());
    assert_eq!(
        Ok(rows),
        outcome(&engine, &BindVars::new(), in_subquery, false)
    );
    assert_eq!((runs, scans), (1, 0));

    for q in [
        // Reuse off, and a writing query, plan as before.
        in_subquery,
        // The subquery reads the loop variable.
        "FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.n == m.n RETURN c._key) RETURN m",
        // A Bloom index never answers a subquery-valued key.
        r#"FOR m IN comments FILTER m.body IN (FOR c IN cards FILTER c.n == 2 RETURN CONCAT("comment ", c.n)) RETURN m"#,
        // A range or a LIKE bound by a subquery still scans.
        r#"FOR m IN comments FILTER m.n > FIRST(FOR c IN cards FILTER c._key == "card05" RETURN c.n) RETURN m"#,
        r#"FOR m IN comments FILTER m.card_id LIKE CONCAT(FIRST(FOR c IN cards FILTER c._key == "card05" RETURN SUBSTRING(c._key, 0, 5)), "%") RETURN m"#,
        // Null and numeric zero keys are declined.
        r#"FOR m IN comments FILTER m.card_id == FIRST(FOR c IN cards FILTER c._key == "k4" RETURN c.column_id) RETURN m"#,
        r#"FOR m IN comments FILTER m.n == FIRST(FOR c IN cards FILTER c._key == "k3" RETURN c.n) RETURN m"#,
    ] {
        let reuse = q != in_subquery;
        assert_eq!(first_access(&engine, q, reuse).0, "full_scan", "{q}");
    }
    let writing = r#"FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == "col001" RETURN c._key) UPDATE m WITH {seen: true} IN comments"#;
    assert_eq!(first_access(&engine, writing, true).0, "full_scan");

    let (_, runs, scans) = run_counted(
        &engine,
        &BindVars::new(),
        r#"FOR m IN comments FILTER m.card_id LIKE CONCAT(FIRST(FOR c IN cards FILTER c._key == "card05" RETURN SUBSTRING(c._key, 0, 5)), "%") RETURN m"#,
    );
    assert_eq!((runs, scans), (1, 1));
}

/// A Bloom or Cuckoo filter holds `value.to_string()`, so a stored `2.0` is
/// a definite miss for the key `2`: neither may answer a subquery-valued key.
/// Documents inserted before and after the index both take part.
#[test]
fn probabilistic_indexes_never_answer_a_subquery_key() {
    for kind in [IndexType::Bloom, IndexType::Cuckoo] {
        let (engine, _dir) = engine();
        for c in ["ys", "xs"] {
            engine.create_collection(c.to_string(), None).unwrap();
        }
        let ys = engine.get_collection("ys").unwrap();
        ys.insert(json!({"_key": "y1", "n": 2.0})).unwrap();
        ys.create_index(
            "by_n".to_string(),
            vec!["n".to_string()],
            kind.clone(),
            false,
        )
        .unwrap();
        ys.insert(json!({"_key": "y2", "n": 4.0})).unwrap();
        ys.insert(json!({"_key": "y3", "n": 7})).unwrap();
        let xs = engine.get_collection("xs").unwrap();
        xs.insert(json!({"_key": "x1", "v": 2})).unwrap();
        xs.insert(json!({"_key": "x2", "v": 4})).unwrap();
        for (q, want) in [
            (
                "FOR y IN ys FILTER y.n IN (FOR x IN xs RETURN x.v) RETURN y._key",
                json!(["y1", "y2"]),
            ),
            (
                "FOR y IN ys FILTER y.n == FIRST(FOR x IN xs RETURN x.v) RETURN y._key",
                json!(["y1"]),
            ),
            (
                r#"FOR y IN ys FILTER y.n == FIRST(FOR x IN xs FILTER x._key == "x2" RETURN x.v) RETURN y._key"#,
                json!(["y2"]),
            ),
            (
                "FOR y IN ys FILTER y.n == FIRST(FOR x IN xs RETURN x.v) COLLECT WITH COUNT INTO n RETURN n",
                json!([1]),
            ),
        ] {
            assert_same_with_and_without_reuse(&engine, &BindVars::new(), q);
            let got = outcome(&engine, &BindVars::new(), q, true).unwrap();
            assert_eq!(Value::Array(got), want, "{kind:?} {q}");
        }
    }
}

/// Users, cards and comments with an author, for the run counts below.
fn seed_authors(engine: &StorageEngine) {
    for c in ["users", "cards", "comments"] {
        engine.create_collection(c.to_string(), None).unwrap();
    }
    let users = engine.get_collection("users").unwrap();
    for i in 0..20 {
        users
            .insert(json!({"_key": format!("u{i:02}"), "active": i % 3 != 0}))
            .unwrap();
    }
    let cards = engine.get_collection("cards").unwrap();
    for i in 0..30 {
        cards.insert(json!({"_key": format!("c{i:02}")})).unwrap();
    }
    let comments = engine.get_collection("comments").unwrap();
    for i in 0..200 {
        comments
            .insert(json!({
                "_key": format!("m{i:03}"),
                "card_id": format!("c{:02}", (i * 7) % 33),
                "author": format!("u{:02}", (i * 3) % 23),
            }))
            .unwrap();
    }
}

fn index(engine: &StorageEngine, coll: &str, field: &str) {
    engine
        .get_collection(coll)
        .unwrap()
        .create_index(
            format!("{coll}_{field}"),
            vec![field.to_string()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
}

/// The planner evaluates the subquery node the FILTER re-check evaluates, so
/// an uncorrelated one runs once per query, served by an index or not; and
/// it evaluates nothing for a stage no index can serve.
#[test]
fn a_subquery_key_runs_once_whichever_index_serves() {
    let per_card = "FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key FILTER m.author IN (FOR u IN users FILTER u.active RETURN u._key) RETURN m._key) RETURN ms";
    let mut want = None;
    for indexes in [
        &[][..],
        &[("comments", "card_id")][..],
        &[("comments", "author")][..],
        &[("comments", "card_id"), ("comments", "author")][..],
    ] {
        let (engine, _dir) = engine();
        seed_authors(&engine);
        for (coll, field) in indexes {
            index(&engine, coll, field);
        }
        assert_same_with_and_without_reuse(&engine, &BindVars::new(), per_card);
        let (rows, runs, _) = run_counted(&engine, &BindVars::new(), per_card);
        assert_eq!(runs, 30 + 1, "{indexes:?}");
        assert_eq!(
            want.get_or_insert_with(|| rows.clone()),
            &rows,
            "{indexes:?}"
        );

        let none = r#"FOR m IN comments FILTER m.card_id == "none" AND m.author IN (FOR u IN users RETURN u._key) RETURN m"#;
        let (rows, runs, _) = run_counted(&engine, &BindVars::new(), none);
        assert!(rows.is_empty());
        let author_indexed = indexes.contains(&("comments", "author"));
        let card_indexed = indexes.contains(&("comments", "card_id"));
        // Run only when the author index could serve the stage, which it
        // does once the card_id index (the planner's choice) is absent.
        let want_runs = u64::from(author_indexed && !card_indexed);
        assert_eq!(runs, want_runs, "{indexes:?}");
    }
}

/// Two leading FILTERs plan on the same nodes as one: the same run count.
#[test]
fn a_second_filter_does_not_rerun_the_subquery_key() {
    let (engine, _dir) = engine();
    seed(&engine);
    add_indexes(&engine);
    let inner = "FOR c IN cards FILTER c.column_id == col._key RETURN c._key";
    let two = format!(
        "FOR col IN cols SORT col._key LET ms = (FOR m IN comments FILTER m.body != null FILTER m.card_id IN ({inner}) SORT m._key RETURN m._key) RETURN {{col: col._key, ms}}"
    );
    let one = format!(
        "FOR col IN cols SORT col._key LET ms = (FOR m IN comments FILTER m.card_id IN ({inner}) SORT m._key RETURN m._key) RETURN {{col: col._key, ms}}"
    );
    for q in [&two, &one] {
        assert_same_with_and_without_reuse(&engine, &BindVars::new(), q);
    }
    let (two_rows, two_runs, _) = run_counted(&engine, &BindVars::new(), &two);
    let (one_rows, one_runs, _) = run_counted(&engine, &BindVars::new(), &one);
    assert_eq!(two_rows, one_rows);
    assert_eq!((two_runs, one_runs), (20, 20));
}

/// A subquery key that fails is left to the scan, which treats the error as
/// before; the planner does not run it again for the same bindings.
#[test]
fn a_failing_subquery_key_is_run_once_by_the_planner() {
    let (engine, _dir) = engine();
    seed_authors(&engine);
    engine.create_collection("empty".to_string(), None).unwrap();
    index(&engine, "empty", "author");
    // 20 × 20 users exceed the ceiling; every collection is under it.
    let q = "FOR o IN 1..5 LET r = (FOR m IN empty FILTER m.author IN (FOR a IN users FOR b IN users RETURN a._key) AND o > 0 RETURN m) RETURN r";
    let query = parse(q).unwrap();
    for reuse in [false, true] {
        let executor = QueryExecutor::new(&engine)
            .with_max_intermediate_rows(100)
            .with_reuse_within_query(reuse);
        let rows = executor.execute(&query).unwrap();
        assert_eq!(rows, vec![json!([]); 5]);
        // Five runs of `r`; reuse adds the planner's one failed run.
        assert_eq!(executor.subquery_runs(), 5 + u64::from(reuse));
    }
}

/// A one-argument builtin reads its argument by reference; the result, and
/// a missing variable's error, are what the owned path gives.
#[test]
fn one_argument_builtins_read_their_argument_in_place() {
    let (engine, _dir) = engine();
    let mut binds = BindVars::new();
    binds.insert("arr".to_string(), json!([7, 8, 9]));
    binds.insert("nul".to_string(), Value::Null);
    let args = [
        "a",
        "o.xs",
        "o.missing",
        "o",
        "s",
        "[1, 2]",
        "\"abc\"",
        "null",
        "@arr",
        "@nul",
    ];
    for f in ["LENGTH", "FIRST", "LAST", "COUNT"] {
        for arg in args {
            let lets = r#"LET a = [1, 2, 3] LET o = {xs: [4, 5]} LET s = "hello""#;
            let by_ref = format!("{lets} RETURN {f}({arg})");
            // An element of a subquery is never resolved by reference (nor
            // folded away): the owned path.
            let owned = format!("{lets} RETURN {f}((FOR i IN [1] RETURN {arg})[0])");
            let got = |q: &str| {
                QueryExecutor::with_bind_vars(&engine, binds.clone())
                    .execute(&parse(q).unwrap())
                    .map_err(|e| e.to_string())
            };
            assert_eq!(got(&by_ref), got(&owned), "{by_ref}");
        }
    }
    assert_eq!(
        run(
            &engine,
            "LET a = [1, 2, 3] RETURN [LENGTH(a), FIRST(a), LAST(a), COUNT(a)]"
        ),
        vec![json!([3, 1, 3, 3])]
    );
    for f in ["LENGTH", "FIRST", "LAST", "COUNT"] {
        let err = QueryExecutor::new(&engine)
            .execute(&parse(&format!("RETURN {f}(nope)")).unwrap())
            .unwrap_err()
            .to_string();
        assert!(err.contains("Variable 'nope' not found"), "{f}: {err}");
    }
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

    let (got, runs, _) = run_counted(&engine, &BindVars::new(), &nested);
    let want = run(&engine, &hoisted);
    assert!(!want.is_empty());
    assert_eq!(got, want);
    // Per-row evaluation is 800 × (1 + 400) subquery runs; reuse is one each.
    assert_eq!(runs, 2, "nested IN subqueries ran {runs} times");

    // NOT IN takes the same path.
    let not_in = run(
        &engine,
        &format!(
            r#"FOR m IN comments FILTER m.commentable_type == "card" AND m.commentable_id NOT IN {card_ids} RETURN 1"#
        ),
    );
    assert_eq!(not_in.len() + want.len(), 640);
}

/// An inner subquery reading only the outer loop's row runs once per outer
/// row, not once per row of the subquery between them (7.2 s to 12 ms on the
/// board).
#[test]
fn subquery_reading_an_outer_row_runs_once_per_outer_row() {
    let (engine, _dir) = engine();
    seed(&engine);
    let q = "FOR col IN cols LET ms = (FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == col._key RETURN c._key) RETURN m._key) RETURN LENGTH(ms)";
    let (rows, runs, _) = run_counted(&engine, &BindVars::new(), q);
    assert_eq!(rows.len(), 10);
    assert!(rows.iter().any(|n| n.as_u64() > Some(0)));
    // Ten columns: the comments subquery and the cards subquery once each
    // per column. Per comment would be 10 × 159.
    assert_eq!(runs, 20);
}

#[test]
fn shared_let_runs_its_subquery_once() {
    let (engine, _dir) = engine();
    seed(&engine);
    let q = "FOR m IN comments LET cs = (FOR c IN cards RETURN c) RETURN LENGTH(cs)";
    let (rows, runs, scans) = run_counted(&engine, &BindVars::new(), q);
    assert_eq!(rows.len(), 159);
    assert!(rows.iter().all(|n| *n == json!(66)));
    assert_eq!(runs, 1);
    // The comments scan and one of cards.
    assert_eq!(scans, 2);
}

/// EXPLAIN binds a LET the way execution does: one subquery run, not one
/// per comment.
#[test]
fn explain_runs_a_shared_let_once() {
    let (engine, _dir) = engine();
    seed(&engine);
    let query = parse("FOR m IN comments LET cs = (FOR c IN cards RETURN c) RETURN 1").unwrap();
    let executor = QueryExecutor::new(&engine);
    executor.explain(&query).unwrap();
    assert_eq!(executor.subquery_runs(), 1);
}

/// `(FOR doc IN cards ...)` inside `FOR doc IN comments` binds its own `doc`
/// before reading it, so it does not depend on the comment.
#[test]
fn subquery_rebinding_the_outer_variable_is_not_correlated() {
    let (engine, _dir) = engine();
    seed(&engine);
    let q = "FOR doc IN comments FILTER doc.card_id IN (FOR doc IN cards FILTER doc.column_id == @col RETURN doc._key) RETURN doc._key";
    let (rows, runs, _) = run_counted(&engine, &binds(), q);
    assert!(!rows.is_empty());
    assert_eq!(runs, 1);
}

#[test]
fn bare_collect_into_reads_the_outer_row() {
    let (engine, _dir) = engine();
    for (q, want) in [
        (
            "FOR x IN [1, 2, 3] RETURN (FOR y IN [10] COLLECT k = y INTO g RETURN g[0].x)",
            json!([[1], [2], [3]]),
        ),
        (
            "FOR x IN [1, 2, 3] RETURN (FOR z IN [0] RETURN (FOR y IN [10] COLLECT k = y INTO g RETURN g[0].x))",
            json!([[[1]], [[2]], [[3]]]),
        ),
        (
            "WITH t AS (FOR x IN [1, 2, 3] RETURN (FOR y IN [10] COLLECT k = y INTO g RETURN g[0].x)) FOR r IN t RETURN r",
            json!([[1], [2], [3]]),
        ),
    ] {
        assert_eq!(Value::Array(run(&engine, q)), want, "{q}");
    }
}

#[test]
fn backtick_at_name_is_read_per_row() {
    let (engine, _dir) = engine();
    assert_eq!(
        run(
            &engine,
            "FOR x IN [[1], [2]] LET `@ids` = x RETURN (FOR d IN 1..3 FILTER d IN @ids RETURN d)"
        ),
        vec![json!([1]), json!([2])]
    );
}

/// A lambda parameter rebound in place must never look unchanged.
#[test]
fn lambda_parameter_is_read_per_element() {
    let (engine, _dir) = engine();
    assert_eq!(
        run(
            &engine,
            "RETURN MAP([1, 1, 2, 2, 3], x -> FIRST(FOR i IN [x] RETURN i))"
        ),
        vec![json!([1, 1, 2, 2, 3])]
    );
}

#[test]
fn left_operand_error_wins_over_the_subquery() {
    let (engine, _dir) = engine();
    seed(&engine);
    for q in [
        "FOR c IN cols RETURN undefinedVar IN (FOR k IN cards FILTER k.column_id == c._key RETURN k.x[true])",
        "FOR c IN cols RETURN undefinedVar IN (FOR k IN cards RETURN k.x[true])",
    ] {
        for reuse in [true, false] {
            let err = outcome(&engine, &BindVars::new(), q, reuse).unwrap_err();
            assert!(
                err.contains("Variable 'undefinedVar' not found"),
                "{q} (reuse {reuse}): {err}"
            );
        }
    }
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

/// Columns, cards, comments and reactions with no index, for the hash
/// probe's counters. Card `c00` has several comments, and comes first.
fn seed_probe(engine: &StorageEngine) {
    for c in ["cols", "cards", "comments", "reactions", "xs", "ys", "docs"] {
        engine.create_collection(c.to_string(), None).unwrap();
    }
    let insert = |coll: &str, docs: Vec<Value>| {
        let coll = engine.get_collection(coll).unwrap();
        for doc in docs {
            coll.insert(doc).unwrap();
        }
    };
    insert(
        "cols",
        (0..5).map(|i| json!({"_key": format!("col{i}")})).collect(),
    );
    insert(
        "cards",
        (0..30)
            .map(|i| {
                json!({
                    "_key": format!("c{i:02}"),
                    "column_id": format!("col{}", i % 5),
                    "owner": format!("u{}", i % 4),
                })
            })
            .collect(),
    );
    insert(
        "comments",
        (0..200)
            .map(|i| {
                let card = format!("c{:02}", (i * 7) % 33);
                json!({
                    "_key": format!("m{i:03}"),
                    "card_id": card,
                    "commentable_type": if i % 5 == 0 { "doc" } else { "card" },
                    "commentable_id": card,
                    "author": format!("u{}", i % 4),
                })
            })
            .collect(),
    );
    insert(
        "reactions",
        (0..60)
            .map(|i| json!({"_key": format!("r{i:02}"), "comment_id": format!("m{:03}", (i * 3) % 200)}))
            .collect(),
    );
    insert(
        "xs",
        (0..3)
            .map(|i| json!({"_key": format!("x{i}"), "k": i}))
            .collect(),
    );
    insert(
        "ys",
        (0..10)
            .map(|i| json!({"_key": format!("y{i}"), "bucket": 0, "k": i % 3}))
            .collect(),
    );
    insert(
        "docs",
        (0..8)
            .map(|i| json!({"_key": format!("d{i}"), "bucket": i % 4}))
            .collect(),
    );
}

/// Results equal to reuse off, and the scans and re-checked candidates.
fn probe_counted(engine: &StorageEngine, q: &str) -> (u64, u64) {
    assert_same_with_and_without_reuse(engine, &BindVars::new(), q);
    let query = parse(q).unwrap();
    let executor = executor(engine, &BindVars::new(), true);
    executor.execute(&query).unwrap();
    (executor.collection_scans(), executor.probe_candidates())
}

/// A FOR that would scan its collection once per card reads it once per
/// query: the cards, the first card's scan, the build.
#[test]
fn hash_probe_reads_a_correlated_collection_once() {
    let (engine, _dir) = engine();
    seed_probe(&engine);
    let per_card = "FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key RETURN m._key) RETURN ms";
    let (scans, candidates) = probe_counted(&engine, per_card);
    assert_eq!(scans, 3);
    // Each comment is a candidate for its own card only.
    assert!(candidates <= 200, "{candidates}");

    // A nested FOR at the top level: rows from the first run on.
    let join = "FOR c IN cols FOR card IN cards FILTER card.column_id == c._key RETURN [c._key, card._key]";
    assert_eq!(probe_counted(&engine, join), (2, 30));

    // Reuse off scans per card.
    let query = parse(per_card).unwrap();
    let off = executor(&engine, &BindVars::new(), false);
    off.execute(&query).unwrap();
    assert_eq!((off.collection_scans(), off.probe_candidates()), (31, 0));
}

/// A key that is not deterministic is drawn once per (row, document) by the
/// scan; the probe would draw it once per row and again per candidate.
#[test]
fn hash_probe_never_keys_on_a_nondeterministic_value() {
    let (engine, _dir) = engine();
    seed_probe(&engine);
    for q in [
        "FOR a IN xs FOR b IN ys FILTER b.bucket == RAND_INT(0, 0) RETURN [a._key, b._key]",
        "FOR a IN xs FOR b IN ys FILTER b.bucket == DATE_NOW() * 0 RETURN [a._key, b._key]",
    ] {
        assert_eq!(probe_counted(&engine, q).0, 4, "{q}");
    }
    // Another conjunct can still be the key.
    let q = "FOR a IN xs FOR b IN ys FILTER b.bucket == RAND_INT(0, 0) AND b.k == a.k RETURN [a._key, b._key]";
    assert_eq!(probe_counted(&engine, q).0, 2);

    let query =
        parse("FOR i IN 1..2 FOR d IN docs FILTER d.bucket == FLOOR(RAND() * 4) RETURN d._key")
            .unwrap();
    let on = executor(&engine, &BindVars::new(), true);
    on.execute(&query).unwrap();
    assert_eq!(on.collection_scans(), 2);
    let twin = "FOR i IN 1..2 FOR d IN docs FILTER d.bucket == i RETURN d._key";
    assert_eq!(probe_counted(&engine, twin).0, 1);
}

/// A constant discriminator first in the FILTER does not become the key:
/// each row probes the term with the fewest candidates.
#[test]
fn hash_probe_keys_each_row_on_its_most_selective_term() {
    let (engine, _dir) = engine();
    seed_probe(&engine);
    for q in [
        r#"FOR card IN cards LET ms = (FOR m IN comments FILTER m.commentable_type == "card" AND m.commentable_id == card._key RETURN m._key) RETURN ms"#,
        r#"FOR card IN cards LET ms = (FOR m IN comments FILTER m.commentable_type == "card" FILTER m.commentable_id == card._key RETURN m._key) RETURN ms"#,
    ] {
        let (scans, candidates) = probe_counted(&engine, q);
        assert!(scans <= 3, "{q}: {scans}");
        // Keyed on the type it would be ~29 cards × 160 card comments.
        assert!(candidates <= 200, "{q}: {candidates}");
    }
    // A selective constant IN list after a low-cardinality correlated term.
    let q = r#"FOR card IN cards LET ms = (FOR m IN comments FILTER m.author == card.owner AND m._key IN ["m001", "m012", "m023"] RETURN m._key) RETURN ms"#;
    let (_, candidates) = probe_counted(&engine, q);
    assert!(candidates <= 29 * 3, "{candidates}");
}

/// Document vectors are shared by FORs that read the same fields (or whole
/// documents), and all of them together stay under the row ceiling.
#[test]
fn hash_probe_shares_and_bounds_what_it_holds() {
    let (engine, _dir) = engine();
    seed_probe(&engine);
    // Both FORs read whole documents: one build serves both. A LET runs for
    // every row before the next LET does, so the second FOR finds the
    // vector already built and never scans: the cards, the first card's
    // scan, the build.
    let whole = "FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key RETURN m) LET mine = (FOR m IN comments FILTER m.author == card.owner RETURN m) RETURN {ms, mine}";
    assert_eq!(probe_counted(&engine, whole).0, 3);

    // Two projections need two vectors, and only one fits: the second FOR
    // scans, every run, with no error.
    let projected = "FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key RETURN m._key) LET mine = (FOR m IN comments FILTER m.author == card.owner RETURN m._key) RETURN {ms, mine}";
    let query = parse(projected).unwrap();
    let mut results = Vec::new();
    for reuse in [false, true] {
        let executor = QueryExecutor::new(&engine)
            .with_max_intermediate_rows(250)
            .with_reuse_within_query(reuse);
        results.push(executor.execute(&query).unwrap());
        if reuse {
            // Cards; `ms` scanned once, then built; `mine` scanned for
            // every card, its build declined.
            assert_eq!(executor.collection_scans(), 1 + 2 + 30);
        }
    }
    assert_eq!(results[0], results[1]);

    // A single projected subquery is served from its own vector.
    let one = "FOR card IN cards LET ms = (FOR m IN comments FILTER m.author == card.owner RETURN m._key) RETURN ms";
    assert_eq!(probe_counted(&engine, one).0, 3);
}

/// Two FORs that each run once keep their own projected scans: only a FOR
/// that scanned before (or has rows to probe) builds a table.
#[test]
fn hash_probe_leaves_one_shot_fors_to_the_scan() {
    let (engine, _dir) = engine();
    seed_probe(&engine);
    let q = r#"LET a = (FOR t IN comments FILTER t.author == "u1" RETURN t._key) LET b = (FOR t IN comments FILTER t.author == "u2" RETURN t._key) RETURN [LENGTH(a), LENGTH(b)]"#;
    assert_eq!(probe_counted(&engine, q), (2, 0));
}

/// A key or a re-check that runs a subquery whose FOR probes too, on the
/// same executor: no lock is held across it (a regression hangs here), and
/// each collection is built once.
#[test]
fn hash_probe_nests() {
    let (engine, _dir) = engine();
    seed_probe(&engine);
    for (q, want) in [
        // cols; cards built by the outer FOR, reused by the inner one.
        ("FOR a IN cols FOR b IN cards FILTER b.column_id == FIRST(FOR z IN cards FILTER z.column_id == a._key RETURN z.column_id) RETURN b._key", 2),
        // cols; comments built; cards scanned once, then built.
        ("FOR col IN cols FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == col._key RETURN c._key) RETURN [col._key, m._key]", 4),
        // cols; cards built; comments scanned once, then built.
        ("FOR col IN cols FOR c IN cards FILTER c.column_id == col._key FILTER LENGTH(FOR m IN comments FILTER m.card_id == c._key RETURN 1) > 0 RETURN c._key", 4),
        // cards; comments scanned, reactions scanned then built (first
        // card's scan path); comments built.
        ("FOR card IN cards LET ms = (FOR m IN comments FILTER m.card_id == card._key FILTER LENGTH(FOR r IN reactions FILTER r.comment_id == m._key RETURN 1) > 0 RETURN m._key) RETURN ms", 5),
    ] {
        assert_eq!(probe_counted(&engine, q).0, want, "{q}");
    }
    let target2 = "FOR col IN cols SORT col._key LET ms = (FOR m IN comments FILTER m.card_id IN (FOR c IN cards FILTER c.column_id == col._key RETURN c._key) FILTER m.author != null SORT m._key RETURN m._key) RETURN {col: col._key, ms}";
    let (scans, _) = probe_counted(&engine, target2);
    assert!(scans <= 5, "{scans}");
}
