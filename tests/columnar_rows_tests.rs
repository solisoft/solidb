//! A columnar collection read as rows by SDBQL (card #891): the columns
//! the query reads and the rows its FILTERs keep are all that is built.
//! Every query here is also run on a document collection holding the same
//! rows, and must answer the same.

use serde_json::{json, Value};
use solidb::sdbql::{parse, QueryExecutor};
use solidb::storage::columnar::ColumnarIndexType;
use solidb::sync::columnar;
use solidb::{ColumnDef, ColumnType, ColumnarCollection, CompressionType, StorageEngine};
use std::collections::HashMap;
use tempfile::TempDir;

const ROWS: i64 = 20_000;

/// A columnar collection `m` and a document collection `d` with the same
/// rows: most in a sealed block, a few in the tail.
fn fixture() -> (StorageEngine, ColumnarCollection, TempDir) {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    let column = |name: &str, data_type, nullable| ColumnDef {
        name: name.to_string(),
        data_type,
        nullable,
        indexed: false,
        index_type: None,
    };
    let columns = vec![
        column("host", ColumnType::String, false),
        column("v", ColumnType::Int64, false),
        column("ms", ColumnType::Float64, true),
        column("ok", ColumnType::Bool, false),
        column("n", ColumnType::Int64, true),
    ];
    let col = columnar::create(&db, None, "m", columns, CompressionType::Lz4).unwrap();
    let rows: Vec<Value> = (0..ROWS)
        .map(|i| {
            let ms = if i % 4 == 0 {
                Value::Null
            } else {
                json!(i as f64 / 8.0)
            };
            let n = if i % 5 == 0 { Value::Null } else { json!(i) };
            json!({"host": format!("h{}", i % 7), "v": i, "ms": ms, "ok": i % 3 == 0, "n": n})
        })
        .collect();
    // Two blocks, so that a range may pass one over and take the other
    // whole, and a tail.
    col.insert_rows(rows[..8_000].to_vec()).unwrap();
    col.insert_rows(rows[8_000..19_990].to_vec()).unwrap();
    col.insert_rows(rows[19_990..].to_vec()).unwrap();
    db.create_collection("d".to_string(), None).unwrap();
    db.get_collection("d").unwrap().insert_batch(rows).unwrap();
    (engine, col, dir)
}

fn run(engine: &StorageEngine, query: &str, binds: &[(&str, Value)]) -> Vec<Value> {
    let binds: HashMap<String, Value> = binds
        .iter()
        .map(|(name, value)| (name.to_string(), value.clone()))
        .collect();
    QueryExecutor::with_database_and_bind_vars(engine, "app".to_string(), binds)
        .execute(&parse(query).expect("query parses"))
        .unwrap_or_else(|e| panic!("{query}: {e}"))
}

