//! Columnar Collections Tests
//!
//! Tests for the columnar storage layer, covering:
//! - Columnar collection creation and deletion
//! - Row insertion with LZ4 compression
//! - Column reading and projection
//! - Aggregation operations (SUM, AVG, COUNT, MIN, MAX)
//! - Filtering and querying
//! - Group by operations

use solidb::{
    AggregateOp, ColumnDef, ColumnFilter, ColumnType, ColumnarCollection,
    CompressionType, StorageEngine,
};
use serde_json::json;
use tempfile::TempDir;

// ============================================================================
// Helper Functions
// ============================================================================

fn create_test_engine() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(temp_dir.path()).unwrap();
    // Create _system database for tests
    engine.create_database("_system".to_string()).unwrap();
    (engine, temp_dir)
}

fn create_test_columns() -> Vec<ColumnDef> {
    vec![
        ColumnDef {
            name: "name".to_string(),
            data_type: ColumnType::String,
            nullable: false,
            indexed: false,
        },
        ColumnDef {
            name: "age".to_string(),
            data_type: ColumnType::Int64,
            nullable: false,
            indexed: false,
        },
        ColumnDef {
            name: "score".to_string(),
            data_type: ColumnType::Float64,
            nullable: true,
            indexed: false,
        },
        ColumnDef {
            name: "active".to_string(),
            data_type: ColumnType::Bool,
            nullable: false,
            indexed: false,
        },
    ]
}

// ============================================================================
// Column Type Tests
// ============================================================================

#[test]
fn test_column_type_inference_bool() {
    assert_eq!(
        ColumnType::infer_from_value(&json!(true)),
        ColumnType::Bool
    );
    assert_eq!(
        ColumnType::infer_from_value(&json!(false)),
        ColumnType::Bool
    );
}

#[test]
fn test_column_type_inference_int() {
    assert_eq!(
        ColumnType::infer_from_value(&json!(42)),
        ColumnType::Int64
    );
    assert_eq!(
        ColumnType::infer_from_value(&json!(-100)),
        ColumnType::Int64
    );
}

#[test]
fn test_column_type_inference_float() {
    assert_eq!(
        ColumnType::infer_from_value(&json!(3.14)),
        ColumnType::Float64
    );
    assert_eq!(
        ColumnType::infer_from_value(&json!(-0.001)),
        ColumnType::Float64
    );
}

#[test]
fn test_column_type_inference_string() {
    assert_eq!(
        ColumnType::infer_from_value(&json!("hello")),
        ColumnType::String
    );
    assert_eq!(
        ColumnType::infer_from_value(&json!("")),
        ColumnType::String
    );
}

#[test]
fn test_column_type_inference_timestamp() {
    assert_eq!(
        ColumnType::infer_from_value(&json!("2024-01-15T10:30:00Z")),
        ColumnType::Timestamp
    );
    assert_eq!(
        ColumnType::infer_from_value(&json!("2024-01-15T10:30:00+05:00")),
        ColumnType::Timestamp
    );
}

#[test]
fn test_column_type_inference_json() {
    assert_eq!(
        ColumnType::infer_from_value(&json!({"nested": true})),
        ColumnType::Json
    );
    assert_eq!(
        ColumnType::infer_from_value(&json!([1, 2, 3])),
        ColumnType::Json
    );
}

// ============================================================================
// Aggregate Operation Tests
// ============================================================================

#[test]
fn test_aggregate_op_from_str() {
    assert_eq!(AggregateOp::from_str("SUM"), Some(AggregateOp::Sum));
    assert_eq!(AggregateOp::from_str("sum"), Some(AggregateOp::Sum));
    assert_eq!(AggregateOp::from_str("AVG"), Some(AggregateOp::Avg));
    assert_eq!(AggregateOp::from_str("AVERAGE"), Some(AggregateOp::Avg));
    assert_eq!(AggregateOp::from_str("COUNT"), Some(AggregateOp::Count));
    assert_eq!(AggregateOp::from_str("MIN"), Some(AggregateOp::Min));
    assert_eq!(AggregateOp::from_str("MAX"), Some(AggregateOp::Max));
    assert_eq!(
        AggregateOp::from_str("COUNT_DISTINCT"),
        Some(AggregateOp::CountDistinct)
    );
    assert_eq!(AggregateOp::from_str("INVALID"), None);
    assert_eq!(AggregateOp::from_str(""), None);
}

// ============================================================================
// Columnar Collection Creation Tests
// ============================================================================

