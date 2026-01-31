use solidb::storage::columnar::{ColumnDef, ColumnType, ColumnarCollection, CompressionType};

use rocksdb::DB;
use solidb::sdbql::{parse, QueryExecutor};
use solidb::storage::engine::StorageEngine;
use std::sync::{Arc, RwLock};
use tempfile::TempDir;

#[allow(dead_code)]
fn create_test_db() -> (Arc<RwLock<DB>>, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db = DB::open_default(temp_dir.path()).unwrap();
    (Arc::new(RwLock::new(db)), temp_dir)
}

#[test]
fn test_explain_columnar_query() {
    let _dir = TempDir::new().unwrap();

    // Create StorageEngine wrapper (this opens the DB)
    let storage = StorageEngine::new(_dir.path()).unwrap();
    // database.db_arc() will provide the handle

    // Check if StorageEngine has a method to get the DB Arc.
    // Step 393 showed `db: Arc<RwLock<DB>>` is a field, but no public getter was visible in the snippet.
    // However, Database struct has `db_arc()`.
    // StorageEngine usually has `get_database`.

    // Let's create the database first
    storage.create_database("testdb".to_string()).unwrap();
    let database = storage.get_database("testdb").unwrap();
    let db_arc = database.db_arc(); // valid way to get it

    // Create a regular collection first (required for executor to find the collection)
    // The collection name must match the name used in the query ("metrics")
    database
        .create_collection("metrics".to_string(), None)
        .unwrap();

    // Manually create the column family for the columnar collection
    // Using unsafe pattern similar to Database::create_collection
    {
        let cf_name = "testdb:_columnar_metrics";
        // Check if exists first to be safe, though unexpected in fresh db
        if db_arc.cf_handle(cf_name).is_none() {
            let db_ptr = Arc::as_ptr(&db_arc) as *mut DB;
            unsafe {
                (*db_ptr)
                    .create_cf(cf_name, &rocksdb::Options::default())
                    .unwrap();
            }
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
