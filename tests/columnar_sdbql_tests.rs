//! Columnar collections as a first-class SDBQL data source.
//!
//! Columnar data is stored under `COL_META_`/`COL_DATA_`/`COL_ROW_` prefixes,
//! not the `doc:` prefix the document scanner walks. Before this, a columnar
//! collection was reachable from SDBQL through exactly one hard-coded shape —
//! `FOR x IN c COLLECT AGGREGATE ... RETURN ...` — and every other query
//! reported `CollectionNotFound`. The same collection both existed and did
//! not, depending on the shape of the query.
//!
//! The aggregate fast path had its own problems, all silent: it ignored the
//! RETURN clause entirely, dropped every aggregate after the first when
//! grouping, reported the group column under storage's internal `_agg` name,
//! and double-encoded string group keys as `"\"a\""`.

use rust_rocksdb::Options;
use serde_json::{json, Value};
use solidb::sdbql::{parse, QueryExecutor};
use solidb::storage::columnar::{ColumnDef, ColumnType, ColumnarCollection, CompressionType};
use solidb::storage::engine::StorageEngine;
use tempfile::TempDir;

/// Build a storage engine with a columnar `metrics` collection holding three
/// rows across two hosts.
fn fixture() -> (StorageEngine, TempDir) {
    let dir = TempDir::new().unwrap();
    let storage = StorageEngine::new(dir.path()).unwrap();
    storage.create_database("testdb".to_string()).unwrap();
    let database = storage.get_database("testdb").unwrap();
    let db_arc = database.db_arc();

    let cf_name = "testdb:_columnar_metrics";
    if db_arc.cf_handle(cf_name).is_none() {
        db_arc.create_cf(cf_name, &Options::default()).unwrap();
    }

    let columns = vec![
        ColumnDef {
            name: "host".to_string(),
            data_type: ColumnType::String,
            nullable: false,
            indexed: false,
            index_type: None,
        },
        ColumnDef {
            name: "value".to_string(),
            data_type: ColumnType::Float64,
            nullable: false,
            indexed: false,
            index_type: None,
        },
    ];

    let columnar = ColumnarCollection::new(
        "metrics".to_string(),
        "testdb",
        db_arc,
        columns,
        CompressionType::Lz4,
    )
    .unwrap();

    columnar
        .insert_rows(vec![
            json!({"host": "a", "value": 10.0}),
            json!({"host": "b", "value": 20.0}),
            json!({"host": "a", "value": 30.0}),
        ])
        .unwrap();

    (storage, dir)
}

fn run(storage: &StorageEngine, query: &str) -> Vec<Value> {
    let parsed = parse(query).expect("query parses");
    let executor = QueryExecutor::with_database(storage, "testdb".to_string());
    executor.execute(&parsed).expect("query executes")
}

// ===========================================================================
// Columnar as a data source — every shape, not just the aggregate one
// ===========================================================================

/// The simplest query of all. This took a different fast path in the executor
/// than filtered queries, so it needed its own fix.
#[test]
fn plain_scan_returns_rows() {
    let (storage, _d) = fixture();
    let rows = run(&storage, "FOR m IN metrics RETURN m");
    assert_eq!(rows.len(), 3, "expected all three rows, got {rows:?}");
    assert!(rows.iter().any(|r| r["host"] == "a" && r["value"] == 10.0));
}

#[test]
fn filter_works() {
    let (storage, _d) = fixture();
    let rows = run(
        &storage,
        r#"FOR m IN metrics FILTER m.host == "a" RETURN m.value"#,
    );
    assert_eq!(rows, vec![json!(10.0), json!(30.0)]);
}

#[test]
fn sort_and_limit_work() {
    let (storage, _d) = fixture();
    let rows = run(
        &storage,
        "FOR m IN metrics SORT m.value DESC LIMIT 2 RETURN m.value",
    );
    assert_eq!(rows, vec![json!(30.0), json!(20.0)]);
}

#[test]
fn limit_with_offset_works() {
    let (storage, _d) = fixture();
    let rows = run(&storage, "FOR m IN metrics LIMIT 1,2 RETURN m.value");
    assert_eq!(rows.len(), 2, "offset+count should skip the first row");
}