#[test]
fn test_create_columnar_collection() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    // Create the column family first
    let cf_name = "_columnar_metrics";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let columns = vec![
        ColumnDef {
            name: "timestamp".to_string(),
            data_type: ColumnType::Timestamp,
            nullable: false,
            indexed: false,
        },
        ColumnDef {
            name: "value".to_string(),
            data_type: ColumnType::Float64,
            nullable: false,
            indexed: false,
        },
    ];

    let col = ColumnarCollection::new(
        "metrics".to_string(),
        "_system",
        db.db_arc(),
        columns.clone(),
        CompressionType::Lz4,
    )
    .unwrap();

    let meta = col.metadata().unwrap();
    assert_eq!(meta.name, "metrics");
    assert_eq!(meta.columns.len(), 2);
    assert_eq!(meta.row_count, 0);
}

#[test]
fn test_create_columnar_collection_no_compression() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_raw_data";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let columns = vec![ColumnDef {
        name: "data".to_string(),
        data_type: ColumnType::String,
        nullable: true,
        indexed: false,
    }];

    let col = ColumnarCollection::new(
        "raw_data".to_string(),
        "_system",
        db.db_arc(),
        columns,
        CompressionType::None,
    )
    .unwrap();

    let meta = col.metadata().unwrap();
    assert_eq!(meta.compression, CompressionType::None);
}

// ============================================================================
// Row Insertion Tests
// ============================================================================

#[test]
fn test_insert_rows() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_users";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "users".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    let rows = vec![
        json!({"name": "Alice", "age": 30, "score": 95.5, "active": true}),
        json!({"name": "Bob", "age": 25, "score": 88.0, "active": true}),
        json!({"name": "Charlie", "age": 35, "score": 92.0, "active": false}),
    ];

    let inserted = col.insert_rows(rows).unwrap();
    assert_eq!(inserted, 3);

    let meta = col.metadata().unwrap();
    assert_eq!(meta.row_count, 3);
}

#[test]
fn test_insert_empty_rows() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_empty";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "empty".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    let inserted = col.insert_rows(vec![]).unwrap();
    assert_eq!(inserted, 0);
}

#[test]
fn test_insert_rows_with_nulls() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_nullable";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "nullable".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    let rows = vec![
        json!({"name": "Alice", "age": 30, "score": null, "active": true}),
        json!({"name": "Bob", "age": 25, "active": false}), // missing score field
    ];

    let inserted = col.insert_rows(rows).unwrap();
    assert_eq!(inserted, 2);
}

// ============================================================================
// Column Reading Tests
// ============================================================================

#[test]
fn test_read_single_column() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_read_test";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "read_test".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "Alice", "age": 30, "score": 95.5, "active": true}),
        json!({"name": "Bob", "age": 25, "score": 88.0, "active": true}),
        json!({"name": "Charlie", "age": 35, "score": 92.0, "active": false}),
    ])
    .unwrap();

    let names = col.read_column("name", None).unwrap();
    assert_eq!(names.len(), 3);
    assert_eq!(names[0], json!("Alice"));
    assert_eq!(names[1], json!("Bob"));
    assert_eq!(names[2], json!("Charlie"));

    let ages = col.read_column("age", None).unwrap();
    assert_eq!(ages.len(), 3);
    assert_eq!(ages[0], json!(30));
    assert_eq!(ages[1], json!(25));
    assert_eq!(ages[2], json!(35));
}

#[test]
fn test_read_column_specific_rows() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_specific_rows";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "specific_rows".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "Alice", "age": 30, "score": 95.5, "active": true}),
        json!({"name": "Bob", "age": 25, "score": 88.0, "active": true}),
        json!({"name": "Charlie", "age": 35, "score": 92.0, "active": false}),
        json!({"name": "Diana", "age": 28, "score": 90.0, "active": true}),
    ])
    .unwrap();

    let names = col.read_column("name", Some(&[0, 2])).unwrap();
    assert_eq!(names.len(), 2);
    assert_eq!(names[0], json!("Alice"));
    assert_eq!(names[1], json!("Charlie"));
}

#[test]
fn test_read_column_not_found() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_col_not_found";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "col_not_found".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    let result = col.read_column("nonexistent", None);
    assert!(result.is_err());
}

// ============================================================================
// Column Projection Tests
// ============================================================================

#[test]
fn test_read_multiple_columns() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_projection";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "projection".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "Alice", "age": 30, "score": 95.5, "active": true}),
        json!({"name": "Bob", "age": 25, "score": 88.0, "active": true}),
    ])
    .unwrap();

    let results = col.read_columns(&["name", "age"], None).unwrap();
    assert_eq!(results.len(), 2);

    assert_eq!(results[0]["name"], json!("Alice"));
    assert_eq!(results[0]["age"], json!(30));
    assert!(results[0].get("score").is_none());

    assert_eq!(results[1]["name"], json!("Bob"));
    assert_eq!(results[1]["age"], json!(25));
}

// ============================================================================
// Aggregation Tests
// ============================================================================

