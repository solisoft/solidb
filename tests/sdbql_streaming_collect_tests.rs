//! A full-scan `FOR … [FILTER | LET]* COLLECT …` is folded while the
//! collection is read (`execution::scan_collect`), instead of materialising
//! every document as a row first. These tests run each query both ways —
//! `with_streaming_collect(true)` against `(false)`, the materialising path —
//! on the same data and require the same answer, error included; check that
//! the streaming path is the one that ran (EXPLAIN says so); and check that it
//! holds O(groups) rows: under a row ceiling far below the collection's size,
//! it answers where the materialising path is stopped by the ceiling.

use serde_json::{json, Value};
use solidb::sdbql::QueryPrincipal;
use solidb::storage::StorageEngine;
use solidb::{parse, BindVars, QueryExecutor};
use tempfile::TempDir;

const DB: &str = "db";
const FOLDED: &str = "COLLECT folded into the scan";

/// Group keys of every kind, including the pairs `==` holds equal (`1` and
/// `1.0`, `-0.0` and `0`, `[1, 2]` and `[1.0, 2]`, missing and null) and the
/// ones it does not (`1` and `"1"`); values SUM / AVG / MIN / MAX must skip or
/// order (null, strings, booleans, arrays, objects, overflowing floats).
fn mixed_docs() -> Vec<Value> {
    let keys = [
        json!(null),
        json!(1),
        json!(1.0),
        json!(2),
        json!(-0.0),
        json!(0),
        json!("a"),
        json!("b"),
        json!("1"),
        json!(true),
        json!(false),
        json!([1, 2]),
        json!([1.0, 2]),
        json!({"x": 1}),
        json!({"x": 1.0}),
    ];
    let values = [
        json!(10),
        json!(2.5),
        json!(null),
        json!("5"),
        json!(true),
        json!([1]),
        json!({"a": 1}),
        json!(-3),
        json!(1e308),
        json!(7),
        json!("zz"),
    ];
    let mut docs = Vec::new();
    let ints = [json!(12), json!(10.0), json!(null), json!(3), json!("x")];
    for n in 0..66usize {
        let (v, i, t) = (
            &values[n % values.len()],
            &ints[n % 5],
            ["red", "blue"][n % 2],
        );
        let mut doc = json!({
            "_key": format!("d{n:03}"),
            "v": v,
            "i": i,
            "t": t,
            "nested": {"deep": n % 3, "list": [n, n + 1]},
            "padding": "p".repeat(40),
        });
        // Every seventh document has no `k` at all.
        if n % 7 != 3 {
            doc["k"] = keys[n % keys.len()].clone();
        }
        if n % 11 == 0 {
            doc["a.b"] = json!("literal");
        } else if n % 11 == 1 {
            doc["a"] = json!({"b": "path"});
        }
        docs.push(doc);
    }
    docs
}

fn engine_with(docs: Vec<Value>) -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection("c".to_string(), None).unwrap();
    db.get_collection("c").unwrap().insert_batch(docs).unwrap();
    db.create_collection("users".to_string(), None).unwrap();
    db.get_collection("users")
        .unwrap()
        .insert_batch(vec![
            json!({"_key": "u1", "name": "Ann", "k": 1}),
            json!({"_key": "u2", "name": "Bob", "k": "a"}),
            json!({"_key": "u3", "name": "Cy", "k": null}),
            json!({"_key": "u4", "name": "Di", "k": "none"}),
        ])
        .unwrap();
    (engine, tmp)
}

fn executor<'a>(engine: &'a StorageEngine, stream: bool, binds: &BindVars) -> QueryExecutor<'a> {
    QueryExecutor::with_database_and_bind_vars(engine, DB.to_string(), binds.clone())
        .with_streaming_collect(stream)
}

fn run(
    engine: &StorageEngine,
    q: &str,
    stream: bool,
    binds: &BindVars,
) -> Result<Vec<Value>, String> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    executor(engine, stream, binds)
        .execute(&query)
        .map_err(|e| e.to_string())
}

/// Whether EXPLAIN (analyze) says the scan was folded into the COLLECT.
fn folded(engine: &StorageEngine, q: &str, binds: &BindVars) -> bool {
    let query = parse(q).unwrap();
    executor(engine, true, binds)
        .explain(&query)
        .unwrap()
        .warnings
        .iter()
        .any(|w| w.contains(FOLDED))
}

/// The query's answer, after checking the streaming path gives exactly the
/// materialising path's (order and errors included) and that it is the
/// path that ran.
fn same_with(engine: &StorageEngine, q: &str, binds: &BindVars) -> Result<Vec<Value>, String> {
    let streamed = run(engine, q, true, binds);
    let materialised = run(engine, q, false, binds);
    assert_eq!(streamed, materialised, "{q}");
    if streamed.is_ok() {
        assert!(folded(engine, q, binds), "not folded: {q}");
    }
    streamed
}

fn same(engine: &StorageEngine, q: &str) -> Vec<Value> {
    same_with(engine, q, &BindVars::new()).unwrap_or_else(|e| panic!("{q}: {e}"))
}

#[test]
fn group_keys_of_every_kind() {
    let (engine, _tmp) = engine_with(mixed_docs());
    let groups = same(
        &engine,
        "FOR d IN c COLLECT k = d.k WITH COUNT INTO n RETURN {k, n}",
    );
    // Missing and null are one group; 1 and 1.0, -0.0 and 0, [1, 2] and
    // [1.0, 2], {x: 1} and {x: 1.0} are one each; "1" is its own. In AQL
    // order: null, false, true, numbers, strings, arrays, objects.
    let keys: Vec<Value> = groups.iter().map(|g| g["k"].clone()).collect();
    assert_eq!(keys.len(), 11, "{groups:?}");
    assert_eq!(keys[0], Value::Null);
    assert_eq!(keys[1], json!(false));
    assert_eq!(keys[2], json!(true));
    assert_eq!(&keys[6..9], &[json!("1"), json!("a"), json!("b")]);
    let total: i64 = groups.iter().map(|g| g["n"].as_i64().unwrap()).sum();
    assert_eq!(total, 66);

    same(&engine, "FOR d IN c COLLECT k = d.k RETURN k");
    same(&engine, "FOR d IN c COLLECT k = d.k, t = d.t RETURN [k, t]");
    same(
        &engine,
        "FOR d IN c COLLECT k = d.nested.deep, t = d.t WITH COUNT INTO n RETURN {k, t, n}",
    );
    same(
        &engine,
        "FOR d IN c COLLECT l = d.nested.list[0] % 4 RETURN l",
    );
    same(&engine, "FOR d IN c COLLECT k = d._key RETURN k");
}