/// A standalone OFFSET has no count; the scan must stay bounded by the data,
/// not by a sentinel maximum.
#[test]
fn offset_without_limit_works() {
    let (storage, _d) = fixture();
    let rows = run(&storage, "FOR m IN metrics OFFSET 1 RETURN m");
    assert_eq!(rows.len(), 2);

    let rows = run(&storage, "FOR m IN metrics OFFSET 1 RETURN m.value");
    assert_eq!(rows.len(), 2);
}

/// The columnar scan is its own fast path in the executor and used to return
/// before DISTINCT was applied.
#[test]
fn return_distinct_applies_to_columnar_rows() {
    let (storage, _d) = fixture();
    let rows = run(&storage, "FOR m IN metrics RETURN DISTINCT m.host");
    assert_eq!(rows.len(), 2, "hosts a and b, once each: {rows:?}");
}

/// The point of making columnar a real data source: it composes with the rest
/// of the language instead of living behind a separate API.
#[test]
fn columnar_joins_documents() {
    let (storage, _d) = fixture();
    let database = storage.get_database("testdb").unwrap();
    database
        .create_collection("hosts".to_string(), None)
        .unwrap();
    let hosts = database.get_collection("hosts").unwrap();
    hosts
        .insert(json!({"_key": "a", "region": "eu-a"}))
        .unwrap();
    hosts
        .insert(json!({"_key": "b", "region": "eu-b"}))
        .unwrap();

    let rows = run(
        &storage,
        "FOR m IN metrics FOR h IN hosts FILTER h._key == m.host \
         RETURN {v: m.value, region: h.region}",
    );
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().any(|r| r["v"] == 30.0 && r["region"] == "eu-a"));
}

// ===========================================================================
// Aggregate fast path — must honour RETURN
// ===========================================================================

/// `RETURN {sum: total}` used to come back as `{"total": ...}`: the fast path
/// built its own object and never evaluated the RETURN expression.
#[test]
fn aggregate_honours_a_renamed_return_key() {
    let (storage, _d) = fixture();
    let rows = run(
        &storage,
        "FOR m IN metrics COLLECT AGGREGATE total = SUM(m.value) RETURN {sum: total}",
    );
    assert_eq!(rows, vec![json!({"sum": 60.0})]);
}

/// A scalar RETURN used to come back wrapped in an object.
#[test]
fn aggregate_honours_a_scalar_return() {
    let (storage, _d) = fixture();
    let rows = run(
        &storage,
        "FOR m IN metrics COLLECT AGGREGATE total = SUM(m.value) RETURN total",
    );
    assert_eq!(rows, vec![json!(60.0)]);
}

/// Grouped queries returned raw storage rows and stopped after the first
/// aggregate, so `hi` vanished.
#[test]
fn grouped_query_keeps_every_aggregate() {
    let (storage, _d) = fixture();
    let mut rows = run(
        &storage,
        "FOR m IN metrics COLLECT host = m.host \
         AGGREGATE lo = MIN(m.value), hi = MAX(m.value) RETURN {host, lo, hi}",
    );
    rows.sort_by_key(|r| r["host"].as_str().unwrap_or("").to_string());

    assert_eq!(
        rows,
        vec![
            json!({"host": "a", "lo": 10.0, "hi": 30.0}),
            json!({"host": "b", "lo": 20.0, "hi": 20.0}),
        ]
    );
}

/// String group keys were JSON-encoded into the key and re-wrapped, so `a`
/// came back as `"\"a\""`.
#[test]
fn grouped_string_keys_are_not_double_encoded() {
    let (storage, _d) = fixture();
    let rows = run(
        &storage,
        "FOR m IN metrics COLLECT host = m.host AGGREGATE total = SUM(m.value) RETURN host",
    );
    let mut hosts: Vec<&str> = rows.iter().filter_map(|r| r.as_str()).collect();
    hosts.sort_unstable();
    assert_eq!(hosts, vec!["a", "b"], "got {rows:?}");
}

/// The group column is bound to the COLLECT variable, which need not match the
/// underlying column name.
#[test]
fn grouped_result_uses_the_collect_variable_name() {
    let (storage, _d) = fixture();
    let mut rows = run(
        &storage,
        "FOR m IN metrics COLLECT h = m.host AGGREGATE total = SUM(m.value) RETURN {h, total}",
    );
    rows.sort_by_key(|r| r["h"].as_str().unwrap_or("").to_string());

    assert_eq!(
        rows,
        vec![
            json!({"h": "a", "total": 40.0}),
            json!({"h": "b", "total": 20.0}),
        ]
    );
}
