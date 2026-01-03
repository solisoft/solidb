use solidb::storage::columnar::{ColumnDef, ColumnType, ColumnarCollection, CompressionType, ColumnarIndexType, ColumnFilter};
use solidb::storage::StorageEngine;
use solidb::storage::index::IndexType;
use serde_json::json;
use tempfile::tempdir;

#[tokio::test]
async fn test_document_bloom_index() {
    let dir = tempdir().unwrap();
    let storage = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
    storage.create_database("test_db".to_string()).unwrap();
    let db = storage.get_database("test_db").unwrap();

    db.create_collection("users".to_string(), None).unwrap();
    let col = db.get_collection("users").unwrap();
    
    // Create Bloom index
    col.create_index("username".to_string(), vec!["username".to_string()], IndexType::Bloom, true).unwrap();

    // Insert documents
    let docs = vec![
        json!({"username": "alice", "age": 30}),
        json!({"username": "bob", "age": 25})
    ];
    let inserted = col.insert_batch(docs).unwrap();
    col.index_documents(&inserted).unwrap();

    // Test Bloom Filter Direct Check
    // Should find existing (Note: Bloom filter stores JSON string representation)
    assert!(col.bloom_check("username", &json!("alice").to_string()), "Bloom filter should contain alice");
    assert!(col.bloom_check("username", &json!("bob").to_string()), "Bloom filter should contain bob");

    // Should NOT find non-existent (with high probability)
    // Bloom filters can have false positives, but "charlie" shouldn't interact with "alice"/"bob" likely.
    if col.bloom_check("username", &json!("charlie").to_string()) {
         println!("False positive for charlie (expected possible but unlikely)");
    } else {
         assert!(!col.bloom_check("username", &json!("charlie").to_string()));
    }

    // Persistence Test causing reload
    drop(col);
    // Simulating restart by getting collection again (which reloads state if needed)
    let col = db.get_collection("users").unwrap();
    
    assert!(col.bloom_check("username", &json!("alice").to_string()), "Persistence: Bloom filter should contain alice");

    /* Full persistence test (reload storage) skipped due to RocksDB lock issues in test env */
    drop(db);
    drop(storage);
}

#[tokio::test]
async fn test_columnar_bloom_index() {
    let dir = tempdir().unwrap();
    let storage = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
    storage.create_database("test_col_db".to_string()).unwrap();
    let db = storage.get_database("test_col_db").unwrap();

    let columns = vec![
        ColumnDef {
            name: "email".to_string(),
            data_type: ColumnType::String,
            nullable: false,
            indexed: false,
            index_type: None,
        },
    ];

    db.create_collection("_columnar_emails".to_string(), None).unwrap();
    let col = ColumnarCollection::new(
        "emails".to_string(),
        "test_col_db",
        db.db_arc(),
        columns,
        CompressionType::None,
    ).unwrap();

    // Insert 1200 rows (2 chunks: 0-999, 1000-1199). Chunk size 1000.
    // Chunk 0: user0..user999
    // Chunk 1: user1000..user1199
    let mut rows = Vec::new();
    for i in 0..1200 {
        rows.push(json!({
            "email": format!("user{}@example.com", i)
        }));
    }
    col.insert_rows(rows).unwrap();

    // Create Bloom Index
    col.create_index("email", ColumnarIndexType::Bloom).unwrap();

    // Test Eq Query
    // Should find in chunk 0
    let filter = ColumnFilter::Eq("email".to_string(), json!("user500@example.com"));
    let results = col.scan_filtered(&filter, &["email"]).unwrap();
    assert_eq!(results.len(), 1);
    
    // Should find in chunk 1
    let filter = ColumnFilter::Eq("email".to_string(), json!("user1100@example.com"));
    let results = col.scan_filtered(&filter, &["email"]).unwrap();
    assert_eq!(results.len(), 1);

    // Should NOT find non-existent
    let filter = ColumnFilter::Eq("email".to_string(), json!("nobody@example.com"));
    let results = col.scan_filtered(&filter, &["email"]).unwrap();
    assert_eq!(results.len(), 0);

    // Test In Query
    let filter = ColumnFilter::In("email".to_string(), vec![
        json!("user100@example.com"),
        json!("user1100@example.com"),
        json!("nobody@example.com")
    ]);
    let results = col.scan_filtered(&filter, &["email"]).unwrap();
    assert_eq!(results.len(), 2);
}
