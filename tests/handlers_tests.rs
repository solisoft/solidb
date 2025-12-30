//! Server Handlers Tests
//!
//! Comprehensive tests for server/handlers.rs including:
//! - Database CRUD operations
//! - Collection CRUD operations
//! - Document CRUD operations
//! - Query execution
//! - Index management
//! - Error handling
//! - Edge cases

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use solidb::storage::StorageEngine;
use solidb::server::routes::create_router;
use solidb::scripting::ScriptStats;
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt; // for oneshot

fn create_test_app() -> (axum::Router, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    
    let script_stats = Arc::new(ScriptStats::default());
    
    let router = create_router(
        engine,
        None, // ClusterManager
        None, // SyncLog
        None, // ShardCoordinator
        None, // QueueWorker
        script_stats,
        0 // port
    );
    
    (router, tmp_dir)
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024*1024).await.unwrap();
    serde_json::from_slice(&body).unwrap_or(json!({"error": "Invalid JSON"}))
}

async fn setup_db_and_collection(app: &axum::Router, db_name: &str, coll_name: &str) {
    // Create DB
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": db_name }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Create Collection
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri(&format!("/_api/database/{}/collection", db_name))
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": coll_name }).to_string()))
            .unwrap(),
    ).await.unwrap();
}

// ============================================================================
// Database Handler Tests
// ============================================================================

#[tokio::test]
async fn test_create_database() {
    let (app, _tmp) = create_test_app();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "newdb" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["name"], "newdb");
}

#[tokio::test]
async fn test_create_duplicate_database() {
    let (app, _tmp) = create_test_app();
    
    // Create first
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "dupdb" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Create duplicate
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "dupdb" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Should return conflict
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_list_databases() {
    let (app, _tmp) = create_test_app();
    
    // Create some DBs
    for name in ["db1", "db2", "db3"] {
        let _ = app.clone().oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "name": name }).to_string()))
                .unwrap(),
        ).await.unwrap();
    }
    
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/databases")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let dbs = json["databases"].as_array().unwrap();
    assert!(dbs.len() >= 3);
}

#[tokio::test]
async fn test_delete_database() {
    let (app, _tmp) = create_test_app();
    
    // Create DB
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "todelete" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Delete DB
    let response = app.clone().oneshot(
        Request::builder()
            .method("DELETE")
            .uri("/_api/database/todelete")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    
    // Verify deleted
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/databases")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    let json = response_json(response).await;
    let dbs: Vec<&str> = json["databases"].as_array().unwrap()
        .iter().map(|v| v.as_str().unwrap()).collect();
    assert!(!dbs.contains(&"todelete"));
}

#[tokio::test]
async fn test_delete_nonexistent_database() {
    let (app, _tmp) = create_test_app();
    
    let response = app.oneshot(
        Request::builder()
            .method("DELETE")
            .uri("/_api/database/nonexistent")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Collection Handler Tests
// ============================================================================

#[tokio::test]
async fn test_create_collection() {
    let (app, _tmp) = create_test_app();
    
    // Create DB first
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "testdb" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/testdb/collection")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "users" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["name"], "users");
}

#[tokio::test]
async fn test_create_edge_collection() {
    let (app, _tmp) = create_test_app();
    
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "graphdb" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/graphdb/collection")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "edges", "type": "edge" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    // Edge collections are created successfully
}

#[tokio::test]
async fn test_list_collections() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "mydb", "col1").await;
    
    // Add another collection
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/mydb/collection")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "col2" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Correct endpoint is GET /_api/database/{db}/collection (not /collections)
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/database/mydb/collection")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let collections = json["collections"].as_array().unwrap();
    assert!(collections.len() >= 2);
}

#[tokio::test]
async fn test_delete_collection() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "deldb", "todel").await;
    
    let response = app.oneshot(
        Request::builder()
            .method("DELETE")
            .uri("/_api/database/deldb/collection/todel")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_collection_in_nonexistent_db() {
    let (app, _tmp) = create_test_app();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/nodb/collection")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "col" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_truncate_collection() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "truncdb", "data").await;
    
    // Insert some documents
    for i in 0..5 {
        let _ = app.clone().oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/truncdb/document/data")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "num": i }).to_string()))
                .unwrap(),
        ).await.unwrap();
    }
    
    // Truncate
    let response = app.clone().oneshot(
        Request::builder()
            .method("PUT")
            .uri("/_api/database/truncdb/collection/data/truncate")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    
    // Verify count is 0
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/truncdb/cursor")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "query": "FOR d IN data RETURN d" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    let json = response_json(response).await;
    assert_eq!(json["result"].as_array().unwrap().len(), 0);
}

// ============================================================================
// Document Handler Tests
// ============================================================================

#[tokio::test]
async fn test_insert_document() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "docdb", "items").await;
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/docdb/document/items")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "Widget", "price": 19.99 }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json["_key"].is_string());
}

#[tokio::test]
async fn test_insert_document_with_key() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "docdb", "items").await;
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/docdb/document/items")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "_key": "myitem", "name": "Custom" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["_key"], "myitem");
}

#[tokio::test]
async fn test_get_document() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "getdb", "items").await;
    
    // Insert
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/getdb/document/items")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "_key": "doc1", "value": 42 }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Get
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/database/getdb/document/items/doc1")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["value"], 42);
}

