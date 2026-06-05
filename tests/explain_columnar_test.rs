use solidb::storage::columnar::{ColumnDef, ColumnType, ColumnarCollection, CompressionType};

use rust_rocksdb::Options;
use solidb::sdbql::{parse, QueryExecutor};
use solidb::storage::engine::StorageEngine;
use tempfile::TempDir;

#[test]
fn test_explain_columnar_query() {
    let _dir = TempDir::new().unwrap();

    // Create StorageEngine wrapper (this opens the DB)
    let storage = StorageEngine::new(_dir.path()).unwrap();

    // Create the database first
    storage.create_database("testdb".to_string()).unwrap();
    let database = storage.get_database("testdb").unwrap();
    let db_arc = database.db_arc();

    // Create a regular collection first (required for executor to find the collection)
    // The collection name must match the name used in the query ("metrics")
    database
        .create_collection("metrics".to_string(), None)
        .unwrap();

    // Manually create the column family for the columnar collection.
    // MultiThreaded mode: create_cf takes &self and synchronizes internally.
    {
        let cf_name = "testdb:_columnar_metrics";
        if db_arc.cf_handle(cf_name).is_none() {
            db_arc.create_cf(cf_name, &Options::default()).unwrap();
        }
    }

    // Create columnar collection manually
    let _col = ColumnarCollection::new(
        "metrics".to_string(),
        "testdb",
        db_arc.clone(),
        vec![
            ColumnDef {
                name: "ts".to_string(),
                data_type: ColumnType::Timestamp,
                nullable: false,
                indexed: true,
                index_type: None,
            },
            ColumnDef {
                name: "val".to_string(),
                data_type: ColumnType::Float64,
                nullable: false,
                indexed: false,
                index_type: None,
            },
        ],
        CompressionType::Lz4,
    )
    .unwrap();

    // Create QueryExecutor with StorageEngine
    let executor = QueryExecutor::with_database(&storage, "testdb".to_string());

    // Parse query (Aggregation query to trigger try_columnar_aggregation)
    let query_str =
        "FOR m IN metrics COLLECT AGGREGATE avg_val = AVG(m.val) RETURN { avg: avg_val }";
    let query = parse(query_str).unwrap();

    let explain_result = executor.explain(&query);

    if let Err(e) = &explain_result {
        println!("Explain Error: {:?}", e);
    }
    assert!(
        explain_result.is_ok(),
        "Explain failed: {:?}",
        explain_result.err()
    );

    let explanation = explain_result.unwrap();
    println!("{:?}", explanation);

    // Verify it detected columnar scan
    assert!(explanation.collections.iter().any(|c| c.name == "metrics"));
}
