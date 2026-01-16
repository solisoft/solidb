use serde_json::{json, Value};
use solidb::sdbql::executor::QueryExecutor;
use solidb::sdbql::parser::Parser;
use solidb::storage::document::Document;
use solidb::storage::engine::StorageEngine;
use std::sync::Arc;
use tempfile::tempdir;

fn setup_test_db() -> (Arc<StorageEngine>, String) {
    let dir = tempdir().unwrap();
    let engine = StorageEngine::new(dir.path()).unwrap();
    engine.initialize().unwrap();
    (Arc::new(engine), dir.path().to_str().unwrap().to_string())
}

#[test]
fn test_create_materialized_view_basic() {
    let (engine, _dir) = setup_test_db();

    // Create source collection and data
    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    users.insert(json!({"name": "Alice", "age": 30})).unwrap();
    users.insert(json!({"name": "Bob", "age": 25})).unwrap();
    users.insert(json!({"name": "Charlie", "age": 35})).unwrap();

    // Create View
    let query_str =
        "CREATE MATERIALIZED VIEW older_users AS FOR u IN users FILTER u.age >= 30 RETURN u";
    let mut parser = Parser::new(query_str).unwrap();
    let query = parser.parse().unwrap();
    let executor = QueryExecutor::new(&engine);
    let result = executor.execute(&query).unwrap();

    assert!(!result.is_empty());
    assert_eq!(
        result[0],
        Value::String("Materialized view 'older_users' created".to_string())
    );

    // Verify _views metadata
    let views = engine.get_collection("_system:_views").unwrap();
    let meta = views.get("older_users").unwrap();
    assert_eq!(meta.get("_key").unwrap(), "older_users");
    assert_eq!(meta.get("type").unwrap(), "materialized");

    // Verify View Data
    // View creation uses _system prefix if no DB.
    // The executor logic: if db is None -> _system.
    // Create view name: _system:older_users
    let view = engine.get_collection("_system:older_users").unwrap();
    assert_eq!(view.count(), 2);

    let docs = view.scan(None);
    let names: Vec<String> = docs
        .iter()
        .map(|d: &Document| d.get("name").unwrap().as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"Alice".to_string()));
    assert!(names.contains(&"Charlie".to_string()));
    assert!(!names.contains(&"Bob".to_string())); // Bob is 25
}

#[test]
fn test_refresh_materialized_view() {
    let (engine, _dir) = setup_test_db();

    // Setup
    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();
    products.insert(json!({"name": "A", "price": 100})).unwrap();

    let executor = QueryExecutor::new(&engine);

    // Create View
    let create_q = "CREATE MATERIALIZED VIEW expensive_products AS FOR p IN products FILTER p.price > 50 RETURN p";
    let mut parser = Parser::new(create_q).unwrap();
    let query = parser.parse().unwrap();
    executor.execute(&query).unwrap();

    let view = engine.get_collection("_system:expensive_products").unwrap();
    assert_eq!(view.count(), 1);

    // Update Source Data
    products.insert(json!({"name": "B", "price": 200})).unwrap();
    products.insert(json!({"name": "C", "price": 10})).unwrap(); // Should be filtered out

    // Verify View is stale
    assert_eq!(view.count(), 1);

    // Refresh View
    let refresh_q = "REFRESH MATERIALIZED VIEW expensive_products";
    let mut parser = Parser::new(refresh_q).unwrap();
    let query = parser.parse().unwrap();
    let result = executor.execute(&query).unwrap();

    assert_eq!(
        result[0],
        Value::String("Materialized view 'expensive_products' refreshed".to_string())
    );

    // Verify View is updated
    // count might need recalculation if not auto? insert_batch updates count.
    // truncate updates count?
    // Let's verify data directly.
    assert_eq!(view.count(), 2);

    let docs = view.scan(None);
    let names: Vec<String> = docs
        .iter()
        .map(|d: &Document| d.get("name").unwrap().as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"A".to_string()));
    assert!(names.contains(&"B".to_string()));
    assert!(!names.contains(&"C".to_string()));
}