/// The queries, with `$c` for the collection: each sorts what it returns,
/// so that both collections are compared row for row.
fn queries() -> Vec<(&'static str, Vec<(&'static str, Value)>)> {
    vec![
        ("FOR r IN $c FILTER r.host == 'h3' SORT r.v RETURN r.v", vec![]),
        (
            "FOR r IN $c FILTER r.v >= 100 AND r.v < 120 SORT r.v RETURN {host: r.host, v: r.v, ms: r.ms, ok: r.ok}",
            vec![],
        ),
        (
            "FOR r IN $c FILTER r.host == @h AND r.ok == true SORT r.v RETURN [r.v, r.ms]",
            vec![("h", json!("h5"))],
        ),
        ("FOR r IN $c FILTER 19990 <= r.v SORT r.v RETURN [r.v, r.host]", vec![]),
        ("FOR r IN $c FILTER @low < r.v AND @high > r.v SORT r.v RETURN r.v", vec![("low", json!(64)), ("high", json!(70))]),
        ("FOR r IN $c FILTER r.v == 5 OR r.host == 'h1' SORT r.v LIMIT 20 RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.v < 3 OR r.v > 19996 OR r.v == 777 SORT r.v RETURN r.v", vec![]),
        // Part of the FILTER is the collection's to answer, part is not.
        ("FOR r IN $c FILTER r.ms > 2499.5 AND r.host == 'h2' SORT r.v RETURN [r.v, r.ms]", vec![]),
        ("FOR r IN $c FILTER r.v == 5 OR r.ms == 1.25 SORT r.v RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.host == 'h4' FILTER r.v % 1000 == 4 SORT r.v RETURN r.v", vec![]),
        // Constants of another kind than the column, nulls, strings in order.
        ("FOR r IN $c FILTER r.v == 5.0 SORT r.v RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.v == '5' SORT r.v RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.v > 19997.5 SORT r.v RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.ms == null AND r.v < 20 SORT r.v RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.ms != null AND r.v < 6 SORT r.v RETURN [r.v, r.ms]", vec![]),
        ("FOR r IN $c FILTER r.host > 'h5' AND r.v < 30 SORT r.v RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.host == 'nobody' RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.ok == false AND r.v < 10 SORT r.v RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.missing == null AND r.v < 3 SORT r.v RETURN [r.v, r.missing]", vec![]),
        // Ranges of a column that holds nulls: to SDBQL null is below
        // every number, so `<` keeps those rows (card #893: they were lost).
        ("FOR r IN $c FILTER r.n < 4 SORT r.v RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.n <= 6 AND r.v < 40 SORT r.v RETURN [r.v, r.n]", vec![]),
        ("FOR r IN $c FILTER r.n > 19990 SORT r.v RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.n >= @n SORT r.v RETURN r.n", vec![("n", json!(19996))]),
        ("FOR r IN $c FILTER r.n == null AND r.v > 19980 SORT r.v RETURN r.v", vec![]),
        // Ranges of floats, against a float and against an integer.
        ("FOR r IN $c FILTER r.ms < 0.5 AND r.v < 9 SORT r.v RETURN [r.v, r.ms]", vec![]),
        ("FOR r IN $c FILTER r.ms > 2499.5 SORT r.v RETURN r.ms", vec![]),
        ("FOR r IN $c FILTER r.ms >= 100 AND r.ms <= 101 SORT r.v RETURN r.ms", vec![]),
        ("FOR r IN $c FILTER r.ms >= 2499 SORT r.v RETURN r.v", vec![]),
        // Counts, which the collection answers without building a row
        // when every comparison is its own; and one it must leave.
        ("FOR r IN $c FILTER r.v >= 5000 AND r.v < 15000 COLLECT WITH COUNT INTO n RETURN n", vec![]),
        ("FOR r IN $c FILTER r.host == 'h3' COLLECT WITH COUNT INTO n RETURN {n}", vec![]),
        ("RETURN LENGTH(FOR r IN $c FILTER r.n < 100 RETURN 1)", vec![]),
        ("RETURN LENGTH(FOR r IN $c FILTER r.ms >= 10.5 AND r.ms < 2000 RETURN r)", vec![]),
        ("RETURN LENGTH(FOR r IN $c FILTER r.ok == true OR r.v > 19990 RETURN r.v)", vec![]),
        ("RETURN LENGTH(FOR r IN $c FILTER r.v < @n AND r.v % 2 == 0 RETURN 1)", vec![("n", json!(100))]),
        ("FOR x IN [10, 1000] RETURN LENGTH(FOR r IN $c FILTER r.v < x RETURN 1)", vec![]),
        // Ranges of floats that a block's bounds rule out or take whole.
        ("FOR r IN $c FILTER r.ms > 5000 RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.ms >= 1100.5 AND r.ms < 1101.5 SORT r.v RETURN r.v", vec![]),
        ("RETURN LENGTH(FOR r IN $c FILTER r.ms >= 0 RETURN 1)", vec![]),
        // A range no block holds, one that takes whole blocks, and both ends.
        ("FOR r IN $c FILTER r.v > 50000 RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.v >= 0 COLLECT WITH COUNT INTO n RETURN n", vec![]),
        ("FOR r IN $c FILTER r.v >= 19985 AND r.v < 19995 SORT r.v RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.v < 3 OR r.v > 19996 OR r.host == 'nobody' SORT r.v RETURN r.v", vec![]),
        // Limits, with and without a FILTER, and a sort that picks the end.
        ("FOR r IN $c SORT r.v DESC LIMIT 3 RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.host == 'h1' SORT r.v LIMIT 2, 3 RETURN r.v", vec![]),
        // A FILTER that names an outer row is not the collection's to answer.
        ("FOR x IN [3, 9000, 19995] FOR r IN $c FILTER r.v == x RETURN [x, r.host]", vec![]),
        ("FOR x IN ['h0', 'h6'] FOR r IN $c FILTER r.host == x AND r.v < 14 SORT r.v RETURN [x, r.v]", vec![]),
        // Counted, not returned.
        ("FOR r IN $c FILTER r.host == 'h2' AND r.v >= 10000 COLLECT WITH COUNT INTO n RETURN n", vec![]),
        ("RETURN LENGTH(FOR r IN $c FILTER r.v < 100 RETURN 1)", vec![]),
        // ROLLUP's subtotals and grand total are of every group, not only
        // of those a SORT on an aggregate with a LIMIT returns.
        ("FOR r IN $c COLLECT h = r.host ROLLUP AGGREGATE s = SUM(r.v) SORT s DESC LIMIT 2 RETURN [h, s]", vec![]),
        ("FOR r IN $c COLLECT h = r.host, ok = r.ok CUBE AGGREGATE n = COUNT() SORT n DESC LIMIT 3 RETURN [h, ok, n]", vec![]),
        // A distinct count of a subtotal is of the union of the groups'
        // values, not the sum of their counts.
        ("FOR r IN $c FILTER r.v < 4000 COLLECT h = r.host ROLLUP AGGREGATE d = COUNT_DISTINCT(r.ok) SORT h RETURN [h, d]", vec![]),
    ]
}

