use serde_json::json;
use solidb::storage::columnar::{
    ColumnDef, ColumnFilter, ColumnType, ColumnarCollection, ColumnarIndexType, CompressionType,
};
use solidb::storage::StorageEngine;

use tempfile::tempdir;

#[tokio::test]
async fn test_bitmap_index_correctness() {
    let dir = tempdir().unwrap();
    let storage = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
    storage.create_database("test_db".to_string()).unwrap();
    let db = storage.get_database("test_db").unwrap();

    let columns = vec![
        ColumnDef {
            name: "status".to_string(),
            data_type: ColumnType::String,
            nullable: false,
            indexed: false,
            index_type: None,
        },
        ColumnDef {
            name: "id".to_string(),
            data_type: ColumnType::Int64,
            nullable: false,
            indexed: false,
            index_type: None,
        },
    ];

    db.create_collection("_columnar_users".to_string(), None)
        .unwrap();
    let col = ColumnarCollection::new(
        "users".to_string(),
        "test_db",
        db.db_arc(),
        columns,
        CompressionType::None,
    )
    .unwrap();

    // Insert data: 100 rows, status matches row_id % 4
    // 0: "active", 1: "inactive", 2: "pending", 3: "banned"
    let mut rows = Vec::new();
    for i in 0..100 {
        let status = match i % 4 {
            0 => "active",
            1 => "inactive",
            2 => "pending",
            _ => "banned",
        };
        rows.push(json!({
            "status": status,
            "id": i
        }));
    }
    col.insert_rows(rows).unwrap();

    // Create Bitmap Index on "status"
    col.create_index("status", ColumnarIndexType::Bitmap)
        .unwrap();

    // Query Eq "active" -> Should match indices 0, 4, 8...
    let filter = ColumnFilter::Eq("status".to_string(), json!("active"));
    let results = col.scan_filtered(&filter, &["id"]).unwrap();

    assert_eq!(results.len(), 25);
    for row in results {
        let id = row["id"].as_i64().unwrap();
        assert_eq!(id % 4, 0);
    }

    // Query In ["active", "pending"] -> 50 rows
    let filter = ColumnFilter::In(
        "status".to_string(),
        vec![json!("active"), json!("pending")],
    );
    let results = col.scan_filtered(&filter, &["id"]).unwrap();

    assert_eq!(results.len(), 50);
}

#[tokio::test]
async fn test_minmax_index_correctness() {
    let dir = tempdir().unwrap();
    let storage = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
    storage.create_database("test_db".to_string()).unwrap();
    let db = storage.get_database("test_db").unwrap();

    let columns = vec![ColumnDef {
        name: "val".to_string(),
        data_type: ColumnType::Int64,
        nullable: false,
        indexed: false,
        index_type: None,
    }];

    db.create_collection("_columnar_metrics".to_string(), None)
        .unwrap();
    let col = ColumnarCollection::new(
        "metrics".to_string(),
        "test_db",
        db.db_arc(),
        columns,
        CompressionType::None,
    )
    .unwrap();

    // Insert 2000 rows. Row i = i.
    // Chunk 0: 0-999. Chunk 1: 1000-1999. (CHUNK_SIZE=1000)
    let mut rows = Vec::new();
    for i in 0..2000 {
        rows.push(json!({ "val": i }));
    }
    // Batch insert to ensure chunks are filled
    col.insert_rows(rows).unwrap();

    // Create MinMax Index
    col.create_index("val", ColumnarIndexType::MinMax).unwrap();

    // Query Gt 1500. Should prune Chunk 0 completely (Max 999 < 1500).
    // Should scan Chunk 1 and filter.
    let filter = ColumnFilter::Gt("val".to_string(), json!(1500));
    let results = col.scan_filtered(&filter, &["val"]).unwrap();

    // Result count: 1999 - 1500 = 499 rows?
    // 1501..1999 -> 499 values. +1 if inclusive? Gt is exclusive. 1999-1500=499.
    assert_eq!(results.len(), 499);

    // Verify first result
    let min_val = results
        .iter()
        .map(|r| r["val"].as_i64().unwrap())
        .min()
        .unwrap();
    assert_eq!(min_val, 1501);

    // Query Lt 500. Should prune Chunk 1 completely (Min 1000 > 500).
    // Should scan Chunk 0.
    let filter = ColumnFilter::Lt("val".to_string(), json!(500));
    let results = col.scan_filtered(&filter, &["val"]).unwrap();
    assert_eq!(results.len(), 500); // 0..499
}

/// A collection `c` of a database of its own, with a string and a number.
fn stale_fixture(dir: &tempfile::TempDir) -> (StorageEngine, ColumnarCollection) {
    let storage = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
    storage.create_database("app".to_string()).unwrap();
    let db = storage.get_database("app").unwrap();
    db.create_collection("_columnar_c".to_string(), None)
        .unwrap();
    let columns = vec![
        ColumnDef {
            name: "s".to_string(),
            data_type: ColumnType::String,
            nullable: false,
            indexed: false,
            index_type: None,
        },
        ColumnDef {
            name: "v".to_string(),
            data_type: ColumnType::Int64,
            nullable: false,
            indexed: false,
            index_type: None,
        },
    ];
    let col = ColumnarCollection::new(
        "c".to_string(),
        "app",
        db.db_arc(),
        columns,
        CompressionType::Lz4,
    )
    .unwrap();
    (storage, col)
}

