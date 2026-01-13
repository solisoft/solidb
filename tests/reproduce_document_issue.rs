use serde_json::json;
use solidb::storage::StorageEngine;
use solidb::{parse, QueryExecutor};
use tempfile::TempDir;

fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

#[test]
fn test_document_cross_db_reference_failure() {
    let (engine, _tmp) = create_test_engine();

    // Setup: Create db1 and col1, insert a document
    engine.create_database("db1".to_string()).unwrap();
    let db1 = engine.get_database("db1").unwrap();
    db1.create_collection("col1".to_string(), None).unwrap();
    let col1 = db1.get_collection("col1").unwrap();
    
    col1.insert(json!({"_key": "k1", "value": "v1"})).unwrap();

    // Case 1: No DB context (Global executor) - Should work (or fail if my understanding of StorageEngine is wrong)
    // Actually StorageEngine::get_collection("db1:col1") should work.
    let query_str = "RETURN DOCUMENT('db1:col1/k1')";
    let query = parse(query_str).unwrap();
    let executor = QueryExecutor::new(&engine);
    let result = executor.execute(&query).unwrap();
    
    assert_eq!(result[0]["value"], json!("v1"), "Global executor should find the document");

    // Case 2: Inside db1 context - Should work now with the fix
    let executor_db1 = QueryExecutor::with_database(&engine, "db1".to_string());
    let result_db1 = executor_db1.execute(&query).unwrap();
    assert_eq!(result_db1[0]["value"], json!("v1"), "Executor in db1 context should find document via absolute ID");

    // Case 3: Inside ANOTHER db context (db2) - accessing db1:col1
    engine.create_database("db2".to_string()).unwrap();
    let executor_db2 = QueryExecutor::with_database(&engine, "db2".to_string());
    let result_db2 = executor_db2.execute(&query).unwrap();
    assert_eq!(result_db2[0]["value"], json!("v1"), "Executor in db2 context should find document via absolute ID");

}