#[test]
fn every_aggregate_over_mixed_values() {
    let (engine, _tmp) = engine_with(mixed_docs());
    let aggregates = "n = COUNT(), cn = COUNT(d.v), len = LENGTH(d.v), s = SUM(d.v), \
         a = AVG(d.v), mn = MIN(d.v), mx = MAX(d.v), cd = COUNT_DISTINCT(d.v), \
         u = UNIQUE(d.v), su = SORTED_UNIQUE(d.v), l = COLLECT_LIST(d.v), \
         var = VARIANCE(d.v), vs = VARIANCE_SAMPLE(d.v), sd = STDDEV(d.v), \
         sds = STDDEV_SAMPLE(d.v), med = MEDIAN(d.v), ba = BIT_AND(d.i), \
         bo = BIT_OR(d.i), bx = BIT_XOR(d.i), mi = MIN(d.i), mxs = MAX(d.t)";
    for q in [
        format!("FOR d IN c COLLECT k = d.k AGGREGATE {aggregates} RETURN MERGE({{k}}, {{n, cn, len, s, a, mn, mx, cd, u, su, l, var, vs, sd, sds, med, ba, bo, bx, mi, mxs}})"),
        format!("FOR d IN c COLLECT AGGREGATE {aggregates} RETURN {{n, cn, len, s, a, mn, mx, cd, u, su, l, var, vs, sd, sds, med, ba, bo, bx, mi, mxs}}"),
        format!("FOR d IN c COLLECT t = d.t AGGREGATE {aggregates} RETURN {{t, n, s, a, mn, mx, l}}"),
    ] {
        same(&engine, &q);
    }

    // SUM of an empty group is 0, AVG null; with no group variable an
    // empty input still yields its one row.
    let empty = same(
        &engine,
        "FOR d IN c FILTER d.v == 'nothing' COLLECT AGGREGATE s = SUM(d.v), a = AVG(d.v), \
         m = MIN(d.v), n = COUNT() RETURN {s, a, m, n}",
    );
    assert_eq!(empty, vec![json!({"s": 0, "a": null, "m": null, "n": 0})]);
    assert_eq!(
        same(
            &engine,
            "FOR d IN c FILTER false COLLECT WITH COUNT INTO n RETURN n"
        ),
        vec![json!(0)]
    );
    assert!(same(&engine, "FOR d IN c FILTER false COLLECT k = d.k RETURN k").is_empty());

    // Overflowing SUM is null, as JSON has no infinity.
    let overflow = same(
        &engine,
        "FOR d IN c FILTER d.v > 1000000000 COLLECT AGGREGATE s = SUM(d.v) RETURN s",
    );
    assert_eq!(overflow, vec![Value::Null]);
}

#[test]
fn filters_lets_and_bind_variables() {
    let (engine, _tmp) = engine_with(mixed_docs());
    same(
        &engine,
        "FOR d IN c FILTER IS_NUMBER(d.v) COLLECT k = d.k AGGREGATE s = SUM(d.v) RETURN {k, s}",
    );
    same(
        &engine,
        "FOR d IN c FILTER d.t == 'red' FILTER d.v != null COLLECT WITH COUNT INTO n RETURN n",
    );
    same(
        &engine,
        "FOR d IN c LET x = IS_NUMBER(d.v) ? d.v * 2 : 0 FILTER x > 2 LET y = x + 1 \
         COLLECT t = d.t AGGREGATE s = SUM(y), m = MAX(x) RETURN {t, s, m}",
    );
    // A FILTER that reads a name a later LET binds sees it unbound.
    same_with(
        &engine,
        "FOR d IN c FILTER y == null LET y = d.v COLLECT WITH COUNT INTO n RETURN n",
        &BindVars::new(),
    )
    .ok();
    // A FILTER whose condition fails to evaluate drops the row.
    same(
        &engine,
        "FOR d IN c FILTER d.v.missing.deep[0] > 1 COLLECT WITH COUNT INTO n RETURN n",
    );
    let mut binds = BindVars::new();
    binds.insert("min".to_string(), json!(2));
    binds.insert("tag".to_string(), json!("blue"));
    same_with(
        &engine,
        "FOR d IN c FILTER d.v >= @min AND d.t == @tag COLLECT k = d.k \
         AGGREGATE s = SUM(d.v) RETURN {k, s}",
        &binds,
    )
    .unwrap();
    // `doc["a.b"]` reads that key, or the path a → b where there is none.
    let dotted = same(
        &engine,
        "FOR d IN c COLLECT x = d[\"a.b\"] WITH COUNT INTO n RETURN {x, n}",
    );
    assert!(
        dotted.contains(&json!({"x": "literal", "n": 6})),
        "{dotted:?}"
    );
    assert!(dotted.contains(&json!({"x": "path", "n": 6})), "{dotted:?}");
    // A subquery in the FILTER: whole documents, still folded.
    same(
        &engine,
        "FOR d IN c FILTER d.k IN (FOR u IN users RETURN u.k) \
         COLLECT k = d.k WITH COUNT INTO n RETURN {k, n}",
    );
    // Whole documents, used as such.
    same(
        &engine,
        "FOR d IN c FILTER HAS(d, 'k') COLLECT n = LENGTH(ATTRIBUTES(d)) RETURN n",
    );
}

/// `d["a.b"]` reads the key "a.b", or the path a → b where there is none.
/// A scan that decodes only the fields a query reads (not a COLLECT here)
/// used to keep "a.b" and drop "a", so the path read null.
#[test]
fn a_projected_scan_reads_dotted_keys() {
    let (engine, _tmp) = engine_with(mixed_docs());
    let binds = BindVars::new();
    let path = run(
        &engine,
        "FOR d IN c FILTER d[\"a.b\"] == 'path' RETURN d._key",
        true,
        &binds,
    )
    .unwrap();
    assert_eq!(path.len(), 6, "{path:?}");
    let literal = run(
        &engine,
        "FOR d IN c FILTER d[\"a.b\"] == 'literal' RETURN d._key",
        true,
        &binds,
    )
    .unwrap();
    assert_eq!(literal.len(), 6, "{literal:?}");
}

#[test]
fn into_keep_and_projections() {
    let (engine, _tmp) = engine_with(mixed_docs());
    same(&engine, "FOR d IN c COLLECT t = d.t INTO g RETURN {t, g}");
    same(
        &engine,
        "FOR d IN c COLLECT k = d.k INTO keys = d._key RETURN {k, keys}",
    );
    same(
        &engine,
        "LET base = 1 FOR d IN c LET v = d.v COLLECT t = d.t INTO g KEEP v RETURN {t, g}",
    );
    same(
        &engine,
        "LET base = 1 FOR d IN c COLLECT t = d.t INTO g RETURN {t, g}",
    );
    // KEEP of a name not in scope is an error, on both paths.
    let err = same_with(
        &engine,
        "FOR d IN c COLLECT t = d.t INTO g KEEP nope RETURN g",
        &BindVars::new(),
    )
    .unwrap_err();
    assert!(err.contains("KEEP variable"), "{err}");
}