fn compare(engine: &StorageEngine, what: &str) {
    for (query, binds) in queries() {
        let columnar = run(engine, &query.replace("$c", "m"), &binds);
        let documents = run(engine, &query.replace("$c", "d"), &binds);
        assert_eq!(columnar, documents, "{what}: {query}");
    }
}

#[test]
fn a_columnar_collection_answers_as_documents_do() {
    let (engine, col, _dir) = fixture();
    compare(&engine, "no index");

    // The same with indexes the collection may answer the FILTER from.
    col.create_index("host", ColumnarIndexType::Hash).unwrap();
    col.create_index("v", ColumnarIndexType::Sorted).unwrap();
    compare(&engine, "hash index on host, sorted index on v");
    col.drop_index("host").unwrap();
    col.drop_index("v").unwrap();
    col.create_index("host", ColumnarIndexType::Bitmap).unwrap();
    col.create_index("v", ColumnarIndexType::Bloom).unwrap();
    compare(&engine, "bitmap index on host, bloom index on v");

    // Rows written after the indexes are read too.
    col.insert_rows(vec![json!({"host": "h3", "v": 100, "ms": 1.0, "ok": true})])
        .unwrap();
    engine
        .get_database("app")
        .unwrap()
        .get_collection("d")
        .unwrap()
        .insert(json!({"host": "h3", "v": 100, "ms": 1.0, "ok": true}))
        .unwrap();
    compare(&engine, "after an insert");
}

#[test]
fn whole_rows_and_limits_without_a_sort() {
    let (engine, _col, _dir) = fixture();
    // The row whole holds every column, in the order of the ids.
    let rows = run(&engine, "FOR r IN m FILTER r.v == 7 RETURN r", &[]);
    assert_eq!(
        rows,
        [json!({"host": "h0", "v": 7, "ms": 0.875, "ok": false, "n": 7})]
    );
    let first = run(&engine, "FOR r IN m LIMIT 4 RETURN r.v", &[]);
    assert_eq!(first, [json!(0), json!(1), json!(2), json!(3)]);
    let page = run(&engine, "FOR r IN m LIMIT 19995, 10 RETURN r.v", &[]);
    assert_eq!(page, (19_995..20_000).map(|v| json!(v)).collect::<Vec<_>>());
    let kept = run(
        &engine,
        "FOR r IN m FILTER r.host == 'h1' LIMIT 3 RETURN r.v",
        &[],
    );
    assert_eq!(kept, [json!(1), json!(8), json!(15)]);
    // Only one column is asked for: the others are not in the way.
    let hosts = run(&engine, "FOR r IN m FILTER r.v < 2 RETURN r.host", &[]);
    assert_eq!(hosts, [json!("h0"), json!("h1")]);
}

