//! The paths `benchmarks/document_workload.py` measures, each held to the
//! answer of the path it replaces, on the same data:
//!
//! - a subquery over an array in memory (`execution::array_subquery`)
//!   against the pipeline (`with_array_subqueries(false)`);
//! - a LET or a RETURN evaluated for many rows on several threads
//!   (`execution::par_rows`) against one thread (`with_scan_threads(1, …)`),
//!   `DOCUMENT()` with missing keys among them;
//! - a COLLECT behind a range on a single-field index (`try_range_collect`)
//!   against the materialising path (`with_streaming_collect(false)`), and
//!   against a collection that has no index;
//! - a filtered REMOVE (`execution::scan_remove`) against the row-by-row
//!   scan (`with_typed_collect(false)`), its effect on the indexes included.

use serde_json::{json, Value};
use solidb::storage::{IndexType, StorageEngine};
use solidb::{parse, BindVars, QueryExecutor};
use tempfile::TempDir;

const DB: &str = "db";
const METRICS: usize = 40;
const DAYS: usize = 60;

fn value(m: usize, p: usize, i: usize) -> usize {
    (m * 31 + p * 17 + i * 7) % 100
}

/// One document per metric and day, as the benchmark's, plus a few whose
/// `d` is not an integer: missing, null, a string, a float.
fn metric_docs() -> Vec<Value> {
    let mut docs = Vec::new();
    for m in 1..=METRICS {
        for i in 0..DAYS {
            let v: serde_json::Map<String, Value> = (1..=5)
                .map(|p| (p.to_string(), json!(value(m, p, i))))
                .collect();
            let s: usize = (1..=5).map(|p| value(m, p, i)).sum();
            docs.push(json!({
                "_key": format!("m{m}_{i:03}"),
                "m": m, "d": 1000 + i, "v": v, "s": s, "n": 5,
                "tag": format!("t{}", (m + i) % 7),
            }));
        }
    }
    docs.push(json!({"_key": "odd_missing", "m": 1, "s": 1, "n": 1, "tag": "t0"}));
    docs.push(json!({"_key": "odd_null", "m": 1, "d": null, "s": 2, "n": 1, "tag": "t0"}));
    docs.push(json!({"_key": "odd_text", "m": 2, "d": "1010", "s": 3, "n": 1, "tag": "t1"}));
    docs.push(json!({"_key": "odd_float", "m": 2, "d": 1055.5, "s": 4, "n": 1, "tag": "t1"}));
    docs.push(json!({"_key": "odd_whole", "m": 3, "d": 1058.0, "s": 5.5, "n": 1, "tag": "t2"}));
    docs
}

/// `plain` holds the documents with no index; `mv` holds them with a
/// persistent index on `d`, one on `[m, d]` and a hash index on `tag`.
fn engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    for name in ["mv", "plain", "out"] {
        db.create_collection(name.to_string(), None).unwrap();
    }
    db.get_collection("plain")
        .unwrap()
        .insert_batch(metric_docs())
        .unwrap();
    let mv = db.get_collection("mv").unwrap();
    mv.insert_batch(metric_docs()).unwrap();
    mv.create_index(
        "idx_d".into(),
        vec!["d".into()],
        IndexType::Persistent,
        false,
    )
    .unwrap();
    mv.create_index(
        "idx_m_d".into(),
        vec!["m".into(), "d".into()],
        IndexType::Persistent,
        false,
    )
    .unwrap();
    mv.create_index("idx_tag".into(), vec!["tag".into()], IndexType::Hash, false)
        .unwrap();
    (engine, tmp)
}

fn binds(pairs: &[(&str, Value)]) -> BindVars {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

/// How a query is run: the paths under test on or off.
#[derive(Clone, Copy)]
struct Mode {
    arrays: bool,
    threads: usize,
    stream: bool,
    typed: bool,
}

const FAST: Mode = Mode {
    arrays: true,
    threads: 4,
    stream: true,
    typed: true,
};

fn executor<'a>(engine: &'a StorageEngine, mode: Mode, binds: &BindVars) -> QueryExecutor<'a> {
    QueryExecutor::with_database_and_bind_vars(engine, DB.to_string(), binds.clone())
        .with_array_subqueries(mode.arrays)
        .with_scan_threads(mode.threads, 1)
        .with_streaming_collect(mode.stream)
        .with_typed_collect(mode.typed)
}