#[test]
fn after_the_collect() {
    let (engine, _tmp) = engine_with(mixed_docs());
    same(
        &engine,
        "FOR d IN c COLLECT k = d.k WITH COUNT INTO n SORT n DESC, k LIMIT 3 RETURN {k, n}",
    );
    same(
        &engine,
        "FOR d IN c COLLECT t = d.t AGGREGATE n = COUNT() LET half = n / 2 FILTER half > 1 \
         RETURN {t, half}",
    );
    same(
        &engine,
        "FOR d IN c COLLECT t = d.t AGGREGATE n = COUNT() \
         FOR u IN users FILTER u.k == 1 RETURN {t, n, u: u.name}",
    );
    // A pre-FOR binding is out of scope after the COLLECT, on both paths.
    let _ = same_with(
        &engine,
        "LET x = 5 FOR d IN c COLLECT t = d.t RETURN {t, x}",
        &BindVars::new(),
    );
    // Errors are the same error.
    assert!(same_with(
        &engine,
        "FOR d IN c COLLECT t = d.t AGGREGATE s = NOPE(d.v) RETURN s",
        &BindVars::new(),
    )
    .is_err());
}

/// A LIMIT after a COLLECT caps the groups. It used to be pushed into the
/// scan, which then grouped only the first documents (both paths had it).
#[test]
fn limit_after_collect_caps_groups_not_documents() {
    let (engine, _tmp) = engine_with(mixed_docs());
    let all = same(&engine, "FOR d IN c COLLECT k = d.k RETURN k");
    let two = same(&engine, "FOR d IN c COLLECT k = d.k LIMIT 2 RETURN k");
    assert_eq!(two, all[..2].to_vec());
    let skip = same(&engine, "FOR d IN c COLLECT k = d.k LIMIT 3, 2 RETURN k");
    assert_eq!(skip, all[3..5].to_vec());
    assert_eq!(
        same(
            &engine,
            "FOR d IN c COLLECT WITH COUNT INTO n LIMIT 1 RETURN n"
        ),
        vec![json!(66)]
    );
    let query = parse("FOR d IN c COLLECT k = d.k LIMIT 2 RETURN k").unwrap();
    let explain = executor(&engine, false, &BindVars::new())
        .explain(&query)
        .unwrap();
    assert_eq!(explain.collections[0].access_type, "full_scan");
    assert_eq!(explain.documents_scanned, 66);
}

/// The loop folded into the COLLECT can sit inside another loop, or a
/// subquery run once per row.
#[test]
fn inner_loops_and_subqueries() {
    let (engine, _tmp) = engine_with(mixed_docs());
    let binds = BindVars::new();
    for reuse in [true, false] {
        for q in [
            "FOR u IN users FOR d IN c FILTER d.k == u.k \
             COLLECT name = u.name AGGREGATE s = SUM(d.v), n = COUNT() RETURN {name, s, n}",
            "FOR u IN users LET s = (FOR d IN c FILTER d.k == u.k \
             COLLECT AGGREGATE t = SUM(d.v) RETURN t)[0] RETURN {u: u._key, s}",
            "RETURN (FOR d IN c COLLECT t = d.t WITH COUNT INTO n RETURN {t, n})",
        ] {
            let query = parse(q).unwrap();
            let on = executor(&engine, true, &binds)
                .with_reuse_within_query(reuse)
                .execute(&query)
                .unwrap();
            let off = executor(&engine, false, &binds)
                .with_reuse_within_query(reuse)
                .execute(&query)
                .unwrap();
            assert_eq!(on, off, "reuse {reuse}: {q}");
        }
    }
}

/// A collection without a database context (the legacy storage API).
#[test]
fn without_a_database_context() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_collection("sales".to_string(), None).unwrap();
    engine
        .get_collection("sales")
        .unwrap()
        .insert_batch(mixed_docs())
        .unwrap();
    let q = "FOR d IN sales COLLECT k = d.k AGGREGATE s = SUM(d.v), n = COUNT() RETURN {k, s, n}";
    let query = parse(q).unwrap();
    let on = QueryExecutor::new(&engine).execute(&query).unwrap();
    let off = QueryExecutor::new(&engine)
        .with_streaming_collect(false)
        .execute(&query)
        .unwrap();
    assert_eq!(on, off);
    let explain = QueryExecutor::new(&engine).explain(&query).unwrap();
    assert!(explain.warnings.iter().any(|w| w.contains(FOLDED)));
}

#[test]
fn explain_reports_the_fold() {
    let (engine, _tmp) = engine_with(mixed_docs());
    let binds = BindVars::new();
    let q = "FOR d IN c FILTER d.t == 'red' COLLECT k = d.k AGGREGATE s = SUM(d.v) RETURN {k, s}";
    let query = parse(q).unwrap();
    let groups = run(&engine, q, true, &binds).unwrap();

    let ex = executor(&engine, true, &binds);
    let explain = ex.explain(&query).unwrap();
    assert_eq!(explain.collections[0].access_type, "full_scan");
    let note = explain
        .warnings
        .iter()
        .find(|w| w.contains(FOLDED))
        .unwrap_or_else(|| panic!("{:?}", explain.warnings));
    assert!(
        note.contains("'c'") && note.contains("reading only k, t, v from the stored bytes"),
        "{note}"
    );
    // Without the typed reads, the fields are decoded.
    let generic = executor(&engine, true, &binds)
        .with_typed_collect(false)
        .explain(&query)
        .unwrap();
    assert!(
        generic
            .warnings
            .iter()
            .any(|w| w.contains(FOLDED) && w.contains("decoding only k, t, v")),
        "{:?}",
        generic.warnings
    );
    assert_eq!(explain.documents_scanned, 66);
    assert_eq!(explain.documents_returned, groups.len());
    assert_eq!(explain.filters.len(), 1);
    assert_eq!(explain.filters[0].documents_before, 66);
    assert_eq!(explain.filters[0].documents_after, 33);

    // Turned off, EXPLAIN shows the materialising scan, and the COLLECT
    // that follows it: the rows returned are the groups either way.
    let off = executor(&engine, false, &binds).explain(&query).unwrap();
    assert!(!off.warnings.iter().any(|w| w.contains(FOLDED)));
    assert_eq!(off.documents_returned, groups.len());

    // Without running it.
    let plan = ex.explain_plan(&query).unwrap();
    assert!(
        plan.warnings.iter().any(|w| w.contains(FOLDED)),
        "{:?}",
        plan.warnings
    );

    // One collection read.
    let ex = executor(&engine, true, &binds);
    ex.execute(&query).unwrap();
    assert_eq!(ex.collection_scans(), 1);

    // Whole documents when the clauses need them.
    let whole = parse("FOR d IN c COLLECT t = d.t INTO g RETURN {t, g}").unwrap();
    let explain = executor(&engine, true, &binds).explain(&whole).unwrap();
    assert!(
        explain
            .warnings
            .iter()
            .any(|w| w.contains(FOLDED) && w.contains("whole documents")),
        "{:?}",
        explain.warnings
    );
}

