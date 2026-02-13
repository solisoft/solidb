//! Common test utilities for SDBQL tests
//!
//! Provides shared helper functions for:
//! - Creating test storage engines
//! - Executing queries
//! - Working with bind variables and databases

use serde_json::Value;
use solidb::storage::StorageEngine;
use solidb::{parse, BindVars, QueryExecutor};
use tempfile::TempDir;

pub fn create_test_engine() -> (StorageEngine, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    (engine, tmp_dir)
}

pub fn create_seeded_engine() -> (StorageEngine, TempDir) {
    let (engine, tmp) = create_test_engine();

    engine.create_collection("users".to_string(), None).unwrap();
    let users = engine.get_collection("users").unwrap();
    users
        .insert(serde_json::json!({"_key": "alice", "name": "Alice", "age": 30, "dept": "eng"}))
        .unwrap();
    users
        .insert(serde_json::json!({"_key": "bob", "name": "Bob", "age": 25, "dept": "eng"}))
        .unwrap();
    users
        .insert(
            serde_json::json!({"_key": "charlie", "name": "Charlie", "age": 35, "dept": "sales"}),
        )
        .unwrap();
    users
        .insert(
            serde_json::json!({"_key": "diana", "name": "Diana", "age": 28, "dept": "marketing"}),
        )
        .unwrap();

    engine
        .create_collection("products".to_string(), None)
        .unwrap();
    let products = engine.get_collection("products").unwrap();
    products
        .insert(serde_json::json!({"_key": "p1", "name": "Widget", "price": 29.99, "category": "gadgets"}))
        .unwrap();
    products
        .insert(serde_json::json!({"_key": "p2", "name": "Gadget", "price": 49.99, "category": "gadgets"}))
        .unwrap();
    products
        .insert(
            serde_json::json!({"_key": "p3", "name": "Thing", "price": 19.99, "category": "misc"}),
        )
        .unwrap();

    engine
        .create_collection("orders".to_string(), None)
        .unwrap();
    let orders = engine.get_collection("orders").unwrap();
    orders
        .insert(serde_json::json!({"_key": "o1", "user_id": "alice", "total": 79.98, "status": "completed"}))
        .unwrap();
    orders
        .insert(serde_json::json!({"_key": "o2", "user_id": "bob", "total": 29.99, "status": "pending"}))
        .unwrap();
    orders
        .insert(serde_json::json!({"_key": "o3", "user_id": "alice", "total": 49.99, "status": "shipped"}))
        .unwrap();

    (engine, tmp)
}

pub fn execute_query(engine: &StorageEngine, query_str: &str) -> Vec<Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .execute(&query)
        .expect(&format!("Failed to execute: {}", query_str))
}

pub fn execute_single(engine: &StorageEngine, query_str: &str) -> Value {
    let results = execute_query(engine, query_str);
    results.into_iter().next().unwrap_or(Value::Null)
}

pub fn execute_with_binds(engine: &StorageEngine, query_str: &str, binds: BindVars) -> Vec<Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::with_bind_vars(engine, binds);
    executor
        .execute(&query)
        .expect(&format!("Failed to execute: {}", query_str))
}

pub fn execute_with_binds_single(
    engine: &StorageEngine,
    query_str: &str,
    binds: BindVars,
) -> Value {
    let results = execute_with_binds(engine, query_str, binds);
    results.into_iter().next().unwrap_or(Value::Null)
}

pub fn execute_with_database(engine: &StorageEngine, db_name: &str, query_str: &str) -> Vec<Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::with_database(engine, db_name.to_string());
    executor
        .execute(&query)
        .expect(&format!("Failed to execute: {}", query_str))
}

pub fn execute_with_db_and_binds(
    engine: &StorageEngine,
    db_name: &str,
    query_str: &str,
    binds: BindVars,
) -> Vec<Value> {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::with_database_and_bind_vars(engine, db_name.to_string(), binds);
    executor
        .execute(&query)
        .expect(&format!("Failed to execute: {}", query_str))
}

pub fn explain_query(engine: &StorageEngine, query_str: &str) -> solidb::sdbql::QueryExplain {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .explain(&query)
        .expect(&format!("Failed to explain: {}", query_str))
}

pub fn execute_query_expect_err(engine: &StorageEngine, query_str: &str) -> String {
    let query = parse(query_str).expect(&format!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    match executor.execute(&query) {
        Ok(_) => panic!("Expected error but query succeeded"),
        Err(e) => e.to_string(),
    }
}

pub fn create_collection_with_data(
    engine: &StorageEngine,
    collection: &str,
    data: Vec<serde_json::Value>,
) {
    engine
        .create_collection(collection.to_string(), None)
        .unwrap();
    let coll = engine.get_collection(collection).unwrap();
    for doc in data {
        coll.insert(doc).unwrap();
    }
}