#[test]
fn explain_says_how_a_columnar_collection_is_read() {
    let (engine, col, _dir) = fixture();
    let explain = |query: &str| {
        QueryExecutor::with_database(&engine, "app".to_string())
            .explain(&parse(query).expect("query parses"))
            .unwrap_or_else(|e| panic!("{query}: {e}"))
    };
    let query = "FOR r IN m FILTER r.host == 'h1' AND r.v > 19000 RETURN r.ms";
    let plan = explain(query);
    assert_eq!(plan.collections[0].access_type, "columnar_scan");
    assert_eq!(plan.collections[0].index_used, None);
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("3 of its 5 columns") && w.contains("`host`, `v`")),
        "{:?}",
        plan.warnings
    );

    // An equality is an index's; a range against a number is the arrays'.
    col.create_index("host", ColumnarIndexType::Hash).unwrap();
    col.create_index("v", ColumnarIndexType::Sorted).unwrap();
    let plan = explain(query);
    assert_eq!(plan.collections[0].access_type, "columnar_scan");
    assert_eq!(
        plan.collections[0].index_used.as_deref(),
        Some("host (hash)")
    );

    // A count the collection answers is not a scan; an aggregate folded
    // from the arrays is one.
    let plan =
        explain("FOR r IN m FILTER r.v >= 100 AND r.v < 200 COLLECT WITH COUNT INTO n RETURN n");
    assert_eq!(plan.collections[0].access_type, "columnar_count");
    assert_eq!(
        plan.warnings,
        ["`m` is columnar: the rows are counted in the column arrays, none is built"]
    );
    let plan = explain("RETURN LENGTH(FOR r IN m FILTER r.v < 50 RETURN 1)");
    assert_eq!(plan.collections[0].access_type, "columnar_count");
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("counted in the column arrays"))
            && !plan
                .warnings
                .iter()
                .any(|w| w.contains("columns are built")),
        "{:?}",
        plan.warnings
    );
    let plan = explain("FOR r IN m COLLECT h = r.host AGGREGATE s = SUM(r.v) RETURN [h, s]");
    assert_eq!(plan.collections[0].access_type, "columnar_scan");
    assert!(
        !plan
            .warnings
            .iter()
            .any(|w| w.contains("columns are built")),
        "{:?}",
        plan.warnings
    );

    // Nothing the collection answers: every row, the columns read only.
    let plan = explain("FOR r IN m FILTER r.v % 2 == 0 RETURN r.v");
    assert_eq!(plan.collections[0].access_type, "columnar_scan");
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("1 of its 5 columns") && w.contains("every row")),
        "{:?}",
        plan.warnings
    );
}

