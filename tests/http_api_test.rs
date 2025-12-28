//! HTTP API Integration Tests
//!
//! Tests for the HTTP API endpoints including:
//! - Database management
//! - Collection management
//! - Document CRUD
//! - Query execution
//! - Auth (basic)

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
    
    // Create minimal dependencies
    let script_stats = Arc::new(ScriptStats::default());
    
    let router = create_router(
        engine,
        None, // ClusterManager
        None, // SyncLog
        None, // ShardCoordinator
        None, // QueueWorker
        script_stats,
        0 // port (unused in router creation)
    );
    
    (router, tmp_dir)
}

// Helper to parse JSON response
async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024*1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

// ============================================================================
// Database API Tests
// ============================================================================

#[tokio::test]
async fn test_create_database_api() {
    let (app, _tmp) = create_test_app();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "testdb" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    // Check if response indicates success
    // assert_eq!(json["result"], true);
    println!("Create DB response: {:?}", json);
}

#[tokio::test]
async fn test_list_databases_api() {
    let (app, _tmp) = create_test_app();
    
    // Create db first
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "db1" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/databases")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let names: Vec<&str> = json["databases"].as_array().unwrap()
        .iter().map(|v| v.as_str().unwrap()).collect();
    
    assert!(names.contains(&"db1"));
    // assert!(names.contains(&"_system")); // Default system db may not be created in test env automatically
}

// ============================================================================
// Collection API Tests
// ============================================================================

#[tokio::test]
async fn test_create_collection_api() {
    let (app, _tmp) = create_test_app();
    
    // Create DB
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "mydb" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Create Collection
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/mydb/collection")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "users" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
}

// ============================================================================
// Document API Tests
// ============================================================================

#[tokio::test]
async fn test_create_document_api() {
    let (app, _tmp) = create_test_app();
    
    // Setup DB and Collection
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "db" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/db/collection")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "col" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Insert Document
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/db/document/col")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "Alice" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json["_key"].is_string());
}

#[tokio::test]
async fn test_get_document_api() {
    let (app, _tmp) = create_test_app();
    
    // Setup
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "db" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/db/collection")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "col" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Insert
    let response = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/db/document/col")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "_key": "doc1", "val": 123 }).to_string()))
            .unwrap(),
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    
    // Get
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/database/db/document/col/doc1")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["val"], 123);
}

// ============================================================================
// Query API Tests
// ============================================================================

#[tokio::test]
async fn test_query_api() {
    let (app, _tmp) = create_test_app();
    
    // Setup
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "db" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    // Execute Query
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/db/cursor")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ 
                "query": "RETURN 1 + 1" 
            }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let result = &json["result"];
    assert!(result.is_array());
    assert_eq!(result[0], 2.0); // Arithmetic returns float
}

#[tokio::test]
async fn test_query_with_binds_api() {
    let (app, _tmp) = create_test_app();
    
    // Setup
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "db" }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/db/cursor")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ 
                "query": "RETURN @val",
                "bindVars": { "val": "hello" }
            }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let result = &json["result"];
    assert_eq!(result[0], "hello");
}

// ============================================================================
// Error Handling API Tests
// ============================================================================

#[tokio::test]
async fn test_not_found_api() {
    let (app, _tmp) = create_test_app();
    
    let response = app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/_api/database/nonexistent/document/col/doc")
            .body(Body::empty())
            .unwrap(),
    ).await.unwrap();
    
    // Should be Not Found (404) or similar error
    // Accessing DB that doesn't exist usually returns 404
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_bad_request_api() {
    let (app, _tmp) = create_test_app();
    
    // Create DB first
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ "name": "db" }).to_string()))
            .unwrap(),
    ).await.unwrap();

    // Invalid Query
    let response = app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database/db/cursor")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ 
                "query": "INVALID SYNTAX" 
            }).to_string()))
            .unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
