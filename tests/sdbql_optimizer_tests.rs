//! Query optimizer: constant folding (audit P8), IN / LIKE-prefix /
//! STARTS_WITH index use (P9, P1), and the geo-index rules (P11).
//!
//! Every optimization is checked the same way: the optimized query must return
//! exactly what a variant that defeats the optimization returns (`x + 0`,
//! `CONCAT(x, "")` — anything the optimizer cannot see through), and EXPLAIN
//! must report the access path.

use std::collections::HashMap;

use serde_json::{json, Value};
use solidb::parse;
use solidb::sdbql::{BodyClause, Expression, QueryExecutor};
use solidb::storage::{IndexType, StorageEngine};
use tempfile::TempDir;

const DB: &str = "testdb";

fn engine() -> (StorageEngine, TempDir) {
    let tmp = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).expect("engine");
    engine.create_database(DB.to_string()).unwrap();
    (engine, tmp)
}

fn run_with(engine: &StorageEngine, q: &str, binds: HashMap<String, Value>) -> Vec<Value> {
    let exec = QueryExecutor::with_database_and_bind_vars(engine, DB.to_string(), binds);
    exec.execute(&parse(q).unwrap_or_else(|e| panic!("parse {q}: {e:?}")))
        .unwrap_or_else(|e| panic!("execute {q}: {e:?}"))
}

fn run(engine: &StorageEngine, q: &str) -> Vec<Value> {
    run_with(engine, q, HashMap::new())
}

/// Execute `q` once as parsed and once after `fold_constants`; both must agree.
fn run_folded_and_plain(
    engine: &StorageEngine,
    q: &str,
    binds: HashMap<String, Value>,
) -> (Vec<Value>, usize) {
    let exec = QueryExecutor::with_database_and_bind_vars(engine, DB.to_string(), binds);
    let parsed = parse(q).unwrap_or_else(|e| panic!("parse {q}: {e:?}"));
    let plain = exec.execute(&parsed).unwrap();
    let mut folded = parsed.clone();
    let n = exec.fold_constants(&mut folded);
    let folded_result = exec.execute(&folded).unwrap();
    assert_eq!(plain, folded_result, "folding changed the result of {q}");
    (plain, n)
}

fn seed_people(engine: &StorageEngine) {
    let db = engine.get_database(DB).unwrap();
    db.create_collection("people".to_string(), None).unwrap();
    let c = db.get_collection("people").unwrap();
    for i in 0..200 {
        c.insert(json!({
            "_key": format!("k{:03}", i),
            "age": i % 50,
            "name": format!("User{}", i),
        }))
        .unwrap();
    }
    // Values the prefix scan must neither miss nor wrongly include.
    for (k, name) in [
        ("x1", json!("user1")),
        ("x2", json!("User")),
        ("x3", json!("Use")),
        ("x4", json!(1234)),
        ("x5", json!("User1\u{10FFFF}z")),
        ("x6", json!("éclair")),
        ("x7", json!("éclat")),
        ("x8", json!("User1_multi\nline")),
    ] {
        c.insert(json!({"_key": k, "age": 999, "name": name}))
            .unwrap();
    }
    c.insert(json!({"_key": "x9", "age": 7})).unwrap(); // no name
}

fn index(engine: &StorageEngine, coll: &str, name: &str, field: &str, ty: IndexType) {
    let c = engine
        .get_database(DB)
        .unwrap()
        .get_collection(coll)
        .unwrap();
    c.create_index(name.to_string(), vec![field.to_string()], ty, false)
        .unwrap();
}

fn explain_access(engine: &StorageEngine, q: &str) -> solidb::sdbql::QueryExplain {
    let exec = QueryExecutor::with_database(engine, DB.to_string());
    exec.explain(&parse(q).unwrap()).unwrap()
}

// ============================================================================
// Constant folding
// ============================================================================

#[test]
fn fold_replaces_pure_subtrees_with_literals() {
    let (engine, _tmp) = engine();
    let exec = QueryExecutor::with_database_and_bind_vars(
        &engine,
        DB.to_string(),
        HashMap::from([("q".to_string(), json!("ALICE"))]),
    );
    let mut q = parse("FOR d IN [1] FILTER d == LOWER(@q) RETURN d").unwrap();
    assert!(exec.fold_constants(&mut q) >= 1);
    let filter = q
        .body_clauses
        .iter()
        .find_map(|c| match c {
            BodyClause::Filter(f) => Some(&f.expression),
            _ => None,
        })
        .expect("filter");
    match filter {
        Expression::BinaryOp { right, .. } => {
            assert_eq!(**right, Expression::Literal(json!("alice")))
        }
        other => panic!("unexpected filter {other:?}"),
    }
}

