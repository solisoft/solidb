//! Index Optimization Tests
//!
//! Verifies that the Query Planner correctly utilizes indexes for optimization.

use solidb::storage::{StorageEngine, IndexType};
use solidb::sdbql::{QueryExecutor, QueryExplain};
use solidb::parse;
use serde_json::json;
use tempfile::TempDir;

fn create_seeded_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    
    engine.create_database("testdb".to_string()).unwrap();
    let db = engine.get_database("testdb").unwrap();
    
    db.create_collection("users".to_string(), None).unwrap();
    let users = db.get_collection("users").unwrap();
    
    // Insert 100 users
    for i in 0..100 {
        users.insert(json!({
            "name": format!("User{}", i),
            "age": i,
            "city": if i % 2 == 0 { "Paris" } else { "London" }
        })).unwrap();
    }
    
    (engine, tmp_dir)
}

#[test]
fn test_index_usage_equality() {
    let (engine, _tmp) = create_seeded_engine();
    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());
    
    let query_str = "FOR u IN users FILTER u.age == 50 RETURN u";
    let query = parse(query_str).unwrap();
    
    // 1. Before Index: Should be Full Scan
    let explain = executor.explain(&query).unwrap();
    let access = &explain.collections[0];
    assert_eq!(access.name, "users");
    assert_eq!(access.access_type, "full_scan");
    assert!(access.index_used.is_none());
    
    // 2. Create Index on 'age'
    let db = engine.get_database("testdb").unwrap();
    let users = db.get_collection("users").unwrap();
    users.create_index("age_idx".to_string(), vec!["age".to_string()], IndexType::Hash, false).unwrap();
    
    // 3. After Index: Should be Index Lookup
    let explain = executor.explain(&query).unwrap();
    let access = &explain.collections[0];
    assert_eq!(access.access_type, "index_lookup", "Query should use index after creation");
    assert!(access.index_used.is_some());
    assert!(access.index_used.as_ref().unwrap().contains("age"));
}

#[test]
fn test_index_usage_composite() {
    let (engine, _tmp) = create_seeded_engine();
    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());
    
    // Query filtering on two fields
    let query_str = "FOR u IN users FILTER u.city == 'Paris' AND u.age == 10 RETURN u";
    let query = parse(query_str).unwrap();
    
    // 1. Before Index
    let explain = executor.explain(&query).unwrap();
    assert_eq!(explain.collections[0].access_type, "full_scan");
    
    // 2. Create Composite Index on [city, age]
    let db = engine.get_database("testdb").unwrap();
    let users = db.get_collection("users").unwrap();
    users.create_index("city_age_idx".to_string(), vec!["city".to_string(), "age".to_string()], IndexType::Hash, false).unwrap();
    
    // 3. After Index
    let explain = executor.explain(&query).unwrap();
    let access = &explain.collections[0];
    
    // Note: SoliDB optimizer needs to be smart enough to pick composite index
    // If it doesn't support composite optimization yet, this test will fail (or show full_scan).
    // Assuming standard behavior for verifying optimization.
    assert_eq!(access.access_type, "index_lookup", "Should use composite index");
}

#[test]
fn test_index_usage_unique_constraint() {
    let (engine, _tmp) = create_seeded_engine();
    
    let db = engine.get_database("testdb").unwrap();
    let users = db.get_collection("users").unwrap();
    
    // Create Unique Index on 'name'
    users.create_index("name_idx".to_string(), vec!["name".to_string()], IndexType::Hash, true).unwrap();
    
    let executor = QueryExecutor::with_database(&engine, "testdb".to_string());
    let query_str = "FOR u IN users FILTER u.name == 'User50' RETURN u";
    let query = parse(query_str).unwrap();
    
    let explain = executor.explain(&query).unwrap();
    assert_eq!(explain.collections[0].access_type, "index_lookup");
}
