//! Verify Multiple Sort Fields
//!
//! Verifies SDBQL support for sorting by multiple fields with mixed directions.

use solidb::storage::StorageEngine;
use solidb::sdbql::QueryExecutor;
use solidb::parse;
use serde_json::json;
use tempfile::TempDir;

fn create_seeded_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    
    engine.create_database("testdb".to_string()).unwrap();
    let db = engine.get_database("testdb").unwrap();
    
    db.create_collection("items".to_string(), None).unwrap();
    let items = db.get_collection("items").unwrap();
    
    // Insert test data
    // Group 1: Scores 10, 20
    // Group 2: Scores 10, 30
    items.insert(json!({"name": "A", "group": 1, "score": 10})).unwrap();
    items.insert(json!({"name": "B", "group": 1, "score": 20})).unwrap();
    items.insert(json!({"name": "C", "group": 2, "score": 10})).unwrap();
    items.insert(json!({"name": "D", "group": 2, "score": 30})).unwrap();
    
    (engine, tmp_dir)
}

#[test]
fn test_multiple_sort_fields() {
    let (engine, _tmp) = create_seeded_engine();
    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());
    
    // Sort by Group ASC, Score DESC
    // Expected order: 
    // Group 1: B (20), A (10)
    // Group 2: D (30), C (10)
    let query_str = "FOR i IN items SORT i.group ASC, i.score DESC RETURN i.name";
    let query = parse(query_str).unwrap();
    
    let result = executor.execute(&query).unwrap();
    let names: Vec<String> = result.iter().map(|v| v.as_str().unwrap().to_string()).collect();
    
    assert_eq!(names, vec!["B", "A", "D", "C"]);
}

#[test]
fn test_multiple_sort_fields_mixed() {
    let (engine, _tmp) = create_seeded_engine();
    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());
    
    // Sort by Group DESC, Score ASC
    // Expected order:
    // Group 2: C (10), D (30)
    // Group 1: A (10), B (20)
    let query_str = "FOR i IN items SORT i.group DESC, i.score ASC RETURN i.name";
    let query = parse(query_str).unwrap();
    
    let result = executor.execute(&query).unwrap();
    let names: Vec<String> = result.iter().map(|v| v.as_str().unwrap().to_string()).collect();
    
    assert_eq!(names, vec!["C", "D", "A", "B"]);
}
