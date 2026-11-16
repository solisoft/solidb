//! Schema changes of a columnar collection (card #888): a column added,
//! dropped or renamed after the collection holds rows, in sealed blocks
//! and in the tail.

use serde_json::{json, Value};
use solidb::error::DbError;
use solidb::sdbql::{parse, QueryExecutor};
use solidb::storage::columnar::ColumnarIndexType;
use solidb::sync::columnar;
use solidb::{ColumnDef, ColumnType, ColumnarCollection, CompressionType, StorageEngine};
use tempfile::TempDir;

fn column(name: &str, data_type: ColumnType, nullable: bool) -> ColumnDef {
    ColumnDef {
        name: name.to_string(),
        data_type,
        nullable,
        indexed: false,
        index_type: None,
    }
}

/// A collection `metrics` (host, v) holding 5,000 rows in a sealed block
/// and 10 in the tail; `v` counts from 0.
fn metrics() -> (StorageEngine, ColumnarCollection, TempDir) {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    let columns = vec![
        column("host", ColumnType::String, false),
        column("v", ColumnType::Int64, false),
    ];
    let col = columnar::create(&db, None, "metrics", columns, CompressionType::Lz4).unwrap();
    let row = |i: i64| json!({"host": format!("h{}", i % 5), "v": i});
    col.insert_rows((0..5_000).map(row).collect()).unwrap();
    col.insert_rows((5_000..5_010).map(row).collect()).unwrap();
    (engine, col, dir)
}

fn run(engine: &StorageEngine, query: &str) -> Vec<Value> {
    QueryExecutor::with_database(engine, "app".to_string())
        .execute(&parse(query).expect("query parses"))
        .unwrap_or_else(|e| panic!("{query}: {e}"))
}

fn reload(engine: &StorageEngine) -> ColumnarCollection {
    let db = engine.get_database("app").unwrap();
    ColumnarCollection::load("metrics".to_string(), "app", db.db_arc()).unwrap()
}

#[test]
fn a_column_added_is_null_in_the_rows_that_were_there() {
    let (engine, col, _dir) = metrics();
    col.add_column(column("region", ColumnType::String, true))
        .unwrap();
    col.add_column(column("ms", ColumnType::Float64, true))
        .unwrap();

    // Old rows, in the block and in the tail, hold null.
    let old = run(
        &engine,
        "FOR r IN metrics FILTER r.v == 12 OR r.v == 5005 SORT r.v RETURN [r.v, r.region, r.ms]",
    );
    assert_eq!(old, [json!([12, null, null]), json!([5005, null, null])]);

    // New rows take the columns, and old and new are read together.
    col.insert_rows(vec![
        json!({"host": "h9", "v": 9_000, "region": "eu", "ms": 1.5}),
        json!({"host": "h9", "v": 9_001, "region": "us", "ms": 2.5}),
        json!({"host": "h9", "v": 9_002}),
    ])
    .unwrap();
    let per_region = run(
        &engine,
        "FOR r IN metrics COLLECT region = r.region AGGREGATE n = COUNT(r.v), ms = SUM(r.ms) SORT region RETURN [region, n, ms]",
    );
    assert_eq!(
        per_region,
        [
            json!([null, 5_011, 0]),
            json!(["eu", 1, 1.5]),
            json!(["us", 1, 2.5])
        ],
        "{per_region:?}"
    );
    let eu = run(
        &engine,
        "FOR r IN metrics FILTER r.region == 'eu' RETURN r.v",
    );
    assert_eq!(eu, [json!(9_000)]);

    // What a new column cannot be.
    let required = col.add_column(column("zone", ColumnType::String, false));
    assert!(
        matches!(required, Err(DbError::BadRequest(_))),
        "{:?}",
        required
    );
    let again = col.add_column(column("region", ColumnType::String, true));
    assert!(
        matches!(again, Err(DbError::ConflictError(_))),
        "{:?}",
        again
    );

    // The schema is the stored one.
    let names: Vec<String> = reload(&engine)
        .metadata()
        .unwrap()
        .columns
        .iter()
        .map(|c| c.name.clone())
        .collect();
    assert_eq!(names, ["host", "v", "region", "ms"]);
}

