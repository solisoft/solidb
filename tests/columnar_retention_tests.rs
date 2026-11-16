//! Retention of a columnar collection (card #890): rows older than the
//! collection's retention are removed, whole blocks at a time, by the id
//! each row was given when it was inserted.

use serde_json::{json, Map, Value};
use solidb::storage::columnar::ColumnarIndexType;
use solidb::sync::{columnar, LogEntry, Operation, SyncLog};
use solidb::{
    ColumnDef, ColumnFilter, ColumnType, ColumnarCollection, CompressionType, StorageEngine,
};
use std::sync::Arc;
use tempfile::TempDir;

fn columns() -> Vec<ColumnDef> {
    let column = |name: &str, data_type| ColumnDef {
        name: name.to_string(),
        data_type,
        nullable: false,
        indexed: false,
        index_type: None,
    };
    vec![
        column("sensor", ColumnType::String),
        column("v", ColumnType::Int64),
    ]
}

/// An engine with a database `app` and a columnar collection `metrics`.
fn metrics() -> (StorageEngine, ColumnarCollection, TempDir) {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    let col = columnar::create(&db, None, "metrics", columns(), CompressionType::Lz4).unwrap();
    (engine, col, dir)
}

/// The id of a row inserted at `ms`: a UUIDv7 of that instant, `n` in its
/// last bits so that ids of one instant differ and stay in order.
fn id_at(ms: u64, n: u64) -> String {
    let value = ((ms as u128) << 80) | (0x7u128 << 76) | (0x8u128 << 60) | n as u128;
    uuid::Uuid::from_u128(value).to_string()
}

/// `count` rows, one a millisecond from `from_ms`, their `v` counting from
/// `first_v`.
fn rows(from_ms: u64, count: u64, first_v: i64) -> Vec<(String, Map<String, Value>)> {
    (0..count)
        .map(|i| {
            let row = json!({"sensor": format!("s{}", i % 4), "v": first_v + i as i64});
            (id_at(from_ms + i, i), row.as_object().unwrap().clone())
        })
        .collect()
}

/// The `v` of every row, in id order.
fn values(col: &ColumnarCollection) -> Vec<i64> {
    col.scan_filtered(&ColumnFilter::Gte("v".to_string(), json!(i64::MIN)), &["v"])
        .unwrap()
        .iter()
        .filter_map(|row| row["v"].as_i64())
        .collect()
}

const T0: u64 = 1_700_000_000_000;

#[test]
fn old_rows_go_by_whole_blocks_and_the_block_of_the_cutoff_keeps_its_rest() {
    let (_engine, col, _dir) = metrics();
    // Three inserts large enough to be sealed as a block each, an hour
    // apart, then a few rows that stay in the tail.
    let hour = 3_600_000;
    col.insert_rows_with_ids(rows(T0, 5_000, 0)).unwrap();
    col.insert_rows_with_ids(rows(T0 + hour, 5_000, 5_000))
        .unwrap();
    col.insert_rows_with_ids(rows(T0 + 2 * hour, 5_000, 10_000))
        .unwrap();
    col.insert_rows_with_ids(rows(T0 + 3 * hour, 10, 15_000))
        .unwrap();
    assert_eq!(col.count(), 15_010);

    // Nothing is older than the first row.
    assert_eq!(col.prune_before(T0).unwrap(), 0);
    assert_eq!(col.count(), 15_010);

    // The cutoff falls in the second block: the first goes whole, the
    // second keeps the rows from the cutoff on.
    let removed = col.prune_before(T0 + hour + 1_200).unwrap();
    assert_eq!(removed, 5_000 + 1_200);
    assert_eq!(col.count(), 15_010 - 6_200);
    let left = values(&col);
    assert_eq!(left.len(), 8_810);
    assert_eq!(left.first(), Some(&6_200));
    assert_eq!(left.last(), Some(&15_009));
    assert!(
        left.windows(2).all(|pair| pair[0] + 1 == pair[1]),
        "a hole in what is left"
    );

    // The same again changes nothing, and what is stored is what is kept.
    assert_eq!(col.prune_before(T0 + hour + 1_200).unwrap(), 0);
}

#[test]
fn what_is_left_is_read_the_same_after_a_reload() {
    let (engine, col, _dir) = metrics();
    col.insert_rows_with_ids(rows(T0, 6_000, 0)).unwrap();
    col.insert_rows_with_ids(rows(T0 + 10_000, 20, 6_000))
        .unwrap();
    // Past every sealed row and half of the tail.
    assert_eq!(col.prune_before(T0 + 10_010).unwrap(), 6_010);
    let kept = values(&col);
    assert_eq!(kept, (6_010..6_020).collect::<Vec<i64>>());

    let db = engine.get_database("app").unwrap();
    let again = ColumnarCollection::load("metrics".to_string(), "app", db.db_arc()).unwrap();
    assert_eq!(again.count(), 10);
    assert_eq!(values(&again), kept);
    // New rows go in after a prune as before it.
    again
        .insert_rows(vec![json!({"sensor": "s9", "v": 99})])
        .unwrap();
    assert_eq!(values(&again).last(), Some(&99));
}