/// Card #955: a row policy on a columnar collection is applied to every
/// query that reads it, as on a document collection holding the same
/// rows. It is kept on the collection that backs it, under either name
/// (declared on the backing name it used to be stored and never applied,
/// card #896).
#[test]
fn a_row_policy_on_a_columnar_collection_is_applied() {
    use solidb::sdbql::QueryPrincipal;
    let (engine, _col, _dir) = fixture();
    let admin = QueryPrincipal {
        user: "root".into(),
        roles: vec!["admin".into()],
        can_read: true,
        can_write: true,
        can_admin: true,
        admin_of_database: false,
    };
    let reader = QueryPrincipal {
        user: "h3".into(),
        roles: vec!["viewer".into()],
        can_read: true,
        can_write: false,
        can_admin: false,
        admin_of_database: false,
    };
    let ask = |who: &QueryPrincipal, query: &str| {
        QueryExecutor::with_database(&engine, "app".to_string())
            .with_principal(who.clone())
            .execute(&parse(query).expect("query parses"))
            .map_err(|e| e.to_string())
    };
    let queries = [
        "FOR r IN c SORT r.v LIMIT 5 RETURN [r.host, r.v]",
        "FOR r IN c FILTER r.v >= 100 AND r.v < 160 SORT r.v RETURN r.v",
        "FOR r IN c FILTER r.ok == true COLLECT WITH COUNT INTO n RETURN n",
        "RETURN LENGTH(FOR r IN c FILTER r.v < 5000 RETURN 1)",
        "FOR r IN c COLLECT h = r.host AGGREGATE n = COUNT(), s = SUM(r.v), a = AVG(r.ms) RETURN [h, n, s, a]",
        "FOR r IN c COLLECT AGGREGATE s = SUM(r.v), hi = MAX(r.v), d = COUNT_DISTINCT(r.host) RETURN [s, hi, d]",
        "FOR r IN c FILTER r.host == 'h1' COLLECT WITH COUNT INTO n RETURN n",
        "FOR r IN c COLLECT AGGREGATE m = MEDIAN(r.v), p = PERCENTILE(r.v, 90) RETURN [m, p]",
        // SAMPLE reads the source: the policy applies to what it reads.
        "FOR r IN c SAMPLE 100 PERCENT FILTER r.v < 40 SORT r.v RETURN [r.host, r.v]",
    ];
    let both = |who: &QueryPrincipal| -> Vec<Vec<Value>> {
        queries
            .iter()
            .map(|q| {
                let columnar = ask(who, &q.replace(" IN c", " IN m")).unwrap();
                let documents = ask(who, &q.replace(" IN c", " IN d")).unwrap();
                assert_eq!(columnar, documents, "{}: {q}", who.user);
                columnar
            })
            .collect()
    };
    let open = both(&reader);

    for name in ["m", "_columnar_m"] {
        assert_eq!(
            ask(
                &admin,
                &format!("RETURN ROW_POLICY('{name}', 'doc.host == CURRENT_USER')")
            )
            .unwrap(),
            vec![json!("doc.host == CURRENT_USER")]
        );
        assert_eq!(
            ask(&admin, "RETURN ROW_POLICY('m')").unwrap(),
            vec![json!("doc.host == CURRENT_USER")]
        );
        ask(&admin, "RETURN ROW_POLICY('d', 'doc.host == CURRENT_USER')").unwrap();
        // The reader sees the rows of its host, whatever the query reads
        // of them; an admin sees every row.
        let bound = both(&reader);
        assert_ne!(bound, open);
        assert_eq!(bound[0][0], json!(["h3", 3]));
        assert_eq!(bound[6], vec![json!(0)]);
        assert_eq!(bound[5][0][2], json!(1));
        assert_eq!(both(&admin), {
            // (An admin is not bound: the answers before any policy.)
            let unbound: Vec<Vec<Value>> = queries
                .iter()
                .map(|q| ask(&admin, &q.replace(" IN c", " IN d")).unwrap())
                .collect();
            unbound
        });
        // Only an admin declares or lifts one.
        assert!(ask(&reader, "RETURN ROW_POLICY('m', 'true')").is_err());
        // Lifted, every row is back.
        assert_eq!(
            ask(&admin, &format!("RETURN ROW_POLICY('{name}', null)")).unwrap(),
            vec![Value::Null]
        );
        ask(&admin, "RETURN ROW_POLICY('d', null)").unwrap();
        assert_eq!(both(&reader), open);
    }
}

