//! Verify COLLECT aggregation in SDBQL
//!
//! Run with: cargo test --test verify_collect

use serde_json::json;
use solidb::{parse, QueryExecutor, StorageEngine};
use tempfile::TempDir;

fn create_test_storage() -> (StorageEngine, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    (storage, temp_dir)
}

fn setup_test_data(storage: &StorageEngine) {
    // Create users collection with various cities
    let _ = storage.create_collection("users".to_string(), None);
    let users = storage.get_collection("users").unwrap();
    
    users.insert(json!({"name": "Alice", "city": "Paris", "age": 30, "salary": 50000})).unwrap();
    users.insert(json!({"name": "Bob", "city": "London", "age": 25, "salary": 45000})).unwrap();
    users.insert(json!({"name": "Carol", "city": "Paris", "age": 35, "salary": 60000})).unwrap();
    users.insert(json!({"name": "David", "city": "London", "age": 40, "salary": 70000})).unwrap();
    users.insert(json!({"name": "Eve", "city": "Berlin", "age": 28, "salary": 55000})).unwrap();
}

// ==================== Basic Grouping ====================

#[test]
fn test_collect_basic_grouping() {
    let (storage, _dir) = create_test_storage();
    setup_test_data(&storage);

    let query = parse("FOR doc IN users COLLECT city = doc.city RETURN city").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Should have 3 distinct cities
    assert_eq!(results.len(), 3);
    
    let cities: Vec<&str> = results.iter()
        .filter_map(|v| v.as_str())
        .collect();
    
    assert!(cities.contains(&"Paris"));
    assert!(cities.contains(&"London"));
    assert!(cities.contains(&"Berlin"));
}

// ==================== WITH COUNT INTO ====================

#[test]
fn test_collect_with_count() {
    let (storage, _dir) = create_test_storage();
    setup_test_data(&storage);

    let query = parse("FOR doc IN users COLLECT city = doc.city WITH COUNT INTO cnt RETURN { city, cnt }").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Should have 3 groups
    assert_eq!(results.len(), 3);
    
    for result in &results {
        let city = result.get("city").and_then(|v| v.as_str()).unwrap();
        let cnt = result.get("cnt").and_then(|v| v.as_i64()).unwrap();
        
        match city {
            "Paris" => assert_eq!(cnt, 2),
            "London" => assert_eq!(cnt, 2),
            "Berlin" => assert_eq!(cnt, 1),
            _ => panic!("Unexpected city: {}", city),
        }
    }
}

// ==================== INTO variable ====================

#[test]
fn test_collect_into_groups() {
    let (storage, _dir) = create_test_storage();
    setup_test_data(&storage);

    let query = parse("FOR doc IN users COLLECT city = doc.city INTO groups RETURN { city, users: LENGTH(groups) }").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Should have 3 groups
    assert_eq!(results.len(), 3);
    
    for result in &results {
        let city = result.get("city").and_then(|v| v.as_str()).unwrap();
        let user_count = result.get("users").and_then(|v| v.as_i64()).unwrap();
        
        match city {
            "Paris" => assert_eq!(user_count, 2),
            "London" => assert_eq!(user_count, 2),
            "Berlin" => assert_eq!(user_count, 1),
            _ => panic!("Unexpected city: {}", city),
        }
    }
}

// ==================== AGGREGATE ====================

#[test]
fn test_collect_aggregate_sum() {
    let (storage, _dir) = create_test_storage();
    setup_test_data(&storage);

    let query = parse("FOR doc IN users COLLECT city = doc.city AGGREGATE totalSalary = SUM(doc.salary) RETURN { city, totalSalary }").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    assert_eq!(results.len(), 3);
    
    for result in &results {
        let city = result.get("city").and_then(|v| v.as_str()).unwrap();
        let total = result.get("totalSalary").and_then(|v| v.as_f64()).unwrap() as i64;
        
        match city {
            "Paris" => assert_eq!(total, 110000), // 50000 + 60000
            "London" => assert_eq!(total, 115000), // 45000 + 70000
            "Berlin" => assert_eq!(total, 55000),
            _ => panic!("Unexpected city: {}", city),
        }
    }
}

