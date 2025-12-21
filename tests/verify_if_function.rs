use serde_json::json;
use solidb::{parse, QueryExecutor, StorageEngine};
use tempfile::TempDir;

fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

#[test]
fn test_if_function() {
    let (storage, _dir) = create_test_storage();
    storage.create_collection("test_col".to_string(), None).unwrap();
    let col = storage.get_collection("test_col").unwrap();

    col.insert(json!({ "val": 10 })).unwrap();
    col.insert(json!({ "val": 0 })).unwrap();
    col.insert(json!({ "val": null })).unwrap();

    // Test IF with different conditions
    let query_str = "FOR d IN test_col RETURN IF(d.val > 0, 'positive', 'non-positive')";
    let query = parse(query_str).unwrap();
    let executor = QueryExecutor::new(&storage);
    let result = executor.execute(&query).unwrap();

    let results: Vec<String> = result
        .into_iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    assert!(results.contains(&"positive".to_string()));
    assert!(results.contains(&"non-positive".to_string()));
    
    // Check specific values
    // val: 10 -> positive
    // val: 0 -> non-positive
    // val: null -> non-positive
    
    // Test truthiness (null/0 is false, other numbers true)
    let query_count_str = "FOR d IN test_col RETURN IF(d.val, 'truthy', 'falsy')";
    let query_count = parse(query_count_str).unwrap();
    let result_count = executor.execute(&query_count).unwrap();
    
    let results_count: Vec<String> = result_count
        .into_iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
        
    // 10 -> truthy
    // 0 -> falsy
    // null -> falsy
    assert_eq!(results_count.iter().filter(|&r| r == "truthy").count(), 1);
    assert_eq!(results_count.iter().filter(|&r| r == "falsy").count(), 2);
}