/// Shapes that keep the materialising path: same answer, nothing folded.
#[test]
fn shapes_left_to_the_materialising_path() {
    let (engine, _tmp) = engine_with(mixed_docs());
    let binds = BindVars::new();
    let db = engine.get_database(DB).unwrap();
    db.get_collection("c")
        .unwrap()
        .create_index(
            "idx_t".to_string(),
            vec!["t".to_string()],
            solidb::IndexType::Persistent,
            false,
        )
        .unwrap();
    for q in [
        // Served by an index.
        "FOR d IN c FILTER d.t == 'red' COLLECT k = d.k WITH COUNT INTO n RETURN {k, n}",
        // Not a collection, or not a plain FOR / FILTER / LET body.
        "FOR x IN 1..50 COLLECT m = x % 3 WITH COUNT INTO n RETURN {m, n}",
        "LET c = [1, 2, 2] FOR x IN c COLLECT v = x WITH COUNT INTO n RETURN {v, n}",
        "FOR d IN c FOR e IN d.nested.list COLLECT m = e % 2 WITH COUNT INTO n RETURN {m, n}",
        "FOR d IN c SEARCH d.t == 'red' COLLECT k = d.k WITH COUNT INTO n RETURN {k, n}",
    ] {
        let on = run(&engine, q, true, &binds);
        let off = run(&engine, q, false, &binds);
        assert_eq!(on, off, "{q}");
        assert!(on.is_ok(), "{q}: {on:?}");
        assert!(!folded(&engine, q, &binds), "folded: {q}");
    }
}