fn run(
    engine: &StorageEngine,
    q: &str,
    mode: Mode,
    binds: &BindVars,
) -> Result<Vec<Value>, String> {
    let query = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e}"));
    executor(engine, mode, binds)
        .execute(&query)
        .map_err(|e| e.to_string())
}

fn warnings(engine: &StorageEngine, q: &str, binds: &BindVars) -> Vec<String> {
    let query = parse(q).unwrap();
    executor(engine, FAST, binds)
        .explain(&query)
        .unwrap()
        .warnings
}

// ---------- subqueries over an array in memory ----------

/// The answer of `q`, the same (errors included) with the array walked
/// where it is and through the pipeline.
fn same_subquery(engine: &StorageEngine, q: &str, binds: &BindVars) -> Result<Vec<Value>, String> {
    let direct = run(engine, q, FAST, binds);
    let pipeline = run(
        engine,
        q,
        Mode {
            arrays: false,
            ..FAST
        },
        binds,
    );
    assert_eq!(direct, pipeline, "{q}");
    direct
}

#[test]
fn a_subquery_over_an_array_of_an_outer_let() {
    let (engine, _tmp) = engine();
    let none = BindVars::new();
    let rows = same_subquery(
        &engine,
        "LET ds = [{v: {'1': 3}}, null, {v: {'1': 5}}, {v: {}}, {w: 1}]
         RETURN AVG((FOR x IN ds FILTER x != null AND x.v['1'] != null RETURN x.v['1']))",
        &none,
    )
    .unwrap();
    assert_eq!(rows, vec![json!(4.0)]);

    // The array iterated by one subquery after another, for several outer
    // rows, with the loop variable of one named as a variable of the other.
    let rows = same_subquery(
        &engine,
        "FOR m IN [1, 2, 3]
           LET ds = (FOR i IN 0..2 RETURN {m: m, i: i, v: {a: m * 10 + i, b: null}})
           RETURN {m, sums: (FOR p IN ['a', 'b', 'c']
                     RETURN SUM((FOR x IN ds FILTER x.v[p] != null RETURN x.v[p]))),
                   again: (FOR m IN ds RETURN m.i)}",
        &none,
    )
    .unwrap();
    assert_eq!(rows[0]["sums"], json!([33, 0, 0]));
    assert_eq!(rows[2]["again"], json!([0, 1, 2]));
}

#[test]
fn every_source_of_an_array_subquery() {
    let (engine, _tmp) = engine();
    let b = binds(&[
        ("list", json!([{"k": 1}, {"k": 2}, "x", null])),
        ("one", json!({"k": 9})),
    ]);
    for q in [
        "RETURN (FOR x IN [3, 1, 2] FILTER x > 1 RETURN x * 2)",
        "RETURN (FOR x IN 1..5 FILTER x % 2 == 1 RETURN x)",
        "RETURN (FOR x IN @list RETURN x.k)",
        "RETURN (FOR x IN @list FILTER x.k >= 1 FILTER x.k < 2 RETURN x)",
        // Not an array: one row.
        "RETURN (FOR x IN @one RETURN x.k)",
        "LET v = 7 RETURN (FOR x IN v RETURN x + 1)",
        "LET v = null RETURN (FOR x IN v RETURN x)",
        "LET doc = {list: [{a: 1}, {a: 2}]} RETURN (FOR x IN doc.list RETURN x.a)",
        "LET doc = {list: [{a: 1}, {a: 2}]} RETURN (FOR x IN doc.missing RETURN x)",
        "LET a = [[1, 2], [3]] RETURN (FOR x IN a RETURN (FOR y IN x RETURN y * 10))",
        "LET a = [1, 2, 3] RETURN (FOR x IN a FILTER x > 5 RETURN x)",
        "LET a = [] RETURN LENGTH((FOR x IN a RETURN x))",
        // A FILTER that fails to evaluate drops the row.
        "LET a = [1, 'b', 3] RETURN (FOR x IN a FILTER x + 1 > 2 RETURN x)",
        // DISTINCT, LIMIT and SORT stay with the pipeline: same answers.
        "LET a = [2, 1, 2] RETURN (FOR x IN a RETURN DISTINCT x)",
        "LET a = [2, 1, 3] RETURN (FOR x IN a SORT x LIMIT 2 RETURN x)",
        // A name that is a collection too.
        "LET plain = [1, 2] RETURN (FOR x IN plain RETURN x)",
        "LET out = [1, 2] RETURN (FOR x IN out FILTER x == 2 RETURN x)",
    ] {
        same_subquery(&engine, q, &b).unwrap_or_else(|e| panic!("{q}: {e}"));
    }
}