#[test]
fn test_collect_aggregate_avg() {
    let (storage, _dir) = create_test_storage();
    setup_test_data(&storage);

    let query = parse("FOR doc IN users COLLECT city = doc.city AGGREGATE avgAge = AVG(doc.age) RETURN { city, avgAge }").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    for result in &results {
        let city = result.get("city").and_then(|v| v.as_str()).unwrap();
        let avg = result.get("avgAge").and_then(|v| v.as_f64()).unwrap();
        
        match city {
            "Paris" => assert!((avg - 32.5).abs() < 0.1), // (30 + 35) / 2
            "London" => assert!((avg - 32.5).abs() < 0.1), // (25 + 40) / 2
            "Berlin" => assert!((avg - 28.0).abs() < 0.1),
            _ => panic!("Unexpected city: {}", city),
        }
    }
}

#[test]
fn test_collect_aggregate_min_max() {
    let (storage, _dir) = create_test_storage();
    setup_test_data(&storage);

    let query = parse("FOR doc IN users COLLECT city = doc.city AGGREGATE minAge = MIN(doc.age), maxAge = MAX(doc.age) RETURN { city, minAge, maxAge }").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    for result in &results {
        let city = result.get("city").and_then(|v| v.as_str()).unwrap();
        let min_age = result.get("minAge").and_then(|v| v.as_i64()).unwrap();
        let max_age = result.get("maxAge").and_then(|v| v.as_i64()).unwrap();
        
        match city {
            "Paris" => {
                assert_eq!(min_age, 30);
                assert_eq!(max_age, 35);
            }
            "London" => {
                assert_eq!(min_age, 25);
                assert_eq!(max_age, 40);
            }
            "Berlin" => {
                assert_eq!(min_age, 28);
                assert_eq!(max_age, 28);
            }
            _ => panic!("Unexpected city: {}", city),
        }
    }
}

// ==================== Multiple group variables ====================

#[test]
fn test_collect_multiple_group_vars() {
    let (storage, _dir) = create_test_storage();
    let _ = storage.create_collection("orders".to_string(), None);
    let orders = storage.get_collection("orders").unwrap();
    
    orders.insert(json!({"product": "A", "region": "US", "amount": 100})).unwrap();
    orders.insert(json!({"product": "A", "region": "EU", "amount": 150})).unwrap();
    orders.insert(json!({"product": "B", "region": "US", "amount": 200})).unwrap();
    orders.insert(json!({"product": "A", "region": "US", "amount": 120})).unwrap();

    let query = parse("FOR doc IN orders COLLECT product = doc.product, region = doc.region WITH COUNT INTO cnt RETURN { product, region, cnt }").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Should have 3 distinct (product, region) combinations
    assert_eq!(results.len(), 3);
}

// ==================== Empty collection ====================

#[test]
fn test_collect_empty_collection() {
    let (storage, _dir) = create_test_storage();
    let _ = storage.create_collection("empty".to_string(), None);

    let query = parse("FOR doc IN empty COLLECT city = doc.city WITH COUNT INTO cnt RETURN { city, cnt }").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Should return empty array for empty collection
    assert_eq!(results.len(), 0);
}

// ==================== With FILTER ====================

#[test]
fn test_collect_with_filter() {
    let (storage, _dir) = create_test_storage();
    setup_test_data(&storage);

    let query = parse("FOR doc IN users FILTER doc.age >= 30 COLLECT city = doc.city WITH COUNT INTO cnt RETURN { city, cnt }").unwrap();
    let executor = QueryExecutor::new(&storage);
    let results = executor.execute(&query).unwrap();

    // Only Paris (Alice 30, Carol 35), London (David 40), and nothing from Berlin (Eve 28)
    // So we should have Paris with count 2, London with count 1
    for result in &results {
        let city = result.get("city").and_then(|v| v.as_str()).unwrap();
        let cnt = result.get("cnt").and_then(|v| v.as_i64()).unwrap();
        
        match city {
            "Paris" => assert_eq!(cnt, 2),
            "London" => assert_eq!(cnt, 1),
            _ => panic!("Unexpected city: {} (Berlin should not appear)", city),
        }
    }
}
