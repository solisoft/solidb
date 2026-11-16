//! The decoded blocks of columnar collections are held to a budget
//! (`SOLIDB_COLUMNAR_BLOCKS_MB`): past it, the collections read longest
//! ago give theirs back, and read them again at their next use — with the
//! same rows, whatever was written meanwhile.
//!
//! One test, in a process of its own: the budget is read once.

use serde_json::{json, Value};
use solidb::storage::columnar::{
    decoded_blocks_kept, ColumnDef, ColumnType, ColumnarCollection, CompressionType,
};
use solidb::storage::StorageEngine;
use tempfile::TempDir;

const ROWS: i64 = 12_000;

fn create(engine: &StorageEngine, name: &str) -> ColumnarCollection {
    let db = engine.get_database("_system").unwrap();
    db.create_collection(format!("_columnar_{name}"), None)
        .unwrap();
    let columns = [
        ("id", ColumnType::Int64),
        ("name", ColumnType::String),
        ("score", ColumnType::Float64),
    ]
    .into_iter()
    .map(|(name, data_type)| ColumnDef {
        name: name.to_string(),
        data_type,
        nullable: true,
        indexed: false,
        index_type: None,
    })
    .collect();
    ColumnarCollection::new(
        name.to_string(),
        "_system",
        db.db_arc(),
        columns,
        CompressionType::Lz4,
    )
    .unwrap()
}

fn row(of: &str, i: i64) -> Value {
    json!({"id": i, "name": format!("{of}-{:03}", i % 211), "score": i as f64 / 4.0})
}

fn rows(of: &str, from: i64, to: i64) -> Vec<Value> {
    (from..to).map(|i| row(of, i)).collect()
}

fn read(col: &ColumnarCollection) -> Vec<Value> {
    col.read_columns(&["id", "name", "score"], None).unwrap()
}

#[test]
fn decoded_blocks_are_held_to_the_budget() {
    // Read before anything asks for it.
    std::env::set_var("SOLIDB_COLUMNAR_BLOCKS_MB", "1");
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path()).unwrap();
    engine.create_database("_system".to_string()).unwrap();

    // Four collections of about half a megabyte decoded each: two fit.
    let names = ["a", "b", "c", "d"];
    let cols: Vec<ColumnarCollection> = names.iter().map(|n| create(&engine, n)).collect();
    for (name, col) in names.iter().zip(&cols) {
        col.insert_rows(rows(name, 0, ROWS)).unwrap();
        assert_eq!(read(col), rows(name, 0, ROWS), "{name}");
        assert!(
            decoded_blocks_kept() <= 1 << 20,
            "{} bytes kept",
            decoded_blocks_kept()
        );
    }
    let one = decoded_blocks_kept();
    assert!(one > 0, "the collection read last keeps its blocks");

    // Each is read again as it was, whichever gave its blocks back.
    for (name, col) in names.iter().zip(&cols) {
        assert_eq!(read(col), rows(name, 0, ROWS), "{name}, read again");
    }
    // Rows written to a collection that had given its blocks back are
    // there at its next read, once and in order.
    cols[0].insert_rows(rows("a", ROWS, ROWS + 5_000)).unwrap();
    for (name, col) in names.iter().zip(&cols).skip(1) {
        assert_eq!(read(col).len(), ROWS as usize, "{name}");
    }
    assert_eq!(read(&cols[0]), rows("a", 0, ROWS + 5_000));
    assert!(decoded_blocks_kept() <= 1 << 20);

    // A collection alone past the budget is still read whole, and keeps
    // nothing.
    let big = create(&engine, "big");
    big.insert_rows(rows("big", 0, 4 * ROWS)).unwrap();
    assert_eq!(read(&big), rows("big", 0, 4 * ROWS));
    assert_eq!(read(&big).len(), 4 * ROWS as usize);
    assert!(
        decoded_blocks_kept() <= 1 << 20,
        "{} bytes kept",
        decoded_blocks_kept()
    );
}
