//! API Request Validation Tests
//!
//! Tests for API endpoint request validation including:
//! - Invalid JSON handling
//! - Missing required fields
//! - Invalid field types
//! - Edge cases in request parameters

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::auth::AuthService;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

fn create_test_app() -> (axum::Router, TempDir, String) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    let script_stats = Arc::new(ScriptStats::default());

    let router = create_router(engine, None, None, None, None, script_stats, None, 0);

    // Create a JWT token for authentication
    let token = AuthService::create_jwt_with_roles(
        "test_admin",
        Some(vec!["admin".to_string()]),
        None,
    )
    .expect("Failed to create test token");

    (router, tmp_dir, token)
}

fn auth_header(token: &str) -> String {
    format!("Bearer {}", token)
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body)
        .unwrap_or(json!({"raw": String::from_utf8_lossy(&body).to_string()}))
}

// ============================================================================
// Database API Validation Tests
// ============================================================================

#[tokio::test]
async fn test_create_database_missing_name() {
    let (app, _tmp, token) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    // May return 400 or other client error
    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_create_database_empty_name() {
    let (app, _tmp, token) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": ""}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_database_invalid_json() {
    let (app, _tmp, token) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from("{invalid json}"))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 4xx error for invalid JSON
    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_get_nonexistent_database() {
    let (app, _tmp, token) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/database/nonexistent_db")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // May return 404 or 405 (Method Not Allowed)
    assert!(response.status().is_client_error());
}

// ============================================================================
// Collection API Validation Tests
// ============================================================================

#[tokio::test]
async fn test_create_collection_missing_name() {
    let (app, _tmp, token) = create_test_app();

    // First create a database
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "testdb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Try to create collection without name
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    // May return 400 or other client error
    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_create_collection_invalid_type() {
    let (app, _tmp, token) = create_test_app();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "testdb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Invalid collection type
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({"name": "test", "type": "invalid_type"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should fail or be treated as default
    let json = response_json(response).await;
    // Check that it either fails or ignores invalid type
    assert!(json.get("error").is_some() || json.get("name").is_some());
}

#[tokio::test]
async fn test_get_collection_from_nonexistent_db() {
    let (app, _tmp, token) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/database/nonexistent/collection/test")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // API may return 404 or 405
    assert!(response.status().is_client_error());
}

// ============================================================================
// Document API Validation Tests
// ============================================================================

#[tokio::test]
async fn test_insert_document_invalid_json() {
    let (app, _tmp, token) = create_test_app();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "testdb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "docs"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Invalid JSON body
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/document/docs")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from("{not valid json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_get_document_not_found() {
    let (app, _tmp, token) = create_test_app();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "testdb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "docs"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/database/testdb/document/docs/nonexistent_key")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_document_not_found() {
    let (app, _tmp, token) = create_test_app();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "testdb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "docs"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/_api/database/testdb/document/docs/nonexistent")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Query API Validation Tests
// ============================================================================

#[tokio::test]
async fn test_query_missing_query_field() {
    let (app, _tmp, token) = create_test_app();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "testdb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    // May return 400, 422, etc.
    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_query_empty_query_string() {
    let (app, _tmp, token) = create_test_app();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "testdb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"query": ""}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_query_invalid_syntax() {
    let (app, _tmp, token) = create_test_app();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "testdb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({"query": "FOR invalid syntax"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_query_nonexistent_collection() {
    let (app, _tmp, token) = create_test_app();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "testdb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({"query": "FOR doc IN nonexistent RETURN doc"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return error (collection not found)
    assert!(response.status().is_client_error() || response.status().is_server_error());
}

// ============================================================================
// Index API Validation Tests
// ============================================================================

#[tokio::test]
async fn test_create_index_missing_fields() {
    let (app, _tmp, token) = create_test_app();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "testdb"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"name": "docs"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Missing fields array
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/testdb/collection/docs/index")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({"type": "hash"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // May return 400, 404, etc.
    assert!(response.status().is_client_error());
}

// ============================================================================
// Edge Cases Tests
// ============================================================================

#[tokio::test]
async fn test_empty_body() {
    let (app, _tmp, token) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(response.status().is_client_error());
}

#[tokio::test]
async fn test_wrong_content_type() {
    let (app, _tmp, token) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "text/plain")
                .header("Authorization", auth_header(&token))
                .body(Body::from("name=test"))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should handle gracefully
    assert!(response.status().is_client_error() || response.status().is_success());
}

#[tokio::test]
async fn test_special_characters_in_path() {
    let (app, _tmp, token) = create_test_app();

    // URL-encoded special characters
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/database/test%20db")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return client error (DB doesn't exist)
    assert!(response.status().is_client_error());
}