#[tokio::test]
async fn test_get_nonexistent_document() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "getdb", "items").await;
    
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/database/getdb/document/items/nonexistent")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_document() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "updb", "items").await;
    
    // Insert
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/updb/document/items")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "_key": "doc1", "value": 1 }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Update (PUT, not PATCH)
    let response = app.clone().oneshot(
        Request::builder()
            .method("PUT")
            .uri("/_api/database/updb/document/items/doc1")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "value": 100, "extra": "field" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    
    // Verify
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/database/updb/document/items/doc1")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    let json = response_json(response).await;
    assert_eq!(json["value"], 100);
}

#[tokio::test]
async fn test_delete_document() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "deldb", "items").await;
    
    // Insert
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/deldb/document/items")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "_key": "todelete", "val": 1 }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Delete
    let response = app.clone().oneshot(
        Request::builder()
            .method("DELETE")
            .uri("/_api/database/deldb/document/items/todelete")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    
    // Verify deleted
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/database/deldb/document/items/todelete")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Query Handler Tests
// ============================================================================

#[tokio::test]
async fn test_simple_return_query() {
    let (app, _tmp) = create_test_app();
    
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "qdb" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/qdb/cursor")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "query": "RETURN 1 + 2 + 3" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["result"][0], 6.0);
}

#[tokio::test]
async fn test_query_with_collection() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "qdb", "users").await;
    
    // Insert data
    for name in ["Alice", "Bob", "Charlie"] {
        let _ = app.clone().oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/qdb/document/users")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "name": name }).to_string()))
                .unwrap(),
        ).await.unwrap();
    }
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/qdb/cursor")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "query": "FOR u IN users RETURN u.name" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let results = json["result"].as_array().unwrap();
    assert_eq!(results.len(), 3);
}

#[tokio::test]
async fn test_query_with_bind_vars() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "qdb", "users").await;
    
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/qdb/document/users")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "TestUser", "age": 30 }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/qdb/cursor")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ 
                "query": "FOR u IN users FILTER u.name == @name RETURN u.age",
                "bindVars": { "name": "TestUser" }
            }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["result"][0], 30);
}

#[tokio::test]
async fn test_query_parse_error() {
    let (app, _tmp) = create_test_app();
    
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "qdb" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/qdb/cursor")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "query": "NOT VALID SDBQL" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_explain_query() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "edb", "users").await;
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/edb/explain")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "query": "FOR u IN users RETURN u" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json["collections"].is_array());
    assert!(json["timing"].is_object());
}

// ============================================================================
// Index Handler Tests
// ============================================================================

#[tokio::test]
async fn test_create_index() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "idb", "users").await;
    
    // Correct endpoint: /_api/database/{db}/index/{collection}
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/idb/index/users")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ 
                "name": "email_idx",
                "fields": ["email"],
                "unique": true
            }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["name"], "email_idx");
}

#[tokio::test]
async fn test_list_indexes() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "idb", "users").await;
    
    // Create index first using correct endpoint
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/idb/index/users")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ 
                "name": "test_idx",
                "fields": ["name"]
            }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Correct endpoint: GET /_api/database/{db}/index/{collection}
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/database/idb/index/users")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let indexes = json["indexes"].as_array().unwrap();
    assert!(!indexes.is_empty());
}

// ============================================================================
// Edge Cases and Error Handling
// ============================================================================

#[tokio::test]
async fn test_invalid_json_body() {
    let (app, _tmp) = create_test_app();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from("not valid json"))
            .unwrap(),
    ).await.unwrap();
    
    // Should return 422 or 400 for invalid JSON
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY || 
            response.status() == StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_missing_required_field() {
    let (app, _tmp) = create_test_app();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({}).to_string())) // Missing "name" field
            .unwrap(),
    ).await.unwrap();
    
    // Should return 422 or 400 for missing field
    assert!(response.status() == StatusCode::UNPROCESSABLE_ENTITY || 
            response.status() == StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_health_check() {
    let (app, _tmp) = create_test_app();
    
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/health")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["status"], "ok");
}

#[tokio::test]
async fn test_unicode_in_documents() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "unidb", "data").await;
    
    let response = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/unidb/document/data")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ 
                "_key": "unicode",
                "jp": "日本語",
                "emoji": "🎉",
                "special": "O'Brien \"quotes\""
            }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    
    // Verify retrieval
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/database/unidb/document/data/unicode")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    let json = response_json(response).await;
    assert_eq!(json["jp"], "日本語");
    assert_eq!(json["emoji"], "🎉");
}

#[tokio::test]
async fn test_large_document() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "largedb", "data").await;
    
    // Create a document with a large array
    let large_array: Vec<i32> = (0..1000).collect();
    
    let response = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/largedb/document/data")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ 
                "_key": "large",
                "data": large_array
            }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    
    // Verify
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/database/largedb/document/data/large")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    let json = response_json(response).await;
    assert_eq!(json["data"].as_array().unwrap().len(), 1000);
}

#[tokio::test]
async fn test_batch_query_response_metadata() {
    let (app, _tmp) = create_test_app();
    
    setup_db_and_collection(&app, "metadb", "items").await;
    
    // Insert several documents
    for i in 0..10 {
        let _ = app.clone().oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/metadb/document/items")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "num": i }).to_string()))
                .unwrap(),
        ).await.unwrap();
    }
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/metadb/cursor")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ 
                "query": "FOR i IN items RETURN i",
                "batchSize": 5
            }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    // Check that response has expected metadata
    assert!(json.get("result").is_some());
}