#[test]
fn a_column_dropped_takes_its_values_with_it() {
    let (engine, col, _dir) = metrics();
    col.drop_column("host").unwrap();

    let rows = run(
        &engine,
        "FOR r IN metrics FILTER r.v == 7 OR r.v == 5007 SORT r.v RETURN r",
    );
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(row.get("host"), None, "{row}");
    }
    assert_eq!(
        run(
            &engine,
            "FOR r IN metrics COLLECT AGGREGATE s = SUM(r.v) RETURN s"
        ),
        [json!(12_547_545)]
    );

    // A row may no longer hold it.
    let refused = col.insert_rows(vec![json!({"host": "h1", "v": 1})]);
    assert!(
        matches!(refused, Err(DbError::BadRequest(_))),
        "{:?}",
        refused
    );

    // Added again under the same name, it does not find the old values:
    // not in the block, not in the tail.
    col.add_column(column("host", ColumnType::String, true))
        .unwrap();
    let back = run(
        &engine,
        "FOR r IN metrics FILTER r.v == 7 OR r.v == 5007 SORT r.v RETURN r.host",
    );
    assert_eq!(back, [Value::Null, Value::Null]);
    let hosts = run(
        &engine,
        "FOR r IN metrics FILTER r.host != null COLLECT WITH COUNT INTO n RETURN n",
    );
    assert_eq!(hosts, [json!(0)]);

    // The last column stays, and so does one that is not there.
    col.drop_column("host").unwrap();
    assert!(matches!(col.drop_column("v"), Err(DbError::BadRequest(_))));
    assert!(matches!(
        col.drop_column("nope"),
        Err(DbError::BadRequest(_))
    ));
}

#[test]
fn a_column_renamed_keeps_its_values() {
    let (engine, col, _dir) = metrics();
    col.rename_column("v", "value").unwrap();

    assert_eq!(
        run(
            &engine,
            "FOR r IN metrics COLLECT AGGREGATE n = COUNT(r.value), s = SUM(r.value) RETURN [n, s]"
        ),
        [json!([5_010, 12_547_545])]
    );
    let rows = run(
        &engine,
        "FOR r IN metrics FILTER r.value == 3 OR r.value == 5003 SORT r.value RETURN r",
    );
    assert_eq!(
        rows,
        [
            json!({"host": "h3", "value": 3}),
            json!({"host": "h3", "value": 5003})
        ],
        "{rows:?}"
    );
    // New rows are written under the new name, and read with the old ones.
    col.insert_rows(vec![json!({"host": "h0", "value": 100_000})])
        .unwrap();
    assert_eq!(
        run(
            &engine,
            "FOR r IN metrics COLLECT AGGREGATE m = MAX(r.value) RETURN m"
        ),
        [json!(100_000)]
    );
    let old_name = col.insert_rows(vec![json!({"host": "h0", "v": 1})]);
    assert!(
        matches!(old_name, Err(DbError::BadRequest(_))),
        "{:?}",
        old_name
    );

    // A name that is taken, and after a reload the same answers.
    assert!(matches!(
        col.rename_column("value", "host"),
        Err(DbError::ConflictError(_))
    ));
    assert!(matches!(
        col.rename_column("nope", "x"),
        Err(DbError::BadRequest(_))
    ));
    let again = reload(&engine);
    assert_eq!(again.count(), 5_011);
    assert_eq!(
        run(
            &engine,
            "FOR r IN metrics FILTER r.host == 'h4' COLLECT AGGREGATE s = SUM(r.value) RETURN s"
        ),
        [json!(2_511_513)]
    );
}