/// A row policy is applied to each document before it is grouped, as the
/// scan applies it.
#[test]
fn row_policy_is_applied_per_document() {
    const TENANTS: [&str; 2] = ["acme", "other"];
    const STATUSES: [&str; 3] = ["a", "b", "c"];
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine
        .create_collection("orders".to_string(), None)
        .unwrap();
    engine
        .get_collection("orders")
        .unwrap()
        .insert_batch(
            (0..40)
                .map(|i| json!({"tenant": TENANTS[i % 2], "status": STATUSES[i % 3], "n": i}))
                .collect(),
        )
        .unwrap();
    let admin = QueryPrincipal::from_roles("root", vec!["admin".to_string()]);
    let viewer = QueryPrincipal {
        user: "ada".into(),
        roles: vec!["viewer".into()],
        can_read: true,
        can_write: false,
        can_admin: false,
    };
    QueryExecutor::new(&engine)
        .with_principal(admin)
        .execute(&parse(r#"RETURN ROW_POLICY("orders", "doc.tenant == \"acme\"")"#).unwrap())
        .unwrap();
    let q =
        "FOR o IN orders COLLECT s = o.status AGGREGATE n = COUNT(), t = SUM(o.n) RETURN {s, n, t}";
    let query = parse(q).unwrap();
    let run_as = |stream: bool| {
        QueryExecutor::new(&engine)
            .with_principal(viewer.clone())
            .with_streaming_collect(stream)
            .execute(&query)
            .unwrap()
    };
    let on = run_as(true);
    assert_eq!(on, run_as(false));
    let total: i64 = on.iter().map(|g| g["n"].as_i64().unwrap()).sum();
    assert_eq!(total, 20, "only acme's rows: {on:?}");
    let explain = QueryExecutor::new(&engine)
        .with_principal(viewer.clone())
        .explain(&query)
        .unwrap();
    assert!(
        explain
            .warnings
            .iter()
            .any(|w| w.contains(FOLDED) && w.contains("whole documents")),
        "{:?}",
        explain.warnings
    );
}

/// A columnar collection and a search view read other storage: left to the
/// materialising path, which reads them.
#[test]
fn columnar_and_search_views_are_left_alone() {
    use solidb::storage::columnar::{ColumnDef, ColumnType, ColumnarCollection, CompressionType};
    let (engine, _tmp) = engine_with(mixed_docs());
    let database = engine.get_database(DB).unwrap();
    let db_arc = database.db_arc();
    let cf_name = format!("{DB}:_columnar_metrics");
    if db_arc.cf_handle(&cf_name).is_none() {
        db_arc.create_cf(&cf_name).unwrap();
    }
    let column = |name: &str, data_type| ColumnDef {
        name: name.to_string(),
        data_type,
        nullable: false,
        indexed: false,
        index_type: None,
    };
    ColumnarCollection::new(
        "metrics".to_string(),
        DB,
        db_arc,
        vec![
            column("host", ColumnType::String),
            column("value", ColumnType::Float64),
        ],
        CompressionType::Lz4,
    )
    .unwrap()
    .insert_rows(vec![
        json!({"host": "a", "value": 10.0}),
        json!({"host": "b", "value": 20.0}),
        json!({"host": "a", "value": 30.0}),
    ])
    .unwrap();
    let binds = BindVars::new();
    let q = "FOR m IN metrics COLLECT h = m.host AGGREGATE s = SUM(m.value) RETURN {h, s}";
    let on = run(&engine, q, true, &binds).unwrap();
    assert_eq!(on, run(&engine, q, false, &binds).unwrap());
    assert_eq!(
        on,
        vec![json!({"h": "a", "s": 40.0}), json!({"h": "b", "s": 20.0})]
    );
    assert!(!folded(&engine, q, &binds));

    let editor = QueryPrincipal::from_roles("tester", vec!["editor".to_string()]);
    QueryExecutor::with_database(&engine, DB.to_string())
        .with_principal(editor)
        .execute(&parse(r#"RETURN CREATE_VIEW("people", {collection: "users"})"#).unwrap())
        .unwrap();
    let q = "FOR p IN people COLLECT k = p.k WITH COUNT INTO n RETURN {k, n}";
    let on = run(&engine, q, true, &binds).unwrap();
    assert_eq!(on, run(&engine, q, false, &binds).unwrap());
    assert_eq!(on.len(), 4, "{on:?}");
    assert!(!folded(&engine, q, &binds));
}

/// A sharded collection is read from its shards (scatter-gather), never
/// from the local collection, which holds none of its documents.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sharded_collections_are_read_from_their_shards() {
    use solidb::sharding::coordinator::{CollectionShardConfig, ShardCoordinator};
    use std::sync::Arc;
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    engine.create_database("d".to_string()).unwrap();
    let db = engine.get_database("d").unwrap();
    db.create_collection("s".to_string(), None).unwrap();
    let config = CollectionShardConfig {
        num_shards: 3,
        shard_key: "_key".to_string(),
        replication_factor: 1,
    };
    db.get_collection("s")
        .unwrap()
        .set_shard_config(&config)
        .unwrap();
    let coord = Arc::new(ShardCoordinator::new(engine.clone(), None, None));
    coord.init_collection("d", "s", &config).unwrap();
    coord.create_shards("d", "s").await.unwrap();

    let run = |q: &'static str, stream: bool| {
        let (engine, coord) = (engine.clone(), coord.clone());
        tokio::task::spawn_blocking(move || {
            QueryExecutor::with_database(&engine, "d".to_string())
                .with_shard_coordinator(coord)
                .with_streaming_collect(stream)
                .execute(&parse(q).unwrap())
                .map_err(|e| e.to_string())
        })
    };
    run(
        r#"FOR i IN 1..12 INSERT {_key: CONCAT("k", i), n: i, tag: i % 2 == 0 ? "even" : "odd"} INTO s"#,
        true,
    )
    .await
    .unwrap()
    .unwrap();
    let q = "FOR d IN s COLLECT tag = d.tag AGGREGATE total = SUM(d.n), n = COUNT() RETURN {tag, total, n}";
    let on = run(q, true).await.unwrap().unwrap();
    let off = run(q, false).await.unwrap().unwrap();
    assert_eq!(on, off);
    assert_eq!(
        on,
        vec![
            json!({"tag": "even", "total": 42, "n": 6}),
            json!({"tag": "odd", "total": 36, "n": 6}),
        ]
    );
}

/// The memory regression: with the row ceiling at 10,000 over 100,000
/// documents, the streaming COLLECT answers, because it never holds more
/// than its groups; the materialising path reads the collection into rows
/// first and is stopped by the ceiling. What a COLLECT does keep — `INTO`
/// members, one row per group — is still bounded by it.
#[test]
fn streaming_collect_holds_groups_not_documents() {
    const DOCS: usize = 100_000;
    const CEILING: usize = 10_000;
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    let orders = db.get_collection("orders").unwrap();
    let statuses = ["new", "paid", "shipped", "done", "void"];
    for start in (0..DOCS).step_by(10_000) {
        let batch = (start..start + 10_000)
            .map(|i| json!({"status": statuses[i % 5], "amount": (i % 100) as f64 + 0.5, "n": i}))
            .collect();
        orders.insert_batch(batch).unwrap();
    }
    let run = |q: &str, stream: bool| {
        QueryExecutor::with_database(&engine, DB.to_string())
            .with_max_intermediate_rows(CEILING)
            .with_streaming_collect(stream)
            .execute(&parse(q).unwrap())
            .map_err(|e| e.to_string())
    };
    let over_ceiling = |r: Result<Vec<Value>, String>, q: &str| {
        let err = r.expect_err(q);
        assert!(err.contains("intermediate row limit"), "{q}: {err}");
    };

    let q = "FOR o IN orders COLLECT status = o.status \
             AGGREGATE n = COUNT(), total = SUM(o.amount) RETURN {status, n, total}";
    let groups = run(q, true).unwrap();
    assert_eq!(groups.len(), 5);
    for g in &groups {
        assert_eq!(g["n"], json!(DOCS / 5), "{g}");
    }
    let total: f64 = groups.iter().map(|g| g["total"].as_f64().unwrap()).sum();
    assert_eq!(
        total,
        (0..DOCS).map(|i| (i % 100) as f64 + 0.5).sum::<f64>()
    );
    over_ceiling(run(q, false), q);

    let q = "FOR o IN orders FILTER o.amount > 50 COLLECT WITH COUNT INTO n RETURN n";
    assert_eq!(run(q, true).unwrap(), vec![json!(DOCS / 2)]);
    over_ceiling(run(q, false), q);

    let q = "FOR o IN orders COLLECT WITH COUNT INTO n RETURN n";
    assert_eq!(run(q, true).unwrap(), vec![json!(DOCS)]);
    over_ceiling(run(q, false), q);

    // What the COLLECT keeps is still bounded.
    let q = "FOR o IN orders COLLECT s = o.status INTO keys = o._key RETURN LENGTH(keys)";
    over_ceiling(run(q, true), q);
    let q = "FOR o IN orders COLLECT n = o.n RETURN n";
    over_ceiling(run(q, true), q);
}

// ---------------------------------------------------------------------------
// Typed reads and parallel scans (#776)
// ---------------------------------------------------------------------------

/// How a streamed COLLECT is run: typed reads or decoded documents, and
/// with how many threads (`(threads, min_docs_per_thread)`).
#[derive(Clone, Copy, Debug)]
struct Mode {
    typed: bool,
    threads: usize,
    min_docs: usize,
}

const GENERIC: Mode = Mode {
    typed: false,
    threads: 1,
    min_docs: 1,
};
const TYPED: Mode = Mode {
    typed: true,
    threads: 1,
    min_docs: 1,
};
/// Four threads for any collection of eight documents or more.
const PARALLEL: Mode = Mode {
    typed: true,
    threads: 4,
    min_docs: 2,
};

fn executor_in<'a>(engine: &'a StorageEngine, mode: Mode, binds: &BindVars) -> QueryExecutor<'a> {
    executor(engine, true, binds)
        .with_typed_collect(mode.typed)
        .with_scan_threads(mode.threads, mode.min_docs)
}

fn run_in(
    engine: &StorageEngine,
    q: &str,
    mode: Mode,
    binds: &BindVars,
) -> Result<Vec<Value>, String> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    executor_in(engine, mode, binds)
        .execute(&query)
        .map_err(|e| e.to_string())
}

/// The note EXPLAIN gives about the fold, run (`analyze`) and planned.
fn notes(engine: &StorageEngine, q: &str, mode: Mode, binds: &BindVars) -> (String, String) {
    let query = parse(q).unwrap();
    let ex = executor_in(engine, mode, binds);
    let find = |warnings: Vec<String>| {
        warnings
            .into_iter()
            .find(|w| w.contains(FOLDED))
            .unwrap_or_else(|| panic!("not folded: {q}"))
    };
    (
        find(ex.explain(&query).unwrap().warnings),
        find(ex.explain_plan(&query).unwrap().warnings),
    )
}

/// The answer of every way to run `q` — materialised, streamed with decoded
/// documents, streamed with typed reads on one thread and on four — after
/// checking they are all the same, errors included.
fn same_everywhere(
    engine: &StorageEngine,
    q: &str,
    binds: &BindVars,
) -> Result<Vec<Value>, String> {
    let materialised = run(engine, q, false, binds);
    for mode in [GENERIC, TYPED, PARALLEL] {
        assert_eq!(
            run_in(engine, q, mode, binds),
            materialised,
            "{q} in {mode:?}"
        );
    }
    materialised
}

