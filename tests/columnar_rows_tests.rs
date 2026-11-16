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
    ];
    let col = columnar::create(&db, None, "m", columns, CompressionType::Lz4).unwrap();
    let rows: Vec<Value> = (0..ROWS)
        .map(|i| {
            let ms = if i % 4 == 0 {
                Value::Null
            } else {
                json!(i as f64 / 8.0)
            };
            json!({"host": format!("h{}", i % 7), "v": i, "ms": ms, "ok": i % 3 == 0})
        })
        .collect();
    col.insert_rows(rows[..19_990].to_vec()).unwrap();
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
        // Limits, with and without a FILTER, and a sort that picks the end.
        ("FOR r IN $c SORT r.v DESC LIMIT 3 RETURN r.v", vec![]),
        ("FOR r IN $c FILTER r.host == 'h1' SORT r.v LIMIT 2, 3 RETURN r.v", vec![]),
        // A FILTER that names an outer row is not the collection's to answer.
        ("FOR x IN [3, 9000, 19995] FOR r IN $c FILTER r.v == x RETURN [x, r.host]", vec![]),
        ("FOR x IN ['h0', 'h6'] FOR r IN $c FILTER r.host == x AND r.v < 14 SORT r.v RETURN [x, r.v]", vec![]),
        // Counted, not returned.
        ("FOR r IN $c FILTER r.host == 'h2' AND r.v >= 10000 COLLECT WITH COUNT INTO n RETURN n", vec![]),
        ("RETURN LENGTH(FOR r IN $c FILTER r.v < 100 RETURN 1)", vec![]),
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
    col.create_index("v", ColumnarIndexType::MinMax).unwrap();
    compare(&engine, "bitmap index on host, minmax index on v");

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
        [json!({"host": "h0", "v": 7, "ms": 0.875, "ok": false})]
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
