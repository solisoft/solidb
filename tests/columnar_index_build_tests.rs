//! Building an index on a column that already holds many rows (card
//! #916): the entries are built in memory and written in one batch, where
//! each row used to rewrite the list of its value.

use serde_json::{json, Value};
use solidb::storage::columnar::ColumnarIndexType;
use solidb::sync::columnar;
use solidb::{ColumnDef, ColumnFilter, ColumnType, CompressionType, StorageEngine};
use tempfile::TempDir;

const ROWS: i64 = 100_000;

fn filters() -> Vec<ColumnFilter> {
    vec![
        ColumnFilter::Eq("host".to_string(), json!("h3")),
        ColumnFilter::Eq("host".to_string(), json!("nobody")),
        ColumnFilter::Eq("v".to_string(), json!(70_000)),
        ColumnFilter::Gt("v".to_string(), json!(99_990)),
        ColumnFilter::Lt("v".to_string(), json!(12)),
        ColumnFilter::Gte("v".to_string(), json!(65_530)),
    ]
}

/// The `v` of the rows each filter keeps.
fn answers(col: &solidb::ColumnarCollection) -> Vec<Vec<i64>> {
    filters()
        .iter()
        .map(|filter| {
            let mut kept: Vec<i64> = col
                .scan_filtered(filter, &["v"])
                .unwrap()
                .iter()
                .filter_map(|row: &Value| row["v"].as_i64())
                .collect();
            kept.sort_unstable();
            kept
        })
        .collect()
}

#[test]
fn an_index_built_over_many_rows_answers_as_no_index_does() {
    let dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("app".to_string()).unwrap();
    let db = engine.get_database("app").unwrap();
    let column = |name: &str, data_type| ColumnDef {
        name: name.to_string(),
        data_type,
        nullable: false,
        indexed: false,
        index_type: None,
    };
    let columns = vec![
        column("host", ColumnType::String),
        column("v", ColumnType::Int64),
    ];
    let col = columnar::create(&db, None, "metrics", columns, CompressionType::Lz4).unwrap();
    // More rows than one block holds, and a few left in the tail.
    col.insert_rows(
        (0..ROWS)
            .map(|i| json!({"host": format!("h{}", i % 7), "v": i}))
            .collect(),
    )
    .unwrap();
    col.insert_rows(
        (ROWS..ROWS + 5)
            .map(|i| json!({"host": "h3", "v": i}))
            .collect(),
    )
    .unwrap();

    let unindexed = answers(&col);
    assert_eq!(unindexed[0].len(), 14_286 + 5);
    assert_eq!(unindexed[1].len(), 0);
    assert_eq!(unindexed[2], [70_000]);

    for (name, kind) in [
        ("hash", ColumnarIndexType::Hash),
        ("sorted", ColumnarIndexType::Sorted),
        ("bitmap", ColumnarIndexType::Bitmap),
        ("bloom", ColumnarIndexType::Bloom),
    ] {
        for indexed in ["host", "v"] {
            col.create_index(indexed, kind.clone()).unwrap();
            assert_eq!(answers(&col), unindexed, "{name} index on {indexed}");
            // Created twice is refused, and dropping it leaves nothing.
            assert!(col.create_index(indexed, kind.clone()).is_err());
            col.drop_index(indexed).unwrap();
            assert_eq!(
                answers(&col),
                unindexed,
                "after dropping the {name} index on {indexed}"
            );
        }
    }
}