#[test]
fn typed_reads_agree_with_decoded_documents() {
    let (engine, _tmp) = engine_with(mixed_docs());
    let mut binds = BindVars::new();
    binds.insert("min".to_string(), json!(2));
    binds.insert("tag".to_string(), json!("blue"));
    binds.insert("set".to_string(), json!([1, "a", null, [1, 2]]));
    let mergeable = "n = COUNT(), cn = COUNT(d.v), len = LENGTH(d.v), s = SUM(d.v), \
         a = AVG(d.v), mn = MIN(d.v), mx = MAX(d.v), cd = COUNT_DISTINCT(d.v), \
         ba = BIT_AND(d.i), bo = BIT_OR(d.i), bx = BIT_XOR(d.i), mi = MIN(d.i), mxs = MAX(d.t)";
    let ordered = "u = UNIQUE(d.v), su = SORTED_UNIQUE(d.v), l = COLLECT_LIST(d.v), \
         var = VARIANCE(d.v), sd = STDDEV_SAMPLE(d.v), med = MEDIAN(d.v)";
    let typed = [
        "FOR d IN c COLLECT k = d.k WITH COUNT INTO n RETURN {k, n}".to_string(),
        "FOR d IN c COLLECT k = d.k, t = d.t RETURN [k, t]".to_string(),
        "FOR d IN c COLLECT WITH COUNT INTO n RETURN n".to_string(),
        format!("FOR d IN c COLLECT k = d.k AGGREGATE {mergeable} RETURN MERGE({{k}}, {{n, cn, len, s, a, mn, mx, cd, ba, bo, bx, mi, mxs}})"),
        format!("FOR d IN c COLLECT AGGREGATE {mergeable} RETURN {{n, cn, len, s, a, mn, mx, cd, ba, bo, bx, mi, mxs}}"),
        format!("FOR d IN c COLLECT t = d.t AGGREGATE {ordered} RETURN {{t, u, su, l, var, sd, med}}"),
        "FOR d IN c COLLECT k = d.nested.deep AGGREGATE s = SUM(d.nested.deep) RETURN {k, s}".to_string(),
        "FOR d IN c COLLECT k = d.v.a, m = d.v.missing WITH COUNT INTO n RETURN {k, m, n}".to_string(),
        "FOR d IN c COLLECT k = d.nested.list WITH COUNT INTO n RETURN {k, n}".to_string(),
        "FOR d IN c FILTER d.t == 'red' COLLECT k = d.k AGGREGATE s = SUM(d.v) RETURN {k, s}".to_string(),
        "FOR d IN c FILTER d.v != null FILTER d.v > 2 COLLECT WITH COUNT INTO n RETURN n".to_string(),
        "FOR d IN c FILTER 2 < d.v COLLECT k = d.k WITH COUNT INTO n RETURN {k, n}".to_string(),
        "FOR d IN c FILTER d.v <= 'x' AND d.v >= -3 COLLECT WITH COUNT INTO n RETURN n".to_string(),
        "FOR d IN c FILTER d.v >= @min AND d.t == @tag COLLECT k = d.k AGGREGATE s = SUM(d.v) RETURN {k, s}".to_string(),
        "FOR d IN c FILTER d.k IN [1, 'a', null] COLLECT k = d.k WITH COUNT INTO n RETURN {k, n}".to_string(),
        "FOR d IN c FILTER d.k NOT IN @set COLLECT k = d.k WITH COUNT INTO n RETURN {k, n}".to_string(),
        "FOR d IN c FILTER d.t IN {red: 1} COLLECT WITH COUNT INTO n RETURN n".to_string(),
        "FOR d IN c FILTER NOT (d.v < 3) OR d.t == 'blue' COLLECT t = d.t WITH COUNT INTO n RETURN {t, n}".to_string(),
        "FOR d IN c FILTER d.v COLLECT k = d.k WITH COUNT INTO n RETURN {k, n}".to_string(),
        "FOR d IN c FILTER d.nested.deep FILTER d.missing == null COLLECT WITH COUNT INTO n RETURN n".to_string(),
        "FOR d IN c FILTER d.k == [1.0, 2] OR d.k == {x: 1} COLLECT k = d.k WITH COUNT INTO n RETURN {k, n}".to_string(),
        "FOR d IN c FILTER d.v > 1000000000 COLLECT AGGREGATE s = SUM(d.v) RETURN s".to_string(),
    ];
    for q in &typed {
        same_everywhere(&engine, q, &binds).unwrap_or_else(|e| panic!("{q}: {e}"));
        let (run_note, plan_note) = notes(&engine, q, TYPED, &binds);
        assert!(
            run_note.contains("from the stored bytes"),
            "{q}: {run_note}"
        );
        assert_eq!(run_note, plan_note, "{q}");
    }

    // Not compiled: decoded documents, the same answer.
    for q in [
        "FOR d IN c FILTER IS_NUMBER(d.v) COLLECT k = d.k AGGREGATE s = SUM(d.v) RETURN {k, s}",
        "FOR d IN c COLLECT k = d._key RETURN k",
        "FOR d IN c COLLECT x = d[\"a.b\"] WITH COUNT INTO n RETURN {x, n}",
        "FOR d IN c LET y = d.v COLLECT k = y WITH COUNT INTO n RETURN {k, n}",
        "FOR d IN c COLLECT k = d.k INTO g = d.t RETURN {k, g}",
        "FOR d IN c FILTER d.v == d.i COLLECT WITH COUNT INTO n RETURN n",
    ] {
        same_everywhere(&engine, q, &binds).unwrap_or_else(|e| panic!("{q}: {e}"));
        let (run_note, _) = notes(&engine, q, TYPED, &binds);
        assert!(
            !run_note.contains("from the stored bytes"),
            "{q}: {run_note}"
        );
    }
}

const STATUSES3: [&str; 3] = ["new", "paid", "shipped"];

/// `orders`-like documents where the order of addition shows in a float sum,
/// where `1` and `1.0` (and `2` and `2.0` as extremes) are first met in the
/// first key range, and where a group exists only in the last.
fn tricky_docs(count: usize) -> Vec<Value> {
    let amounts = [1e16, 0.1, -1e16, 3.3, 1e-7, 2.5, -0.3];
    (0..count)
        .map(|i| {
            // The first rows of group 1 carry 1.0, later ones 1; in group
            // 0 the least `x` is 2.0 in the first rows, an equal 2 later.
            let g = match i % 7 {
                1 if i < count / 8 => json!(1.0),
                g => json!(g),
            };
            let x = if i % 7 == 0 && i < count / 8 {
                json!(2.0)
            } else if i % 11 == 0 {
                json!(2)
            } else {
                json!(i % 50 + 3)
            };
            let mut doc = json!({
                "_key": format!("k{i:05}"),
                "g": g,
                "x": x,
                "amount": amounts[i % amounts.len()] * (1.0 + (i % 13) as f64),
                "status": STATUSES3[i % 3],
                "flags": i as i64 & 0xff,
                "items": [{"sku": format!("S{}", i % 17), "qty": i % 4}],
            });
            if i >= count - count / 10 {
                doc["status"] = json!("late");
            }
            doc
        })
        .collect()
}

