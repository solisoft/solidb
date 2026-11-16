//! Compaction of a columnar collection (card #898): runs of small blocks
//! are merged into blocks as large as a block may be, and nothing a query
//! answers changes.

use serde_json::{json, Value};
use solidb::sdbql::{parse, QueryExecutor};
use solidb::storage::columnar::ColumnarIndexType;
use solidb::sync::columnar;
use solidb::{
    ColumnDef, ColumnFilter, ColumnType, ColumnarCollection, CompressionType, StorageEngine,
};
use tempfile::TempDir;

fn metrics() -> (StorageEngine, ColumnarCollection, TempDir) {
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
    ];
    let col = columnar::create(&db, None, "metrics", columns, CompressionType::Lz4).unwrap();
    (engine, col, dir)
}

/// `count` rows whose `v` counts from `from`.
fn rows(from: i64, count: i64) -> Vec<Value> {
    (from..from + count)
        .map(|i| {
            let ms = if i % 3 == 0 {
                Value::Null
            } else {
                json!(i as f64 / 4.0)
            };
            json!({"host": format!("h{}", i % 6), "v": i, "ms": ms})
        })
        .collect()
}

fn run(engine: &StorageEngine, query: &str) -> Vec<Value> {
    QueryExecutor::with_database(engine, "app".to_string())
        .execute(&parse(query).expect("query parses"))
        .unwrap_or_else(|e| panic!("{query}: {e}"))
}

/// What a set of queries answers: compared before and after a compaction.
fn answers(engine: &StorageEngine, col: &ColumnarCollection) -> Vec<Value> {
    let mut all = Vec::new();
    for query in [
        "FOR r IN metrics COLLECT AGGREGATE n = COUNT(r.v), s = SUM(r.v), m = SUM(r.ms) RETURN [n, s, m]",
        "FOR r IN metrics COLLECT host = r.host AGGREGATE n = COUNT(r.v), top = MAX(r.v) SORT host RETURN [host, n, top]",
        "FOR r IN metrics FILTER r.v >= 4990 AND r.v < 5010 SORT r.v RETURN r",
        "FOR r IN metrics FILTER r.host == 'h2' AND r.v > 60000 COLLECT WITH COUNT INTO n RETURN n",
    ] {
        all.push(Value::Array(run(engine, query)));
    }
    let by_host = col
        .scan_filtered(&ColumnFilter::Eq("host".to_string(), json!("h4")), &["v"])
        .unwrap();
    all.push(json!(by_host.len()));
    all.push(json!(col.list_row_uuids().unwrap()));
    all
}

#[test]
fn small_blocks_become_one_and_the_answers_stay() {
    let (engine, col, _dir) = metrics();
    // Thirteen inserts of 5,000 rows, each sealed as its own block, and a
    // few rows that stay in the tail.
    for batch in 0..13 {
        col.insert_rows(rows(batch * 5_000, 5_000)).unwrap();
    }
    col.insert_rows(rows(65_000, 7)).unwrap();
    col.create_index("host", ColumnarIndexType::Hash).unwrap();
    let before = answers(&engine, &col);

    assert_eq!(col.compact().unwrap(), (13, 1));
    assert_eq!(answers(&engine, &col), before);
    // Nothing more to merge.
    assert_eq!(col.compact().unwrap(), (1, 1));

    // The same after a reload, and rows go in as before.
    let db = engine.get_database("app").unwrap();
    let again = ColumnarCollection::load("metrics".to_string(), "app", db.db_arc()).unwrap();
    assert_eq!(answers(&engine, &again), before);
    again.insert_rows(rows(70_000, 5_000)).unwrap();
    assert_eq!(again.count(), 70_007);
    assert_eq!(
        run(
            &engine,
            "FOR r IN metrics COLLECT AGGREGATE top = MAX(r.v) RETURN top"
        ),
        [json!(74_999)]
    );
}

#[test]
fn blocks_that_do_not_fit_together_are_left() {
    let (engine, col, _dir) = metrics();
    // 40,000 + 40,000 do not fit in one block; the two of 5,000 after do.
    col.insert_rows(rows(0, 40_000)).unwrap();
    col.insert_rows(rows(40_000, 40_000)).unwrap();
    col.insert_rows(rows(80_000, 5_000)).unwrap();
    col.insert_rows(rows(85_000, 5_000)).unwrap();
    let before = answers(&engine, &col);
    assert_eq!(col.compact().unwrap(), (4, 2));
    assert_eq!(answers(&engine, &col), before);
}

#[test]
fn blocks_that_lost_rows_are_merged_too() {
    let (engine, col, _dir) = metrics();
    col.insert_rows(rows(0, 50_000)).unwrap();
    col.insert_rows(rows(50_000, 50_000)).unwrap();
    // Each block keeps a fifth of its rows.
    let deleted = col
        .delete_where(&ColumnFilter::Ne("host".to_string(), json!("h0")))
        .unwrap();
    assert!(deleted.len() > 80_000, "{}", deleted.len());
    let before = answers(&engine, &col);
    assert_eq!(col.compact().unwrap(), (2, 1));
    assert_eq!(answers(&engine, &col), before);
    assert_eq!(col.count(), 100_000 - deleted.len());
}
