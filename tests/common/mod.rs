#![allow(dead_code)]

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
    let query = parse(query_str).unwrap_or_else(|_| panic!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .execute(&query)
        .unwrap_or_else(|_| panic!("Failed to execute: {}", query_str))
}

pub fn execute_single(engine: &StorageEngine, query_str: &str) -> Value {
    let results = execute_query(engine, query_str);
    results.into_iter().next().unwrap_or(Value::Null)
}

pub fn execute_with_binds(engine: &StorageEngine, query_str: &str, binds: BindVars) -> Vec<Value> {
    let query = parse(query_str).unwrap_or_else(|_| panic!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::with_bind_vars(engine, binds);
    executor
        .execute(&query)
        .unwrap_or_else(|_| panic!("Failed to execute: {}", query_str))
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
    let query = parse(query_str).unwrap_or_else(|_| panic!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::with_database(engine, db_name.to_string());
    executor
        .execute(&query)
        .unwrap_or_else(|_| panic!("Failed to execute: {}", query_str))
}

pub fn execute_with_db_and_binds(
    engine: &StorageEngine,
    db_name: &str,
    query_str: &str,
    binds: BindVars,
) -> Vec<Value> {
    let query = parse(query_str).unwrap_or_else(|_| panic!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::with_database_and_bind_vars(engine, db_name.to_string(), binds);
    executor
        .execute(&query)
        .unwrap_or_else(|_| panic!("Failed to execute: {}", query_str))
}

pub fn explain_query(engine: &StorageEngine, query_str: &str) -> solidb::sdbql::QueryExplain {
    let query = parse(query_str).unwrap_or_else(|_| panic!("Failed to parse: {}", query_str));
    let executor = QueryExecutor::new(engine);
    executor
        .explain(&query)
        .unwrap_or_else(|_| panic!("Failed to explain: {}", query_str))
}

pub fn execute_query_expect_err(engine: &StorageEngine, query_str: &str) -> String {
    let query = parse(query_str).unwrap_or_else(|_| panic!("Failed to parse: {}", query_str));
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

// ---------------------------------------------------------------------------
// Authentication fixtures
// ---------------------------------------------------------------------------

/// Seed a user into `_system:_admins` and give them `roles` in
/// `_system:_user_roles`, then mint a JWT for them.
///
/// Handler tests used to mint a token for a name that existed nowhere. That
/// worked because the auth middleware accepted any signed token, which is the
/// same reason a *deleted* user's token kept working for the rest of its
/// 24-hour lifetime — `refresh_jwt_roles` now rejects a subject that is not an
/// `_admins` row, so a test principal has to be a real one.
///
/// The password hash is a placeholder: these tests present the JWT directly
/// and never go through `/auth/login`.
pub fn seed_user_token(engine: &StorageEngine, username: &str, roles: &[&str]) -> String {
    let db = match engine.get_database("_system") {
        Ok(db) => db,
        Err(_) => {
            // Some suites build a router without calling `engine.initialize()`.
            engine
                .create_database("_system".to_string())
                .expect("create _system database");
            engine.get_database("_system").expect("_system database")
        }
    };

    let admins = db
        .get_or_create_system_collection("_admins")
        .expect("_admins collection");
    if admins.get(username).is_err() {
        admins
            .insert(serde_json::json!({
                "_key": username,
                "password_hash": "$argon2id$v=19$m=19456,t=2,p=1$dGVzdHNhbHQ$00000000000000000000000000000000",
            }))
            .expect("seed _admins row");
    }

    let user_roles = db
        .get_or_create_system_collection("_user_roles")
        .expect("_user_roles collection");
    for role in roles {
        user_roles
            .insert(serde_json::json!({
                "_key": format!("{}:{}", username, role),
                "id": format!("{}:{}", username, role),
                "username": username,
                "role": role,
                "assigned_at": "2026-01-01T00:00:00Z",
                "assigned_by": "test",
            }))
            .expect("seed _user_roles row");
    }

    // The role cache is process-wide and keyed by username; a previous test in
    // the same binary may have cached an empty list for this name.
    solidb::server::auth::AuthService::invalidate_user_roles_cache(username);

    let owned: Vec<String> = roles.iter().map(|r| r.to_string()).collect();
    solidb::server::auth::AuthService::create_jwt_with_roles(username, Some(owned), None)
        .expect("jwt")
}