#[test]
fn the_errors_of_an_array_subquery_are_the_pipelines() {
    let (engine, _tmp) = engine();
    let none = BindVars::new();
    for q in [
        // A mistake of the query, met by a FILTER: the query fails with it.
        "LET a = [1, 2] RETURN (FOR x IN a FILTER NO_SUCH(x) RETURN x)",
        "LET a = [1, 2] RETURN (FOR x IN a FILTER x > @missing RETURN x)",
        "LET a = [1, 2] RETURN (FOR x IN a FILTER NO_SUCH(x) FILTER x > @missing RETURN x)",
        "LET a = [1, 2] RETURN (FOR x IN a FILTER x > @missing FILTER NO_SUCH(x) RETURN x)",
        // An error of the RETURN, and of the source.
        "LET a = [1, 2] RETURN (FOR x IN a RETURN NO_SUCH(x))",
        "RETURN (FOR x IN NO_SUCH(1) RETURN x)",
        "RETURN (FOR x IN nowhere.field RETURN x)",
    ] {
        let failed = same_subquery(&engine, q, &none);
        assert!(failed.is_err(), "{q}: {failed:?}");
    }
}

// ---------- rows evaluated on several threads ----------

fn one_thread() -> Mode {
    Mode { threads: 1, ..FAST }
}

#[test]
fn document_with_missing_keys_keeps_what_it_finds_in_order() {
    let (engine, _tmp) = engine();
    let rows = run(
        &engine,
        "RETURN DOCUMENT('mv', ['m2_001', 'nope', 'm1_000', 7, 'm40_059'])[*]._key",
        FAST,
        &BindVars::new(),
    )
    .unwrap();
    assert_eq!(rows, vec![json!(["m2_001", "m1_000", "m40_059"])]);
}

#[test]
fn many_rows_on_several_threads_are_those_of_one() {
    let (engine, _tmp) = engine();
    // 300 metrics asked, 40 stored; the window reaches before the first
    // day for some: DOCUMENT() returns fewer documents, or none.
    let b = binds(&[
        ("metrics", json!((1..=300).collect::<Vec<_>>())),
        ("days", json!(["001", "000", "-01"])),
        ("perimeters", json!(["1", "2", "3", "4", "5", "6"])),
    ]);
    let q = "FOR m IN @metrics
               LET ds = DOCUMENT('mv', (FOR d IN @days RETURN CONCAT('m', m, '_', d)))
               RETURN {m, n: LENGTH(ds), avg: ZIP(@perimeters, (FOR p IN @perimeters
                 RETURN AVG((FOR x IN ds FILTER x != null AND x.v[p] != null RETURN x.v[p]))))}";
    let spread = run(&engine, q, FAST, &b).unwrap();
    let alone = run(&engine, q, one_thread(), &b).unwrap();
    let pipeline = run(
        &engine,
        q,
        Mode {
            arrays: false,
            threads: 1,
            ..FAST
        },
        &b,
    )
    .unwrap();
    assert_eq!(spread, alone);
    assert_eq!(spread, pipeline);
    assert_eq!(spread.len(), 300);
    // Metric 7, days 0 and 1, perimeter 1; no document, so no value, for 6.
    let expected = (value(7, 1, 0) + value(7, 1, 1)) as f64 / 2.0;
    assert_eq!(spread[6]["n"], json!(2));
    assert_eq!(spread[6]["avg"]["1"], json!(expected));
    assert_eq!(spread[6]["avg"]["6"], json!(null));
    assert_eq!(spread[299]["n"], json!(0));
}

