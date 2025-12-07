use solidb::storage::engine::StorageEngine;
use solidb::aql::executor::QueryExecutor;
use solidb::aql::parser::Parser;
use solidb::storage::index::IndexType;
use tempfile::TempDir;
use serde_json::Value;

#[tokio::test]
async fn test_numeric_sort_order() {
    let temp_dir = TempDir::new().unwrap();
    let storage = StorageEngine::new(temp_dir.path().to_str().unwrap()).unwrap();
    
    // Create collection
    storage.create_collection("dates".to_string()).unwrap();
    let collection = storage.get_collection("dates").unwrap();
    
    // Create index
    collection.create_index("idx1".to_string(), "val".to_string(), IndexType::Persistent, false).unwrap();
    
    // Insert data: 1, 2, 10
    collection.insert(serde_json::json!({"val": 1})).unwrap();
    collection.insert(serde_json::json!({"val": 2})).unwrap();
    collection.insert(serde_json::json!({"val": 10})).unwrap();
    
    // Query with SORT
    let query_str = "FOR d IN dates SORT d.val ASC LIMIT 10 RETURN d.val";
    let mut parser = Parser::new(query_str).expect("Failed to create parser");
    let query = parser.parse().unwrap();
    let executor = QueryExecutor::new(&storage);
    
    let results = executor.execute(&query).unwrap();
    
    println!("Results: {:?}", results);
    
    // Assert 1, 2, 10
    assert_eq!(results[0], Value::from(1), "First should be 1");
    // With new binary encoding, 2 should come after 1, and 10 after 2.
    assert_eq!(results[1], Value::from(2), "Second should be 2");
    assert_eq!(results[2], Value::from(10), "Third should be 10");
}
