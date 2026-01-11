use serde_json::json;
use solidb::storage::columnar::GroupByColumn;
use solidb::{
    AggregateOp, ColumnDef, ColumnType, ColumnarCollection, CompressionType, StorageEngine,
};
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(temp_dir.path()).unwrap();
    engine.create_database("_system".to_string()).unwrap();
    (engine, temp_dir)
}

fn create_test_columns() -> Vec<ColumnDef> {
    vec![
        ColumnDef {
            name: "timestamp".to_string(),
            data_type: ColumnType::Timestamp,
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
    ]
}

#[test]
fn test_time_bucket_grouping() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_metrics_test";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "metrics_test".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    // Insert data spanning multiple hours
    // Times: 10:05, 10:45, 11:10, 11:50, 12:05
    let rows = vec![
        json!({"timestamp": "2024-01-01T10:05:00Z", "value": 10.0}),
        json!({"timestamp": "2024-01-01T10:45:00Z", "value": 20.0}),
        json!({"timestamp": "2024-01-01T11:10:00Z", "value": 30.0}),
        json!({"timestamp": "2024-01-01T11:50:00Z", "value": 40.0}),
        json!({"timestamp": "2024-01-01T12:05:00Z", "value": 50.0}),
    ];

    col.insert_rows(rows).unwrap();

    // Group by 1h bucket
    let group_cols = vec![GroupByColumn::TimeBucket(
        "timestamp".to_string(),
        "1h".to_string(),
    )];

    let results = col
        .group_by(&group_cols, "value", AggregateOp::Sum)
        .unwrap();

    // Expected groups:
    // 10:00 -> 10+20 = 30
    // 11:00 -> 30+40 = 70
    // 12:00 -> 50 = 50

    assert_eq!(results.len(), 3);

    // Verify 10:00
    let group_10 = results
        .iter()
        .find(|r| {
            r["timestamp"]
                .as_str()
                .unwrap()
                .starts_with("2024-01-01T10:00")
        })
        .unwrap();
    assert_eq!(group_10["_agg"], json!(30.0));

    // Verify 11:00
    let group_11 = results
        .iter()
        .find(|r| {
            r["timestamp"]
                .as_str()
                .unwrap()
                .starts_with("2024-01-01T11:00")
        })
        .unwrap();
    assert_eq!(group_11["_agg"], json!(70.0));

    // Verify 12:00
    let group_12 = results
        .iter()
        .find(|r| {
            r["timestamp"]
                .as_str()
                .unwrap()
                .starts_with("2024-01-01T12:00")
        })
        .unwrap();
    assert_eq!(group_12["_agg"], json!(50.0));
}