#[test]
fn a_writing_query_computes_its_rows_on_several_threads() {
    let (engine, _tmp) = engine();
    let b = binds(&[("ids", json!((0..400).collect::<Vec<_>>()))]);
    let q = "FOR i IN @ids
               LET docs = DOCUMENT('mv', [CONCAT('m', i % 45 + 1, '_000'), CONCAT('m', i % 45 + 1, '_001')])
               LET total = SUM((FOR x IN docs RETURN x.s))
               INSERT {_key: CONCAT('k', i), total, n: LENGTH(docs)} INTO out OPTIONS {overwrite: true}";
    let read = "FOR x IN out SORT x._key RETURN {k: x._key, total: x.total, n: x.n}";
    run(&engine, q, FAST, &b).unwrap();
    let spread = run(&engine, read, FAST, &BindVars::new()).unwrap();
    run(
        &engine,
        "FOR x IN out REMOVE x IN out",
        FAST,
        &BindVars::new(),
    )
    .unwrap();
    run(&engine, q, one_thread(), &b).unwrap();
    let alone = run(&engine, read, FAST, &BindVars::new()).unwrap();
    assert_eq!(spread.len(), 400);
    assert_eq!(spread, alone);
}

#[test]
fn the_first_row_that_fails_is_reported_on_several_threads_too() {
    let (engine, _tmp) = engine();
    let b = binds(&[("ids", json!((0..600).collect::<Vec<_>>()))]);
    for q in [
        // Rows 350 and 500 fail, with different messages.
        "FOR i IN @ids
           LET v = (FOR x IN [i] RETURN i == 350 ? NO_SUCH_A(x) : (i == 500 ? NO_SUCH_B(x) : x))
           RETURN v",
        "FOR i IN @ids
           RETURN (FOR x IN [i] RETURN i == 500 ? NO_SUCH_A(x) : (i == 350 ? NO_SUCH_B(x) : x))",
        // A mistake met by a FILTER inside the subquery.
        "FOR i IN @ids LET v = (FOR x IN [i] FILTER i > 400 AND NO_SUCH(x) RETURN x) RETURN v",
    ] {
        let spread = run(&engine, q, FAST, &b);
        let alone = run(&engine, q, one_thread(), &b);
        assert!(spread.is_err(), "{q}");
        assert_eq!(spread, alone, "{q}");
    }
}

// ---------- COLLECT behind a range on a single-field index ----------

const COLLECTS: &[&str] = &[
    "FOR x IN mv FILTER x.d {op} @f COLLECT m = x.m AGGREGATE s = SUM(x.s), n = SUM(x.n)
       RETURN {m, a: s / n}",
    "FOR x IN mv FILTER x.d {op} @f COLLECT AGGREGATE lo = MIN(x.s), hi = MAX(x.d), c = COUNT(x.d),
       a = AVG(x.s) RETURN {lo, hi, c, a}",
    "FOR x IN mv FILTER x.d {op} @f FILTER x.m != 3 COLLECT t = x.tag WITH COUNT INTO c
       RETURN {t, c}",
    "FOR x IN mv FILTER x.d {op} @f AND x.s > 100 COLLECT d = x.d AGGREGATE m = MAX(x.m)
       RETURN {d, m}",
    // Not merged across threads: one thread reads the range in order.
    "FOR x IN mv FILTER x.d {op} @f COLLECT m = x.m AGGREGATE keys = UNIQUE(x.tag)
       RETURN {m, keys}",
];