fn stale_rows(from: i64, to: i64) -> Vec<serde_json::Value> {
    (from..to)
        .map(|i| json!({"s": format!("k{}", i % 7), "v": i}))
        .collect()
}

/// Every filter the API takes, on each of the two columns.
fn stale_filters() -> Vec<ColumnFilter> {
    let s = || "s".to_string();
    let v = || "v".to_string();
    vec![
        ColumnFilter::Eq(s(), json!("k3")),
        ColumnFilter::Eq(s(), json!("nowhere")),
        ColumnFilter::Ne(s(), json!("k3")),
        ColumnFilter::In(s(), vec![json!("k1"), json!("k6")]),
        ColumnFilter::Eq(v(), json!(205)),
        ColumnFilter::Gt(v(), json!(190)),
        ColumnFilter::Gte(v(), json!(190)),
        ColumnFilter::Lt(v(), json!(12)),
        ColumnFilter::Lte(v(), json!(12)),
        ColumnFilter::In(v(), vec![json!(3), json!(150), json!(260)]),
        ColumnFilter::And(vec![
            ColumnFilter::Eq(s(), json!("k2")),
            ColumnFilter::Gt(v(), json!(100)),
        ]),
        ColumnFilter::Or(vec![
            ColumnFilter::Eq(s(), json!("k2")),
            ColumnFilter::Lt(v(), json!(5)),
        ]),
    ]
}

fn stale_answers(col: &ColumnarCollection) -> Vec<Vec<serde_json::Value>> {
    stale_filters()
        .iter()
        .map(|filter| {
            let mut rows = col.scan_filtered(filter, &["s", "v"]).unwrap();
            rows.sort_by_key(|row| row["v"].as_i64());
            rows
        })
        .collect()
}

/// An index is written when it is created and names rows by position. A
/// row inserted afterwards was never found through it, and a delete or a
/// row with an older id moved the positions it named (card #901). Whatever
/// is written after an index is made, a filter answers as with no index.
#[test]
fn an_index_answers_as_no_index_does_after_writes() {
    for index_type in [
        ColumnarIndexType::Sorted,
        ColumnarIndexType::Hash,
        ColumnarIndexType::Bitmap,
        ColumnarIndexType::MinMax,
        ColumnarIndexType::Bloom,
    ] {
        // The same writes on both; only one is indexed.
        let (dir_plain, dir_indexed) = (tempdir().unwrap(), tempdir().unwrap());
        let (_keep_plain, plain) = stale_fixture(&dir_plain);
        let (_keep_indexed, indexed) = stale_fixture(&dir_indexed);
        let both = |step: &dyn Fn(&ColumnarCollection)| {
            step(&plain);
            step(&indexed);
        };
        let same = |when: &str| {
            let (expected, got) = (stale_answers(&plain), stale_answers(&indexed));
            for ((filter, expected), got) in stale_filters().iter().zip(expected).zip(got) {
                assert_eq!(got, expected, "{index_type:?}, {when}: {filter:?}");
            }
        };

        both(&|col| {
            col.insert_rows(stale_rows(0, 200)).unwrap();
        });
        for column in ["s", "v"] {
            indexed.create_index(column, index_type.clone()).unwrap();
        }
        same("right after the index");

        both(&|col| {
            col.insert_rows(stale_rows(200, 230)).unwrap();
        });
        same("after an insert");

        // Enough at once for a block of their own.
        both(&|col| {
            col.insert_rows(stale_rows(230, 5000)).unwrap();
        });
        same("after a large insert");

        // A row from another node, older than every row here: it lands
        // first and moves every position.
        both(&|col| {
            let inserted = col
                .insert_row_with_id(
                    "00000000-0000-7000-8000-000000000001",
                    json!({"s": "k3", "v": 205}),
                )
                .unwrap();
            assert!(inserted);
        });
        same("after a row with an older id");

        // A delete in the middle of what the index held.
        both(&|col| {
            let ids = col.list_row_uuids().unwrap();
            assert!(col.delete_row(&ids[100]).unwrap());
        });
        same("after a delete");

        both(&|col| {
            col.truncate().unwrap();
            col.insert_rows(stale_rows(0, 50)).unwrap();
        });
        same("after a truncate and new rows");

        // Made again, it holds every row.
        for column in ["s", "v"] {
            indexed.drop_index(column).unwrap();
            assert_eq!(indexed.get_index_type(column), None);
            indexed.create_index(column, index_type.clone()).unwrap();
        }
        same("after the index is made again");
    }
}

/// What an index holds survives a reload of the collection.
#[test]
fn what_an_index_holds_is_kept_across_loads() {
    let dir = tempdir().unwrap();
    let (storage, col) = stale_fixture(&dir);
    col.insert_rows(stale_rows(0, 100)).unwrap();
    col.create_index("s", ColumnarIndexType::Bitmap).unwrap();
    col.insert_rows(stale_rows(100, 140)).unwrap();
    assert_eq!(col.metadata().unwrap().index_rows.get("s"), Some(&100));

    let db = storage.get_database("app").unwrap();
    let again = ColumnarCollection::load("c".to_string(), "app", db.db_arc()).unwrap();
    assert_eq!(again.metadata().unwrap().index_rows.get("s"), Some(&100));
    let filter = ColumnFilter::Eq("s".to_string(), json!("k3"));
    assert_eq!(again.scan_filtered(&filter, &["v"]).unwrap().len(), 20);
}