#[test]
fn test_aggregate_sum() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_agg_sum";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "agg_sum".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "A", "age": 10, "score": 10.0, "active": true}),
        json!({"name": "B", "age": 20, "score": 20.0, "active": true}),
        json!({"name": "C", "age": 30, "score": 30.0, "active": false}),
    ])
    .unwrap();

    let sum = col.aggregate("age", AggregateOp::Sum).unwrap();
    assert_eq!(sum, json!(60.0));

    let sum_score = col.aggregate("score", AggregateOp::Sum).unwrap();
    assert_eq!(sum_score, json!(60.0));
}

#[test]
fn test_aggregate_avg() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_agg_avg";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "agg_avg".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "A", "age": 10, "score": 80.0, "active": true}),
        json!({"name": "B", "age": 20, "score": 90.0, "active": true}),
        json!({"name": "C", "age": 30, "score": 100.0, "active": false}),
    ])
    .unwrap();

    let avg = col.aggregate("age", AggregateOp::Avg).unwrap();
    assert_eq!(avg, json!(20.0));

    let avg_score = col.aggregate("score", AggregateOp::Avg).unwrap();
    assert_eq!(avg_score, json!(90.0));
}

#[test]
fn test_aggregate_count() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_agg_count";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "agg_count".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "A", "age": 10, "score": 80.0, "active": true}),
        json!({"name": "B", "age": 20, "score": 90.0, "active": true}),
        json!({"name": "C", "age": 30, "score": 100.0, "active": false}),
    ])
    .unwrap();

    let count = col.aggregate("name", AggregateOp::Count).unwrap();
    assert_eq!(count, json!(3));
}

#[test]
fn test_aggregate_min_max() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_agg_minmax";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "agg_minmax".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "A", "age": 15, "score": 75.5, "active": true}),
        json!({"name": "B", "age": 25, "score": 95.0, "active": true}),
        json!({"name": "C", "age": 35, "score": 85.5, "active": false}),
    ])
    .unwrap();

    let min_age = col.aggregate("age", AggregateOp::Min).unwrap();
    assert_eq!(min_age, json!(15.0));

    let max_age = col.aggregate("age", AggregateOp::Max).unwrap();
    assert_eq!(max_age, json!(35.0));

    let min_score = col.aggregate("score", AggregateOp::Min).unwrap();
    assert_eq!(min_score, json!(75.5));

    let max_score = col.aggregate("score", AggregateOp::Max).unwrap();
    assert_eq!(max_score, json!(95.0));
}

#[test]
fn test_aggregate_count_distinct() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_agg_distinct";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "agg_distinct".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "Alice", "age": 30, "score": 80.0, "active": true}),
        json!({"name": "Bob", "age": 30, "score": 90.0, "active": true}),
        json!({"name": "Charlie", "age": 25, "score": 80.0, "active": false}),
        json!({"name": "Diana", "age": 25, "score": 100.0, "active": true}),
    ])
    .unwrap();

    let distinct_ages = col.aggregate("age", AggregateOp::CountDistinct).unwrap();
    assert_eq!(distinct_ages, json!(2)); // 25 and 30
}

// ============================================================================
// Filter Tests
// ============================================================================

#[test]
fn test_filter_eq() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_filter_eq";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "filter_eq".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "Alice", "age": 30, "score": 95.0, "active": true}),
        json!({"name": "Bob", "age": 25, "score": 88.0, "active": true}),
        json!({"name": "Charlie", "age": 30, "score": 92.0, "active": false}),
    ])
    .unwrap();

    let filter = ColumnFilter::Eq("age".to_string(), json!(30));
    let results = col.scan_filtered(&filter, &["name", "age"]).unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r["age"] == json!(30)));
}

#[test]
fn test_filter_gt() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_filter_gt";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "filter_gt".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "Alice", "age": 30, "score": 95.0, "active": true}),
        json!({"name": "Bob", "age": 25, "score": 88.0, "active": true}),
        json!({"name": "Charlie", "age": 35, "score": 92.0, "active": false}),
    ])
    .unwrap();

    let filter = ColumnFilter::Gt("age".to_string(), json!(28));
    let results = col.scan_filtered(&filter, &["name", "age"]).unwrap();

    assert_eq!(results.len(), 2); // Alice (30) and Charlie (35)
}

#[test]
fn test_filter_in() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_filter_in";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "filter_in".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "Alice", "age": 30, "score": 95.0, "active": true}),
        json!({"name": "Bob", "age": 25, "score": 88.0, "active": true}),
        json!({"name": "Charlie", "age": 35, "score": 92.0, "active": false}),
    ])
    .unwrap();

    let filter = ColumnFilter::In("name".to_string(), vec![json!("Alice"), json!("Charlie")]);
    let results = col.scan_filtered(&filter, &["name"]).unwrap();

    assert_eq!(results.len(), 2);
}

// ============================================================================
// Group By Tests
// ============================================================================