#[test]
fn a_range_collect_is_the_materialising_paths() {
    let (engine, _tmp) = engine();
    let materialised = Mode {
        stream: false,
        ..FAST
    };
    for template in COLLECTS {
        for op in [">=", ">", "<", "<="] {
            for bound in [
                json!(1055),     // a few days: read through the index
                json!(1054.999), // the same documents, another bound
                json!(1040),     // a third of the days
                json!(1010),     // most of them
                json!(0),
                json!(5000),
                json!(1058.0),
            ] {
                let q = template.replace("{op}", op);
                let b = binds(&[("f", bound.clone())]);
                let expected = run(&engine, &q, materialised, &b);
                assert!(expected.is_ok(), "{q} {bound}: {expected:?}");
                for threads in [4, 1] {
                    let got = run(&engine, &q, Mode { threads, ..FAST }, &b);
                    assert_eq!(got, expected, "{q} {bound} on {threads} thread(s)");
                }
                // And what a collection with no index answers: the same,
                // but for `UNIQUE`, which keeps the order of the rows, and a
                // scan reads them in key order where an index reads them
                // in its own.
                if !q.contains("UNIQUE") {
                    let plain = run(&engine, &q.replace(" mv ", " plain "), FAST, &b);
                    assert_eq!(plain, expected, "{q} {bound} without an index");
                }
            }
        }
    }
}

#[test]
fn explain_says_which_way_a_range_collect_reads() {
    let (engine, _tmp) = engine();
    let q = COLLECTS[0].replace("{op}", ">=");
    let few = warnings(&engine, &q, &binds(&[("f", json!(1055))]));
    assert!(
        few.iter()
            .any(|w| w.contains("COLLECT folded into the index read of 'mv'")),
        "{few:?}"
    );
    // However much of the collection the range names: a scan would read
    // in another order.
    let most = warnings(&engine, &q, &binds(&[("f", json!(1010))]));
    assert_eq!(few, most);
}

// ---------- a filtered REMOVE ----------

/// What `mv` holds and what its three indexes answer, read every way.
fn state(engine: &StorageEngine) -> Vec<Value> {
    let none = BindVars::new();
    let mut out = Vec::new();
    for q in [
        "FOR x IN mv SORT x._key RETURN x._key",
        "FOR x IN mv FILTER x.d >= 0 SORT x._key RETURN x._key",
        "FOR x IN mv FILTER x.d == 1003 SORT x._key RETURN x._key",
        "FOR x IN mv FILTER x.m == 4 AND x.d >= 1000 SORT x._key RETURN x._key",
        "FOR x IN mv FILTER x.tag == 't3' SORT x._key RETURN x._key",
        "FOR x IN mv FILTER x.d == 1003 COLLECT WITH COUNT INTO n RETURN n",
        "RETURN COLLECTION_COUNT('mv')",
    ] {
        out.push(json!(run(engine, q, FAST, &none).unwrap()));
    }
    let mv = engine
        .get_database(DB)
        .unwrap()
        .get_collection("mv")
        .unwrap();
    // The index itself, not a query that re-checks its documents.
    for d in [1003, 1030, 1059] {
        let keys: Vec<String> = mv
            .index_lookup_eq("d", &json!(d))
            .unwrap()
            .into_iter()
            .map(|doc| doc.key)
            .collect();
        out.push(json!(keys.len()));
    }
    out
}

