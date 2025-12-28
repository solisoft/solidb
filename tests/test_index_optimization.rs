use serde_json::json;
use solidb::{parse, QueryExecutor, StorageEngine};
use std::time::Instant;
use tempfile::TempDir;

#[test]
fn test_index_optimization_performance() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");

    storage.create_collection("users".to_string(), None).unwrap();
    let collection = storage.get_collection("users").unwrap();

    // Create index on age field
    collection
        .create_index(
            "idx_age".to_string(),
            vec!["age".to_string()],
            solidb::IndexType::Persistent,
            false,
        )
        .unwrap();

    // Insert 1000 documents
    for i in 0..1000 {
        collection
            .insert(json!({
                "name": format!("User{}", i),
                "age": i % 100  // Ages 0-99
            }))
            .unwrap();
    }

    // Query with index (should be fast)
    let query = parse("FOR u IN users FILTER u.age == 30 LIMIT 1 RETURN u").unwrap();
    let executor = QueryExecutor::new(&storage);

    let start = Instant::now();
    let results = executor.execute(&query).unwrap();
    let duration = start.elapsed();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["age"].as_f64().unwrap() as i64, 30);

    // Should be very fast (< 10ms even on slow machines)
    println!("Query with index took: {:?}", duration);
    assert!(
        duration.as_millis() < 50,
        "Query took too long: {:?}",
        duration
    );
}

#[test]
fn test_index_optimization_correctness() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");

    storage.create_collection("users".to_string(), None).unwrap();
    let collection = storage.get_collection("users").unwrap();

    // Create index
    collection
        .create_index(
            "idx_age".to_string(),
            vec!["age".to_string()],
            solidb::IndexType::Persistent,
            false,
        )
        .unwrap();

    // Insert test data
    collection
        .insert(json!({"name": "Alice", "age": 25}))
        .unwrap();
    collection
        .insert(json!({"name": "Bob", "age": 30}))
        .unwrap();
    collection
        .insert(json!({"name": "Charlie", "age": 30}))
        .unwrap();
    collection
        .insert(json!({"name": "David", "age": 35}))
        .unwrap();

    // Query should return Bob or Charlie (both age 30)
    let query = parse("FOR u IN users FILTER u.age == 30 RETURN u.name").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 2);
    let names: Vec<String> = results
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"Bob".to_string()));
    assert!(names.contains(&"Charlie".to_string()));
}