#[test]
fn parallel_scans_give_the_sequential_answer() {
    let (engine, _tmp) = engine_with(tricky_docs(3_000));
    let binds = BindVars::new();
    let coll = engine
        .get_database(DB)
        .unwrap()
        .get_collection("c")
        .unwrap();
    for q in [
        "FOR d IN c COLLECT g = d.g AGGREGATE n = COUNT(), s = SUM(d.amount), a = AVG(d.amount), \
         mn = MIN(d.x), mx = MAX(d.x), cd = COUNT_DISTINCT(d.status), bo = BIT_OR(d.flags), \
         bx = BIT_XOR(d.flags) RETURN {g, n, s, a, mn, mx, cd, bo, bx}",
        "FOR d IN c COLLECT status = d.status AGGREGATE n = COUNT(), total = SUM(d.amount) \
         RETURN {status, n, total}",
        "FOR d IN c FILTER d.amount > 0 COLLECT status = d.status AGGREGATE n = COUNT(), \
         total = SUM(d.amount) RETURN {status, n, total}",
        "FOR d IN c COLLECT WITH COUNT INTO n RETURN n",
        "FOR d IN c COLLECT AGGREGATE s = SUM(d.amount), mn = MIN(d.g) RETURN {s, mn}",
        "FOR d IN c FILTER d.status == 'late' COLLECT g = d.g WITH COUNT INTO n RETURN {g, n}",
        "FOR d IN c FILTER d.status == 'nobody' COLLECT g = d.g WITH COUNT INTO n RETURN {g, n}",
        "FOR d IN c FILTER d.status == 'nobody' COLLECT AGGREGATE s = SUM(d.amount) RETURN s",
    ] {
        let groups = same_everywhere(&engine, q, &binds).unwrap_or_else(|e| panic!("{q}: {e}"));
        assert!(!groups.is_empty() || q.contains("nobody"), "{q}");
    }

    // The group key and the extreme are the first row's: 1.0 and 2.0.
    let first = run_in(
        &engine,
        "FOR d IN c COLLECT g = d.g AGGREGATE mn = MIN(d.x) RETURN {g, mn}",
        PARALLEL,
        &binds,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&first[1]["g"]).unwrap(),
        "1.0",
        "{first:?}"
    );
    assert_eq!(
        serde_json::to_string(&first[0]["mn"]).unwrap(),
        "2.0",
        "{first:?}"
    );

    // The run was split where the engine can read ranges at one snapshot.
    let q = "FOR d IN c COLLECT status = d.status AGGREGATE n = COUNT() RETURN {status, n}";
    let parallel = |note: &str| note.contains("key ranges read in parallel");
    let (mut run_note, plan_note) = notes(&engine, q, PARALLEL, &binds);
    if coll.supports_parallel_scan() {
        // Other tests may hold the process's scan threads for a moment.
        for _ in 0..20 {
            if parallel(&run_note) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
            run_note = notes(&engine, q, PARALLEL, &binds).0;
        }
        assert!(parallel(&run_note), "{run_note}");
        assert_eq!(run_note, plan_note);
    } else {
        assert!(!parallel(&run_note) && !parallel(&plan_note), "{run_note}");
    }
    // Aggregates whose partial results do not merge exactly stay on one
    // thread.
    let (note, plan) = notes(
        &engine,
        "FOR d IN c COLLECT s = d.status AGGREGATE l = COLLECT_LIST(d.g) RETURN {s, l}",
        PARALLEL,
        &binds,
    );
    assert!(!parallel(&note) && !parallel(&plan), "{note}");
    // One thread asked for, one used.
    let (note, _) = notes(&engine, q, TYPED, &binds);
    assert!(!parallel(&note), "{note}");
}

/// Sums whose value depends on the order of addition give one answer: the
/// correctly rounded sum, whatever the split.
#[test]
fn float_sums_do_not_depend_on_the_split() {
    let values = [1e16, 1.0, -1e16, 0.1, 0.2, 0.3, 1e-9, 3.3, -2.2];
    let docs: Vec<Value> = (0..900)
        .map(|i| json!({"_key": format!("f{i:04}"), "v": values[i % values.len()], "g": i % 2}))
        .collect();
    let (engine, _tmp) = engine_with(docs);
    let binds = BindVars::new();
    let q = "FOR d IN c COLLECT g = d.g AGGREGATE s = SUM(d.v), a = AVG(d.v) RETURN {g, s, a}";
    let expected = same_everywhere(&engine, q, &binds).unwrap();
    for (threads, min_docs) in [(2, 2), (3, 2), (7, 2), (16, 2)] {
        let mode = Mode {
            typed: true,
            threads,
            min_docs,
        };
        assert_eq!(
            run_in(&engine, q, mode, &binds).unwrap(),
            expected,
            "{threads}"
        );
    }
    // The array builtins add the same way. Integers, and no number at all,
    // add to an integer; a float zero keeps its sign.
    let ret = |q: &str| serde_json::to_string(&run(&engine, q, true, &binds).unwrap()).unwrap();
    assert_eq!(
        ret("RETURN SUM([0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1])"),
        "[1.0]"
    );
    assert_eq!(ret("RETURN AVG([0.1, 0.2, 0.3])"), "[0.19999999999999998]");
    assert_eq!(
        ret("RETURN [SUM([]), SUM([-0.0]), SUM([1, -1]), SUM(['x'])]"),
        "[[0,-0.0,0,0]]"
    );

    // 100 repetitions of the nine values per group: exactly 100 times their
    // sum, which a running f64 sum misses.
    let per_cycle: f64 = 1.0 + 0.1 + 0.2 + 0.3 + 1e-9 + 3.3 - 2.2;
    let total: f64 = expected.iter().map(|g| g["s"].as_f64().unwrap()).sum();
    assert!((total - 100.0 * per_cycle).abs() < 1e-9, "{expected:?}");
}

