use solidb::storage::columnar::{ColumnDef, ColumnType, ColumnarCollection, CompressionType, ColumnarIndexType, ColumnFilter};
use solidb::storage::StorageEngine;
use serde_json::json;

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

    db.create_collection("_columnar_users".to_string(), None).unwrap();
    let col = ColumnarCollection::new(
        "users".to_string(),
        "test_db",
        db.db_arc(),
        columns,
        CompressionType::None,
    ).unwrap();

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
    col.create_index("status", ColumnarIndexType::Bitmap).unwrap();

    // Query Eq "active" -> Should match indices 0, 4, 8...
    let filter = ColumnFilter::Eq("status".to_string(), json!("active"));
    let results = col.scan_filtered(&filter, &vec!["id"]).unwrap();
    
    assert_eq!(results.len(), 25);
    for row in results {
        let id = row["id"].as_i64().unwrap();
        assert_eq!(id % 4, 0);
    }

    // Query In ["active", "pending"] -> 50 rows
    let filter = ColumnFilter::In("status".to_string(), vec![json!("active"), json!("pending")]);
    let results = col.scan_filtered(&filter, &vec!["id"]).unwrap();
    
    assert_eq!(results.len(), 50);
}

#[tokio::test]
async fn test_minmax_index_correctness() {
    let dir = tempdir().unwrap();
    let storage = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
    storage.create_database("test_db".to_string()).unwrap();
    let db = storage.get_database("test_db").unwrap();

    let columns = vec![
        ColumnDef {
            name: "val".to_string(),
            data_type: ColumnType::Int64,
            nullable: false,
            indexed: false,
            index_type: None,
        },
    ];

    db.create_collection("_columnar_metrics".to_string(), None).unwrap();
    let col = ColumnarCollection::new(
        "metrics".to_string(),
        "test_db",
        db.db_arc(),
        columns,
        CompressionType::None,
    ).unwrap();

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
    let results = col.scan_filtered(&filter, &vec!["val"]).unwrap();
    
    // Result count: 1999 - 1500 = 499 rows?
    // 1501..1999 -> 499 values. +1 if inclusive? Gt is exclusive. 1999-1500=499.
    assert_eq!(results.len(), 499);
    
    // Verify first result
    let min_val = results.iter().map(|r| r["val"].as_i64().unwrap()).min().unwrap();
    assert_eq!(min_val, 1501);

    // Query Lt 500. Should prune Chunk 1 completely (Min 1000 > 500).
    // Should scan Chunk 0.
    let filter = ColumnFilter::Lt("val".to_string(), json!(500));
    let results = col.scan_filtered(&filter, &vec!["val"]).unwrap();
    assert_eq!(results.len(), 500); // 0..499
}