#[test]
fn a_retention_removes_what_is_older_and_nothing_without_one() {
    let (_engine, col, _dir) = metrics();
    let now = chrono::Utc::now().timestamp_millis() as u64;
    // Two hours old, ten minutes old, and rows inserted now.
    col.insert_rows_with_ids(rows(now - 7_200_000, 4_500, 0))
        .unwrap();
    col.insert_rows_with_ids(rows(now - 600_000, 100, 10_000))
        .unwrap();
    col.insert_rows(vec![
        json!({"sensor": "s1", "v": 20_000}),
        json!({"sensor": "s2", "v": 20_001}),
    ])
    .unwrap();

    // No retention: nothing expires.
    assert_eq!(col.metadata().unwrap().retention_secs, None);
    assert_eq!(col.prune_expired().unwrap(), 0);
    assert!(col.set_retention(Some(0)).is_err());

    // An hour: the two-hour-old rows go.
    col.set_retention(Some(3_600)).unwrap();
    assert_eq!(col.metadata().unwrap().retention_secs, Some(3_600));
    assert_eq!(col.prune_expired().unwrap(), 4_500);
    assert_eq!(col.count(), 102);
    // A minute: the ten-minute-old rows too; the rows of now stay.
    col.set_retention(Some(60)).unwrap();
    assert_eq!(col.prune_expired().unwrap(), 100);
    assert_eq!(values(&col), [20_000, 20_001]);
    // And back to keeping everything.
    col.set_retention(None).unwrap();
    assert_eq!(col.prune_expired().unwrap(), 0);
}

#[test]
fn a_filter_answers_right_after_a_prune_moved_the_rows_under_an_index() {
    let (_engine, col, _dir) = metrics();
    col.insert_rows_with_ids(rows(T0, 5_000, 0)).unwrap();
    col.insert_rows_with_ids(rows(T0 + 60_000, 5_000, 5_000))
        .unwrap();
    col.create_index("sensor", ColumnarIndexType::Hash).unwrap();
    let filter = ColumnFilter::Eq("sensor".to_string(), json!("s1"));
    let before = col.scan_filtered(&filter, &["v"]).unwrap().len();
    assert_eq!(before, 2_500);

    // 5,000 rows of the first block and 100 of the second go; a quarter of
    // them were of that sensor.
    assert_eq!(col.prune_before(T0 + 60_100).unwrap(), 5_100);
    let after: Vec<i64> = col
        .scan_filtered(&filter, &["v"])
        .unwrap()
        .iter()
        .filter_map(|row| row["v"].as_i64())
        .collect();
    assert_eq!(after.len(), 2_500 - 1_275);
    assert!(
        after.iter().all(|v| *v >= 5_100 && v % 4 == 1),
        "{:?}",
        &after[..4]
    );
}

#[test]
fn a_retention_reaches_a_peer_which_removes_its_own_rows() {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    let log = Arc::new(
        SyncLog::new(
            "n1".to_string(),
            dir.path().join("log").to_str().unwrap(),
            64,
        )
        .unwrap(),
    );
    columnar::create(&db, Some(&log), "metrics", columns(), CompressionType::Lz4).unwrap();
    columnar::set_retention(&db, Some(&log), "metrics", Some(3_600)).unwrap();

    let entries: Vec<LogEntry> = log
        .get_entries_after(0, 1_000)
        .into_iter()
        .filter(|e| e.collection == "metrics")
        .collect();
    let kinds: Vec<Operation> = entries.iter().map(|e| e.operation).collect();
    assert_eq!(
        kinds,
        [
            Operation::ColumnarCreateCollection,
            Operation::ColumnarSetRetention
        ]
    );

    // The peer applies both, then expires its own old rows.
    let (peer, _unused, _pdir) = {
        let pdir = TempDir::new().unwrap();
        let peer = StorageEngine::new(pdir.path().join("data").to_str().unwrap()).unwrap();
        peer.initialize().unwrap();
        peer.create_database("app".to_string()).unwrap();
        (peer, (), pdir)
    };
    let pdb = peer.get_database("app").unwrap();
    columnar::apply_create(&pdb, "metrics", entries[0].data.as_ref().unwrap()).unwrap();
    columnar::apply_set_retention(&pdb, "metrics", entries[1].data.as_ref().unwrap()).unwrap();
    let on_peer = ColumnarCollection::load("metrics".to_string(), "app", pdb.db_arc()).unwrap();
    assert_eq!(on_peer.metadata().unwrap().retention_secs, Some(3_600));
    let now = chrono::Utc::now().timestamp_millis() as u64;
    on_peer
        .insert_rows_with_ids(rows(now - 7_200_000, 10, 0))
        .unwrap();
    on_peer
        .insert_rows_with_ids(rows(now - 1_000, 5, 100))
        .unwrap();
    assert_eq!(columnar::prune(&pdb, "metrics").unwrap(), 10);
    // Read through a handle of its own, as each request does.
    let reloaded = ColumnarCollection::load("metrics".to_string(), "app", pdb.db_arc()).unwrap();
    assert_eq!(reloaded.count(), 5);
}