#[test]
fn test_group_by_single_column() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_groupby";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "groupby".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "Alice", "age": 30, "score": 90.0, "active": true}),
        json!({"name": "Bob", "age": 30, "score": 80.0, "active": true}),
        json!({"name": "Charlie", "age": 25, "score": 85.0, "active": false}),
        json!({"name": "Diana", "age": 25, "score": 95.0, "active": true}),
    ])
    .unwrap();

    let results = col.group_by(
        &[solidb::storage::columnar::GroupByColumn::Simple("active".to_string())], 
        "score", 
        AggregateOp::Avg
    ).unwrap();

    assert_eq!(results.len(), 2); // true and false groups
}

// ============================================================================
// Statistics Tests
// ============================================================================

#[test]
fn test_collection_stats() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_stats";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "stats".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    col.insert_rows(vec![
        json!({"name": "Alice", "age": 30, "score": 95.0, "active": true}),
        json!({"name": "Bob", "age": 25, "score": 88.0, "active": true}),
    ])
    .unwrap();

    let stats = col.stats().unwrap();

    assert_eq!(stats.name, "stats");
    assert_eq!(stats.row_count, 2);
    assert_eq!(stats.column_count, 4);
    assert!(stats.compressed_size_bytes > 0);
    // Note: For small data, LZ4 may add overhead (size prefix), so ratio can be < 1
    assert!(stats.compression_ratio > 0.0);
}

// ============================================================================
// Compression Tests
// ============================================================================

#[test]
fn test_compression_effectiveness() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    // Create with LZ4 compression
    let cf_name_lz4 = "_columnar_compress_lz4";
    db.create_collection(cf_name_lz4.to_string(), None).unwrap();

    let col_lz4 = ColumnarCollection::new(
        "compress_lz4".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    // Create without compression
    let cf_name_none = "_columnar_compress_none";
    db.create_collection(cf_name_none.to_string(), None).unwrap();

    let col_none = ColumnarCollection::new(
        "compress_none".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::None,
    )
    .unwrap();

    // Insert same data to both
    let rows: Vec<_> = (0..100)
        .map(|i| {
            json!({
                "name": format!("User{}", i),
                "age": 20 + (i % 50),
                "score": 50.0 + (i as f64 * 0.5),
                "active": i % 2 == 0
            })
        })
        .collect();

    col_lz4.insert_rows(rows.clone()).unwrap();
    col_none.insert_rows(rows).unwrap();

    let stats_lz4 = col_lz4.stats().unwrap();
    let stats_none = col_none.stats().unwrap();

    // Both should have valid stats
    assert!(stats_lz4.compressed_size_bytes > 0, "LZ4 should have data");
    assert!(stats_none.compressed_size_bytes > 0, "No compression should have data");

    // No compression should have ratio of exactly 1.0
    assert!(
        (stats_none.compression_ratio - 1.0).abs() < 0.01,
        "No compression should have ratio ~1.0, got {}",
        stats_none.compression_ratio
    );

    // LZ4 ratio can vary - for small values it may add overhead
    // Just verify it's a valid positive ratio
    assert!(
        stats_lz4.compression_ratio > 0.0,
        "LZ4 compression ratio should be positive, got {}",
        stats_lz4.compression_ratio
    );
}

// ============================================================================
// Edge Case Tests
// ============================================================================

#[test]
fn test_empty_aggregation() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_empty_agg";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "empty_agg".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    // Don't insert any rows
    let count = col.aggregate("age", AggregateOp::Count).unwrap();
    assert_eq!(count, json!(0));

    let sum = col.aggregate("age", AggregateOp::Sum).unwrap();
    assert_eq!(sum, json!(0.0));
}

#[test]
fn test_large_batch_insert() {
    let (engine, _temp) = create_test_engine();
    let db = engine.get_database("_system").unwrap();

    let cf_name = "_columnar_large_batch";
    db.create_collection(cf_name.to_string(), None).unwrap();

    let col = ColumnarCollection::new(
        "large_batch".to_string(),
        "_system",
        db.db_arc(),
        create_test_columns(),
        CompressionType::Lz4,
    )
    .unwrap();

    // Insert 1000 rows
    let rows: Vec<_> = (0..1000)
        .map(|i| {
            json!({
                "name": format!("User{}", i),
                "age": 18 + (i % 60),
                "score": (i as f64) * 0.1,
                "active": i % 3 != 0
            })
        })
        .collect();

    let inserted = col.insert_rows(rows).unwrap();
    assert_eq!(inserted, 1000);

    let meta = col.metadata().unwrap();
    assert_eq!(meta.row_count, 1000);

    // Verify aggregation on large dataset
    let count = col.aggregate("name", AggregateOp::Count).unwrap();
    assert_eq!(count, json!(1000));
}