#[test]
fn a_column_with_an_index_is_left_alone() {
    let (_engine, col, _dir) = metrics();
    col.create_index("host", ColumnarIndexType::Hash).unwrap();
    assert!(matches!(
        col.drop_column("host"),
        Err(DbError::ConflictError(_))
    ));
    assert!(matches!(
        col.rename_column("host", "node"),
        Err(DbError::ConflictError(_))
    ));
    // Another column changes under it, and the index still answers.
    col.rename_column("v", "value").unwrap();
    col.add_column(column("ms", ColumnType::Float64, true))
        .unwrap();
    let rows = col
        .scan_filtered(
            &solidb::ColumnFilter::Eq("host".to_string(), json!("h2")),
            &["value"],
        )
        .unwrap();
    assert_eq!(rows.len(), 1_002);
    col.drop_index("host").unwrap();
    col.drop_column("host").unwrap();
}

/// Each change is logged for the peers, which apply it once: a log is
/// replayed from its start by a node that joins.
#[test]
fn column_changes_reach_a_peer_and_replay_without_harm() {
    use solidb::sync::{LogEntry, Operation, SyncLog};
    use std::sync::Arc;

    let node = |logging: bool| {
        let dir = TempDir::new().unwrap();
        let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
        engine.initialize().unwrap();
        engine.create_database("app".to_string()).unwrap();
        let log = logging.then(|| {
            let path = dir.path().join("log");
            Arc::new(SyncLog::new("n1".to_string(), path.to_str().unwrap(), 64).unwrap())
        });
        (engine, log, dir)
    };
    let (a, log, _da) = node(true);
    let log = log.unwrap();
    let (b, _, _db) = node(false);
    let db = a.get_database("app").unwrap();
    let columns = vec![
        column("host", ColumnType::String, false),
        column("v", ColumnType::Int64, false),
        column("old", ColumnType::Int64, true),
    ];
    let col = columnar::create(&db, Some(&log), "metrics", columns, CompressionType::Lz4).unwrap();
    columnar::insert(
        &db,
        Some(&log),
        "metrics",
        vec![json!({"host": "h1", "v": 1, "old": 7})],
    )
    .unwrap();
    columnar::add_column(
        &db,
        Some(&log),
        "metrics",
        column("ms", ColumnType::Float64, true),
    )
    .unwrap();
    columnar::rename_column(&db, Some(&log), "metrics", "v", "value").unwrap();
    columnar::drop_column(&db, Some(&log), "metrics", "old").unwrap();
    columnar::insert(
        &db,
        Some(&log),
        "metrics",
        vec![json!({"host": "h2", "value": 2, "ms": 0.5})],
    )
    .unwrap();
    drop(col);

    let entries: Vec<LogEntry> = log
        .get_entries_after(0, 1_000)
        .into_iter()
        .filter(|e| e.collection == "metrics")
        .collect();
    let apply = |peer: &StorageEngine| {
        let pdb = peer.get_database("app").unwrap();
        for e in &entries {
            match e.operation {
                Operation::ColumnarCreateCollection => {
                    columnar::apply_create(&pdb, "metrics", e.data.as_ref().unwrap()).unwrap()
                }
                Operation::ColumnarInsert => {
                    columnar::apply_insert(&pdb, "metrics", &e.key, e.data.as_ref().unwrap())
                        .unwrap()
                }
                Operation::ColumnarAddColumn
                | Operation::ColumnarDropColumn
                | Operation::ColumnarRenameColumn => columnar::apply_column_change(
                    &pdb,
                    "metrics",
                    e.operation,
                    &e.key,
                    e.data.as_deref(),
                )
                .unwrap(),
                other => panic!("unexpected {other:?}"),
            }
        }
    };
    let rows = |engine: &StorageEngine| run(engine, "FOR r IN metrics SORT r.value RETURN r");
    let expected = [
        json!({"host": "h1", "value": 1, "ms": null}),
        json!({"host": "h2", "value": 2, "ms": 0.5}),
    ];
    assert_eq!(rows(&a), expected);
    apply(&b);
    assert_eq!(rows(&b), expected, "the peer after one pass");
    apply(&b);
    assert_eq!(rows(&b), expected, "the peer after the log was replayed");
}