#[test]
fn fold_leaves_nondeterministic_and_context_functions() {
    let (engine, _tmp) = engine();
    let exec = QueryExecutor::with_database(&engine, DB.to_string());
    for q in [
        "RETURN RAND()",
        "RETURN DATE_NOW()",
        "RETURN UUID()",
        "RETURN CURRENT_USER()",
        "RETURN COLLECTION_COUNT(\"people\")",
        "RETURN DATE_YEAR()",
    ] {
        let mut parsed = parse(q).unwrap();
        let before = parsed.clone();
        assert_eq!(exec.fold_constants(&mut parsed), 0, "{q} must not fold");
        assert_eq!(parsed, before, "{q} must be unchanged");
    }
}

#[test]
fn fold_keeps_for_range_source_and_folds_small_ranges_elsewhere() {
    let (engine, _tmp) = engine();
    let exec = QueryExecutor::with_database(&engine, DB.to_string());
    let mut q = parse("FOR i IN 1..(2+1) FILTER i IN 1..2 RETURN i").unwrap();
    exec.fold_constants(&mut q);
    match &q.body_clauses[0] {
        BodyClause::For(f) => match f.source_expression.as_ref().unwrap() {
            Expression::Range(a, b) => {
                assert_eq!(**a, Expression::Literal(json!(1)));
                assert!(matches!(**b, Expression::Literal(_)), "bound folded");
            }
            other => panic!("FOR source must stay a range, got {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
    match &q.body_clauses[1] {
        BodyClause::Filter(f) => match &f.expression {
            Expression::BinaryOp { right, .. } => {
                assert_eq!(**right, Expression::Literal(json!([1, 2])))
            }
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn fold_error_in_untaken_branch_does_not_fail_the_query() {
    let (engine, _tmp) = engine();
    let (res, _) = run_folded_and_plain(&engine, "RETURN false ? LOWER() : 2", HashMap::new());
    assert_eq!(res, vec![json!(2)]);
}

#[test]
fn folded_queries_return_the_same_results() {
    let (engine, _tmp) = engine();
    seed_people(&engine);
    let binds = || {
        HashMap::from([
            ("q".to_string(), json!("USER1")),
            ("ids".to_string(), json!(["k001", "k002"])),
        ])
    };
    let mut total = 0;
    for q in [
        "RETURN LOWER(@q)",
        "RETURN 1 + 2 * 3",
        "RETURN CONCAT(\"a\", @q, TO_STRING(5))",
        "RETURN [1, 2, 3][1]",
        "RETURN MERGE({a: 1}, {b: [1, 2]})",
        "FOR x IN 1..5 FILTER x IN [2, 1 + 2] RETURN x",
        "RETURN DATE_ADD(\"2024-01-01T00:00:00Z\", 1, \"day\")",
        "LET y = 3 RETURN y + (2 * 2)",
        "FOR x IN [1, 2, 3] RETURN x * (10 + 1)",
        "RETURN (FOR i IN 1..3 RETURN i * (1 + 1))",
        "FOR d IN people FILTER d._key IN @ids SORT d._key RETURN d.name",
        "FOR d IN people FILTER LOWER(d.name) == LOWER(@q) RETURN d._key",
        "FOR d IN people SORT d._key LIMIT 2, 4 RETURN CONCAT(d._key, UPPER(\"-x\"))",
    ] {
        total += run_folded_and_plain(&engine, q, binds()).1;
    }
    assert!(total > 0, "at least some of these must fold");
}

#[test]
fn explain_reports_folded_constants() {
    let (engine, _tmp) = engine();
    seed_people(&engine);
    let e = explain_access(&engine, "FOR d IN people FILTER d.age == 2 + 3 RETURN d");
    assert!(
        e.warnings.iter().any(|w| w.contains("folded")),
        "{:?}",
        e.warnings
    );
    assert!(e.filters.iter().any(|f| !f.expression.contains('+')));
}

// ============================================================================
// IN / LIKE / STARTS_WITH index use
// ============================================================================

#[test]
fn in_list_uses_index_and_matches_scan() {
    let (engine, _tmp) = engine();
    seed_people(&engine);
    let fast = "FOR d IN people FILTER d.age IN [5, 7, 7, 300, 7.0] SORT d._key RETURN d._key";
    let slow = "FOR d IN people FILTER d.age + 0 IN [5, 7, 7, 300, 7.0] SORT d._key RETURN d._key";
    let before = run(&engine, fast);
    index(&engine, "people", "age_idx", "age", IndexType::Persistent);
    let after = run(&engine, fast);
    assert_eq!(after, run(&engine, slow));
    assert_eq!(after, before);
    assert_eq!(after.len(), 4 + 4 + 1, "ages 5 and 7 (x4 each) plus x9");

    let e = explain_access(&engine, fast);
    assert_eq!(e.collections[0].access_type, "index_lookup");
    assert_eq!(e.collections[0].index_used.as_deref(), Some("age_idx"));

    // Bind-variable list, and a LIMIT that is pushed into the lookup.
    let binds = HashMap::from([("ages".to_string(), json!([1, 2, 3]))]);
    let got = run_with(
        &engine,
        "FOR d IN people FILTER d.age IN @ages SORT d._key RETURN d._key",
        binds.clone(),
    );
    let want = run_with(
        &engine,
        "FOR d IN people FILTER d.age + 0 IN @ages SORT d._key RETURN d._key",
        binds.clone(),
    );
    assert_eq!(got, want);
    let limited = run_with(
        &engine,
        "FOR d IN people FILTER d.age IN @ages LIMIT 5 RETURN d",
        binds,
    );
    assert_eq!(limited.len(), 5);

    // Empty list and a list containing null (null is never indexed).
    assert!(run(&engine, "FOR d IN people FILTER d.age IN [] RETURN d").is_empty());
    assert_eq!(
        run(
            &engine,
            "FOR d IN people FILTER d.name IN [null] RETURN d._key"
        ),
        vec![json!("x9")]
    );
}

#[test]
fn key_in_list_uses_primary_key() {
    let (engine, _tmp) = engine();
    seed_people(&engine);
    let got = run(
        &engine,
        "FOR d IN people FILTER d._key IN [\"k010\", \"nope\", \"k003\", \"k010\"] SORT d._key RETURN d._key",
    );
    assert_eq!(got, vec![json!("k003"), json!("k010")]);
    let e = explain_access(
        &engine,
        "FOR d IN people FILTER d._key IN [\"k010\", \"k003\"] RETURN d",
    );
    assert_eq!(e.collections[0].access_type, "index_lookup");
}

#[test]
fn like_prefix_and_starts_with_use_index_and_match_scan() {
    let (engine, _tmp) = engine();
    seed_people(&engine);
    index(&engine, "people", "name_idx", "name", IndexType::Persistent);

    for (fast, slow) in [
        (
            "FOR d IN people FILTER d.name LIKE \"User1%\" SORT d._key RETURN d._key",
            "FOR d IN people FILTER CONCAT(d.name, \"\") LIKE \"User1%\" SORT d._key RETURN d._key",
        ),
        (
            "FOR d IN people FILTER d.name LIKE \"User1_\" SORT d._key RETURN d._key",
            "FOR d IN people FILTER CONCAT(d.name, \"\") LIKE \"User1_\" SORT d._key RETURN d._key",
        ),
        (
            "FOR d IN people FILTER d.name LIKE \"User\" SORT d._key RETURN d._key",
            "FOR d IN people FILTER CONCAT(d.name, \"\") LIKE \"User\" SORT d._key RETURN d._key",
        ),
        (
            "FOR d IN people FILTER STARTS_WITH(d.name, \"User1\") SORT d._key RETURN d._key",
            "FOR d IN people FILTER STARTS_WITH(CONCAT(d.name, \"\"), \"User1\") SORT d._key RETURN d._key",
        ),
        (
            "FOR d IN people FILTER d.name LIKE \"écl%\" SORT d._key RETURN d._key",
            "FOR d IN people FILTER CONCAT(d.name, \"\") LIKE \"écl%\" SORT d._key RETURN d._key",
        ),
        (
            "FOR d IN people FILTER d.name LIKE CONCAT(\"User\", \"19%\") AND d.age > 10 SORT d._key RETURN d._key",
            "FOR d IN people FILTER CONCAT(d.name, \"\") LIKE \"User19%\" AND d.age > 10 SORT d._key RETURN d._key",
        ),
    ] {
        let a = run(&engine, fast);
        let b = run(&engine, slow);
        assert_eq!(a, b, "{fast}");
    }

    let e = explain_access(
        &engine,
        "FOR d IN people FILTER d.name LIKE \"User1%\" RETURN d",
    );
    assert_eq!(e.collections[0].access_type, "index_lookup");
    assert_eq!(e.collections[0].index_used.as_deref(), Some("name_idx"));
    let e = explain_access(
        &engine,
        "FOR d IN people FILTER STARTS_WITH(d.name, \"User1\") RETURN d",
    );
    assert_eq!(e.collections[0].access_type, "index_lookup");
    // A leading wildcard has no prefix to scan.
    let e = explain_access(
        &engine,
        "FOR d IN people FILTER d.name LIKE \"%1\" RETURN d",
    );
    assert_eq!(e.collections[0].access_type, "full_scan");
}

#[test]
fn second_conjunct_index_is_used_when_first_has_none() {
    let (engine, _tmp) = engine();
    seed_people(&engine);
    index(&engine, "people", "age_idx", "age", IndexType::Hash);
    // The planner used to try only the first indexable conjunct (`name`,
    // which has no index) and then scan.
    let fast =
        "FOR d IN people FILTER d.name == \"User55\" AND d.age == 5 SORT d._key RETURN d._key";
    let slow =
        "FOR d IN people FILTER d.name == \"User55\" AND d.age + 0 == 5 SORT d._key RETURN d._key";
    assert_eq!(run(&engine, fast), vec![json!("k055")]);
    assert_eq!(run(&engine, fast), run(&engine, slow));
    let e = explain_access(&engine, fast);
    assert_eq!(e.collections[0].access_type, "index_lookup");
}

// ============================================================================
// Geo index
// ============================================================================

fn seed_places(engine: &StorageEngine, field_path: &[&str], with_missing: bool) {
    let db = engine.get_database(DB).unwrap();
    db.create_collection("places".to_string(), None).unwrap();
    let c = db.get_collection("places").unwrap();
    let mut n = 0;
    for i in 0..20 {
        for j in 0..20 {
            let point = json!({"lat": 48.0 + i as f64 * 0.1, "lon": 2.0 + j as f64 * 0.1});
            let mut doc = point;
            for key in field_path.iter().rev() {
                let mut wrapper = serde_json::Map::new();
                wrapper.insert(key.to_string(), doc);
                doc = Value::Object(wrapper);
            }
            doc["_key"] = json!(format!("p{:03}", n));
            c.insert(doc).unwrap();
            n += 1;
        }
    }
    if with_missing {
        c.insert(json!({"_key": "zz_noloc"})).unwrap();
        c.insert(json!({"_key": "zz_badloc", "loc": {"lat": "x", "lon": 2.0}}))
            .unwrap();
    }
    c.create_geo_index("geo_loc".to_string(), field_path.join("."))
        .unwrap();
}

#[test]
fn geo_filter_uses_geo_index_and_matches_scan() {
    let (engine, _tmp) = engine();
    seed_places(&engine, &["loc"], true);

    for (fast, slow) in [
        (
            "FOR p IN places FILTER IS_NUMBER(p.loc.lat) AND DISTANCE(p.loc.lat, p.loc.lon, 48.85, 2.35) <= 30000 SORT p._key RETURN p._key",
            "FOR p IN places FILTER IS_NUMBER(p.loc.lat) AND DISTANCE(p.loc.lat + 0, p.loc.lon, 48.85, 2.35) <= 30000 SORT p._key RETURN p._key",
        ),
        (
            "FOR p IN places FILTER IS_NUMBER(p.loc.lat) AND 30000 > DISTANCE(48.85, 2.35, p.loc.lat, p.loc.lon) SORT p._key RETURN p._key",
            "FOR p IN places FILTER IS_NUMBER(p.loc.lat) AND 30000 > DISTANCE(48.85, 2.35, p.loc.lat + 0, p.loc.lon) SORT p._key RETURN p._key",
        ),
        (
            "FOR p IN places FILTER IS_NUMBER(p.loc.lat) AND GEO_DISTANCE(p.loc, {lat: 48.85, lon: 2.35}) <= 20000 AND p.loc.lat > 48.5 SORT p._key RETURN p._key",
            "FOR p IN places FILTER IS_NUMBER(p.loc.lat) AND GEO_DISTANCE(MERGE(p.loc, {}), {lat: 48.85, lon: 2.35}) <= 20000 AND p.loc.lat > 48.5 SORT p._key RETURN p._key",
        ),
    ] {
        let a = run(&engine, fast);
        let b = run(&engine, slow);
        assert!(!a.is_empty(), "{fast}");
        assert_eq!(a, b, "{fast}");
        let e = explain_access(&engine, fast);
        assert_eq!(e.collections[0].access_type, "index_lookup", "{fast}");
        assert_eq!(e.collections[0].index_type.as_deref(), Some("Geo"), "{fast}");
    }
}

#[test]
fn geo_filter_on_nested_field_after_backfill() {
    let (engine, _tmp) = engine();
    seed_places(&engine, &["info", "pos"], false);
    let fast = "FOR p IN places FILTER IS_NUMBER(p.info.pos.lat) AND DISTANCE(p.info.pos.lat, p.info.pos.lon, 49.0, 3.0) <= 15000 SORT p._key RETURN p._key";
    let slow = "FOR p IN places FILTER IS_NUMBER(p.info.pos.lat) AND DISTANCE(p.info.pos.lat + 0, p.info.pos.lon, 49.0, 3.0) <= 15000 SORT p._key RETURN p._key";
    let a = run(&engine, fast);
    assert!(!a.is_empty());
    assert_eq!(a, run(&engine, slow));
    let e = explain_access(&engine, fast);
    assert_eq!(e.collections[0].index_type.as_deref(), Some("Geo"));
}

#[test]
fn geo_sort_limit_uses_geo_index_when_every_document_is_indexed() {
    let (engine, _tmp) = engine();
    seed_places(&engine, &["loc"], false);
    let fast =
        "FOR p IN places SORT DISTANCE(p.loc.lat, p.loc.lon, 48.83, 2.37) LIMIT 2, 7 RETURN p._key";
    let slow = "FOR p IN places SORT DISTANCE(p.loc.lat + 0, p.loc.lon, 48.83, 2.37) LIMIT 2, 7 RETURN p._key";
    let a = run(&engine, fast);
    assert_eq!(a.len(), 7);
    assert_eq!(a, run(&engine, slow));
    let e = explain_access(&engine, fast);
    assert_eq!(e.collections[0].access_type, "geo_index_sort");
    assert_eq!(e.collections[0].index_used.as_deref(), Some("geo_loc"));

    // Exact ties (a grid point equidistant from the centre) break like the scan.
    let tie =
        "FOR p IN places SORT DISTANCE(p.loc.lat, p.loc.lon, 48.95, 2.55) LIMIT 4 RETURN p._key";
    let tie_slow =
        "FOR p IN places SORT DISTANCE(p.loc.lat + 0, p.loc.lon, 48.95, 2.55) LIMIT 4 RETURN p._key";
    assert_eq!(run(&engine, tie), run(&engine, tie_slow));
}

#[test]
fn geo_sort_is_not_used_when_a_document_lacks_a_point() {
    let (engine, _tmp) = engine();
    seed_places(&engine, &["loc"], true);
    let fast =
        "FOR p IN places SORT DISTANCE(p.loc.lat, p.loc.lon, 48.83, 2.37) LIMIT 5 RETURN p._key";
    let e = explain_access(&engine, fast);
    assert_ne!(e.collections[0].access_type, "geo_index_sort");
}

#[test]
fn geo_near_returns_nearest_first() {
    let (engine, _tmp) = engine();
    seed_places(&engine, &["loc"], false);
    let c = engine
        .get_database(DB)
        .unwrap()
        .get_collection("places")
        .unwrap();
    let near = c.geo_near("loc", 48.83, 2.37, 5).unwrap();
    assert_eq!(near.len(), 5);
    assert!(near.windows(2).all(|w| w[0].1 <= w[1].1));
    // The nearest grid point is (48.8, 2.4).
    let first = near[0].0.to_value();
    assert_eq!(
        first["loc"]["lat"].as_f64().map(|v| (v * 10.0).round()),
        Some(488.0)
    );
    assert_eq!(
        first["loc"]["lon"].as_f64().map(|v| (v * 10.0).round()),
        Some(24.0)
    );

    let within = c.geo_within("loc", 48.83, 2.37, 10_000.0).unwrap();
    assert!(!within.is_empty());
    assert!(within.iter().all(|(_, d)| *d <= 10_000.0));
}