/// Card #953: the first rows of each partition (`ROW_NUMBER() OVER …` then
/// `FILTER n <= k`) are found in the column arrays, and are those a
/// document collection holding the same rows returns: same rows, same
/// numbers, same order, ties and nulls included.
#[test]
fn the_first_rows_of_each_partition_come_from_the_arrays() {
    let (engine, _col, _dir) = fixture();
    let found = || {
        solidb::storage::columnar::FIRST_ROWS_FROM_ARRAYS.load(std::sync::atomic::Ordering::Relaxed)
    };
    let queries = [
        // (query over `c`, whether the arrays answer it)
        ("FOR r IN c LET n = ROW_NUMBER() OVER (PARTITION BY r.host ORDER BY r.v DESC) FILTER n <= 3 RETURN [r.host, r.v, n]", true),
        ("FOR r IN c LET n = ROW_NUMBER() OVER (PARTITION BY r.host ORDER BY r.ms) FILTER n < 4 RETURN [r.host, r.ms, r.v, n]", true),
        // Ties (a boolean has two values): the earlier row first.
        ("FOR r IN c LET n = ROW_NUMBER() OVER (PARTITION BY r.host ORDER BY r.ok DESC) FILTER n <= 5 RETURN [r.host, r.v, n]", true),
        ("FOR r IN c LET n = ROW_NUMBER() OVER (PARTITION BY r.ok, r.host ORDER BY r.n DESC, r.v) FILTER n == 1 RETURN [r.ok, r.host, r.n, n]", true),
        // No partition; a partition that holds nulls; a string order.
        ("FOR r IN c LET n = ROW_NUMBER() OVER (ORDER BY r.ms DESC, r.v) FILTER n <= 7 RETURN [r.v, n]", true),
        ("FOR r IN c LET n = ROW_NUMBER() OVER (PARTITION BY r.n ORDER BY r.host DESC, r.v DESC) FILTER n <= 2 SORT r.v LIMIT 30 RETURN [r.n, r.host, r.v, n]", true),
        ("FOR r IN c LET n = ROW_NUMBER() OVER (PARTITION BY r.host ORDER BY r.v) FILTER n <= 2 RETURN r", true),
        ("FOR r IN c LET n = ROW_NUMBER() OVER (PARTITION BY r.host ORDER BY r.v DESC) FILTER n <= 2 COLLECT h = r.host AGGREGATE s = SUM(r.v) RETURN [h, s]", true),
        // Left to the rows: another function, an expression, a filter on
        // something else.
        ("FOR r IN c LET n = RANK() OVER (PARTITION BY r.host ORDER BY r.ok) FILTER n <= 1 COLLECT WITH COUNT INTO k RETURN k", false),
        ("FOR r IN c LET n = ROW_NUMBER() OVER (PARTITION BY r.v % 5 ORDER BY r.v) FILTER n <= 2 RETURN [r.v, n]", false),
        ("FOR r IN c LET n = ROW_NUMBER() OVER (PARTITION BY r.host ORDER BY r.v) FILTER n >= 2850 RETURN [r.v, n]", false),
    ];
    // The ordinary path over the same collection: every row built and
    // numbered.
    let ordinary = |query: &str| {
        QueryExecutor::with_database(&engine, "app".to_string())
            .with_streaming_collect(false)
            .execute(&parse(query).expect("query parses"))
            .expect("query runs")
    };
    let sorted = |mut rows: Vec<Value>| {
        rows.sort_by_key(|row| row.to_string());
        rows
    };
    for (query, from_arrays) in queries {
        let over_m = query.replace(" IN c ", " IN m ");
        let before = found();
        let columnar = run(&engine, &over_m, &[]);
        assert_eq!(found() > before, from_arrays, "{query}");
        assert!(!columnar.is_empty(), "{query}");
        // The same rows, numbers and order as the ordinary path.
        let before = found();
        assert_eq!(columnar, ordinary(&over_m), "{query}");
        assert_eq!(
            found(),
            before,
            "{query}: the ordinary path was not the one compared with"
        );
        // And, where no two rows of a partition are equal by the ORDER BY,
        // the rows a document collection returns (it reads in the order
        // of its keys, so ties fall otherwise and the rows come otherwise).
        let unique = query.contains("r.v)") || query.contains("r.v DESC)");
        if unique && !query.contains("RETURN r ") && !query.ends_with("RETURN r") {
            let documents = run(&engine, &query.replace(" IN c ", " IN d "), &[]);
            assert_eq!(sorted(columnar), sorted(documents), "{query}");
        }
    }
    // The bound as a bind variable.
    let bound = run(
        &engine,
        "FOR r IN m LET n = ROW_NUMBER() OVER (PARTITION BY r.host ORDER BY r.v DESC) FILTER n <= @k RETURN n",
        &[("k", json!(2))],
    );
    assert_eq!(bound.len(), 14);
}