#[test]
fn a_range_remove_is_the_row_by_row_ones_indexes_included() {
    let by_rows = Mode {
        typed: false,
        ..FAST
    };
    for (q, b) in [
        (
            "FOR x IN mv FILTER x.d < @c REMOVE x IN mv",
            binds(&[("c", json!(1030))]),
        ),
        (
            "FOR x IN mv FILTER x.d < @c REMOVE x._key IN mv",
            binds(&[("c", json!(1029.5))]),
        ),
        (
            "FOR x IN mv FILTER x.d >= @c FILTER x.m != 2 REMOVE x IN mv",
            binds(&[("c", json!(1010))]),
        ),
        (
            "FOR x IN mv FILTER x.tag == 't3' OR x.d == null REMOVE x IN mv",
            BindVars::new(),
        ),
        (
            "FOR x IN mv FILTER x.d < @c REMOVE x IN mv",
            binds(&[("c", json!(-5))]),
        ),
        ("FOR x IN mv REMOVE x IN mv", BindVars::new()),
        // Shapes the scan leaves to the pipeline: same effect.
        (
            "LET c = 1030 FOR x IN mv FILTER x.d < c REMOVE x IN mv",
            BindVars::new(),
        ),
        (
            "FOR x IN mv FILTER x.d < 1030 REMOVE x IN mv RETURN OLD._key",
            BindVars::new(),
        ),
    ] {
        let (fast, _a) = engine();
        let (slow, _b) = engine();
        let run_stats = |engine: &StorageEngine, mode: Mode| {
            let query = parse(q).unwrap();
            let result = executor(engine, mode, &b)
                .execute_with_stats(&query)
                .unwrap();
            let mut rows = result.results;
            rows.sort_by_key(|v| v.to_string());
            (result.mutations.documents_removed, rows)
        };
        let got = run_stats(&fast, FAST);
        let expected = run_stats(&slow, by_rows);
        assert_eq!(got, expected, "{q}");
        assert_eq!(state(&fast), state(&slow), "{q}");
        // On one thread too.
        let (alone, _c) = engine();
        assert_eq!(run_stats(&alone, one_thread()), expected, "{q}");
        assert_eq!(state(&alone), state(&slow), "{q}");
    }
}

#[test]
fn a_range_remove_leaves_no_index_entry_behind() {
    let (engine, _tmp) = engine();
    let b = binds(&[("c", json!(1030))]);
    let query = parse("FOR x IN mv FILTER x.d < @c REMOVE x IN mv").unwrap();
    let removed = executor(&engine, FAST, &b)
        .execute_with_stats(&query)
        .unwrap()
        .mutations
        .documents_removed;
    // 30 days of every metric, and the two documents whose `d` is null or
    // missing (null is below every number).
    assert_eq!(removed, METRICS * 30 + 2);
    let mv = engine
        .get_database(DB)
        .unwrap()
        .get_collection("mv")
        .unwrap();
    assert_eq!(mv.count(), METRICS * 30 + 3);
    for d in 1000..1030 {
        assert!(
            mv.index_lookup_eq("d", &json!(d)).unwrap().is_empty(),
            "{d}"
        );
    }
    assert_eq!(
        mv.index_lookup_eq("d", &json!(1030)).unwrap().len(),
        METRICS
    );
    assert_eq!(
        mv.index_lookup_eq("tag", &json!("t3")).unwrap().len(),
        run(
            &engine,
            "FOR x IN plain FILTER x.tag == 't3' AND NOT (x.d < 1030) RETURN 1",
            FAST,
            &BindVars::new()
        )
        .unwrap()
        .len()
    );
    // The same documents can be stored again: nothing of them is left.
    let back: Vec<Value> = metric_docs()
        .into_iter()
        .filter(|doc| doc["d"].as_u64().is_some_and(|d| d < 1030))
        .collect();
    assert_eq!(mv.insert_batch(back).unwrap().len(), METRICS * 30);
    assert_eq!(
        mv.index_lookup_eq("d", &json!(1003)).unwrap().len(),
        METRICS
    );
}

#[test]
fn a_subscriber_is_told_what_a_batch_removed() {
    let (engine, _tmp) = engine();
    let mv = engine
        .get_database(DB)
        .unwrap()
        .get_collection("mv")
        .unwrap();
    let mut events = mv.change_sender.subscribe();
    // More than the batch size from which a delete decodes only the
    // indexed fields when nobody listens.
    run(
        &engine,
        "FOR x IN mv FILTER x.d < 1002 REMOVE x IN mv",
        FAST,
        &BindVars::new(),
    )
    .unwrap();
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        // The whole document, not only what the indexes read of it.
        let old = event.old_data.expect("the removed document");
        assert!(old["s"].is_number(), "{old}");
        if event.key.starts_with('m') {
            assert_eq!(old["n"], json!(5), "{old}");
            assert!(old["v"].is_object(), "{old}");
        }
        seen.push(event.key);
    }
    seen.sort();
    assert_eq!(seen.len(), METRICS * 2 + 2);
    assert_eq!(seen[0], "m10_000");
}