/// The key ranges a scan is split into cover every document once, in
/// order, whatever their number and however the keys are spread.
#[test]
fn key_ranges_cover_every_document_once() {
    let mut key_sets: Vec<Vec<String>> = vec![
        vec![],
        vec!["only".to_string()],
        vec!["a".to_string(), "b".to_string()],
        (0..1000).map(|i| format!("{i:06}")).collect(),
    ];
    // Skewed: many keys under one prefix, a few far away.
    let mut skewed: Vec<String> = (0..500).map(|i| format!("aaa{i:04}")).collect();
    skewed.extend((0..5).map(|i| format!("zz{i}")));
    key_sets.push(skewed);
    // Keys that are prefixes of one another.
    key_sets.push(vec![
        "x".into(),
        "xx".into(),
        "xxx".into(),
        "xxxx".into(),
        "y".into(),
    ]);
    for keys in key_sets {
        let docs: Vec<Value> = keys.iter().map(|k| json!({"_key": k, "v": 1})).collect();
        let (engine, _tmp) = engine_with(docs);
        let coll = engine
            .get_database(DB)
            .unwrap()
            .get_collection("c")
            .unwrap();
        for n in [1usize, 2, 3, 4, 8, 16, 64] {
            let ranges = coll.doc_partitions(n);
            assert!(!ranges.is_empty() && ranges.len() <= n, "{n}: {ranges:?}");
            assert_eq!(ranges[0].0, b"doc:".to_vec());
            assert_eq!(ranges.last().unwrap().1, b"doc;".to_vec());
            for pair in ranges.windows(2) {
                assert_eq!(pair[0].1, pair[1].0, "ranges must touch");
                assert!(pair[0].0 < pair[0].1, "empty range {pair:?}");
            }
            let snapshot = coll.scan_snapshot();
            let mut seen = Vec::new();
            for (lo, hi) in &ranges {
                coll.for_each_doc_bytes_in(lo, hi, snapshot.as_ref(), |bytes| {
                    let doc = solidb::storage::serializer::deserialize_doc_as_value(bytes).unwrap();
                    seen.push(doc["_key"].as_str().unwrap().to_string());
                    Ok(true)
                })
                .unwrap();
            }
            let mut expected = keys.clone();
            expected.sort();
            assert_eq!(seen, expected, "{n} ranges: {ranges:?}");
        }
    }
}

/// Key ranges cut where the engine says (solistore's split points, from its
/// memtables and then its table indexes), or from size estimates (RocksDB),
/// read at one snapshot: every document once, in order, before and after
/// the data is flushed, and a write made after the snapshot is not seen.
#[test]
fn engine_cut_ranges_read_one_snapshot() {
    let docs: Vec<Value> = (0..20_000)
        .map(|i| json!({"_key": format!("e{i:06}"), "s": STATUSES3[i % 3], "v": i}))
        .collect();
    let (engine, _tmp) = engine_with(docs);
    let coll = engine
        .get_database(DB)
        .unwrap()
        .get_collection("c")
        .unwrap();
    assert!(coll.supports_parallel_scan());
    let read_all = |ranges: &[(Vec<u8>, Vec<u8>)],
                    snapshot: Option<&solidb::storage::kv::KvSnapshot<'_>>| {
        let mut keys = Vec::new();
        for (lo, hi) in ranges {
            coll.for_each_doc_bytes_in(lo, hi, snapshot, |bytes| {
                let doc = solidb::storage::serializer::deserialize_doc_as_value(bytes).unwrap();
                keys.push(doc["_key"].as_str().unwrap().to_string());
                Ok(true)
            })
            .unwrap();
        }
        keys
    };
    let expected: Vec<String> = (0..20_000).map(|i| format!("e{i:06}")).collect();
    for flushed in [false, true] {
        if flushed {
            engine.flush().unwrap();
        }
        let snapshot = coll.scan_snapshot().unwrap();
        let ranges = coll.doc_partitions_at(4, Some(&snapshot));
        assert!(ranges.len() >= 2, "flushed {flushed}: {ranges:?}");
        // Written after the snapshot: not in what it reads.
        coll.insert(json!({"_key": format!("late{flushed}"), "s": "z"}))
            .unwrap();
        assert_eq!(
            read_all(&ranges, Some(&snapshot)),
            expected,
            "flushed {flushed}"
        );
        drop(snapshot);
        coll.delete(&format!("late{flushed}")).unwrap();
    }
    let binds = BindVars::new();
    let q = "FOR d IN c COLLECT s = d.s AGGREGATE n = COUNT(), t = SUM(d.v) RETURN {s, n, t}";
    let groups = same_everywhere(&engine, q, &binds).unwrap();
    assert_eq!(groups.len(), 3, "{groups:?}");
    let parallel = |note: &str| note.contains("key ranges read in parallel");
    let (mut run_note, plan_note) = notes(&engine, q, PARALLEL, &binds);
    // Other tests may hold the process's scan threads for a moment.
    for _ in 0..20 {
        if parallel(&run_note) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        run_note = notes(&engine, q, PARALLEL, &binds).0;
    }
    assert!(parallel(&run_note), "{run_note}");
    assert_eq!(run_note, plan_note);
}

/// An empty collection and a one-document one, asked for many threads.
#[test]
fn tiny_collections_under_parallel_settings() {
    for docs in [vec![], vec![json!({"_key": "one", "s": "a", "v": 1.5})]] {
        let (engine, _tmp) = engine_with(docs);
        let binds = BindVars::new();
        let mode = Mode {
            typed: true,
            threads: 8,
            min_docs: 1,
        };
        for q in [
            "FOR d IN c COLLECT WITH COUNT INTO n RETURN n",
            "FOR d IN c COLLECT s = d.s AGGREGATE n = COUNT(), t = SUM(d.v) RETURN {s, n, t}",
            "FOR d IN c COLLECT AGGREGATE t = SUM(d.v), m = MAX(d.v) RETURN {t, m}",
        ] {
            let expected = run(&engine, q, false, &binds);
            assert_eq!(run_in(&engine, q, mode, &binds), expected, "{q}");
        }
    }
}

/// The row ceiling holds across the key ranges: folds that would hold more
/// than it between them fall back to the one-thread fold, which answers or
/// fails exactly as before.
#[test]
fn the_row_ceiling_holds_across_key_ranges() {
    let docs: Vec<Value> = (0..6_000)
        .map(|i| json!({"_key": format!("r{i:05}"), "n": i, "s": if i % 2 == 0 { "a" } else { "b" }}))
        .collect();
    let (engine, _tmp) = engine_with(docs);
    let run_capped = |q: &str, mode: Mode| {
        executor_in(&engine, mode, &BindVars::new())
            .with_max_intermediate_rows(2_000)
            .execute(&parse(q).unwrap())
            .map_err(|e| e.to_string())
    };
    let many = "FOR d IN c COLLECT n = d.n RETURN n";
    for mode in [TYPED, PARALLEL] {
        let err = run_capped(many, mode).expect_err(many);
        assert!(err.contains("intermediate row limit"), "{err}");
    }
    let few = "FOR d IN c COLLECT s = d.s WITH COUNT INTO k RETURN {s, k}";
    assert_eq!(run_capped(few, PARALLEL), run_capped(few, TYPED));
    assert_eq!(
        run_capped(few, PARALLEL).unwrap(),
        vec![json!({"s": "a", "k": 3000}), json!({"s": "b", "k": 3000})]
    );
}
