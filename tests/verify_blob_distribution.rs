//! Tests for blob collection distribution architecture
//!
//! This file tests the new blob collection behavior:
//! - Blob collections are NOT auto-sharded by default
//! - Collection type can be changed via the API

use axum::{body::Body, http::{Request, StatusCode}};
use serde_json::{json, Value};
use solidb::{create_router, StorageEngine};
use tempfile::TempDir;
use tower::util::ServiceExt;

/// Helper to create a test app
fn create_test_app() -> (axum::Router, TempDir) {
    // Set admin password for tests
    std::env::set_var("SOLIDB_ADMIN_PASSWORD", "admin");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    storage.initialize().expect("Failed to initialize storage");
    let router = create_router(storage, None, None, None, 0);
    (router, temp_dir)
}

/// Helper to make a POST request with JSON body
async fn post_json(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path).header(axum::http::header::AUTHORIZATION, "Basic YWRtaW46YWRtaW4=")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap_or(json!(null));
    (status, json)
}

/// Helper to make a PUT request with JSON body
async fn put_json(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(path)
                .header(axum::http::header::AUTHORIZATION, "Basic YWRtaW46YWRtaW4=")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap_or(json!(null));
    (status, json)
}

/// Helper to make a GET request
async fn get(app: &axum::Router, path: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .header(axum::http::header::AUTHORIZATION, "Basic YWRtaW46YWRtaW4=")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap_or(json!(null));
    (status, json)
}

/// Test that blob collections are created without auto-sharding
#[tokio::test]
async fn test_blob_collections_not_auto_sharded() {
    let (router, _temp_dir) = create_test_app();

    // Create database
    let (_status, _body) = post_json(&router, "/_api/database/test_db", json!({})).await;
    assert_eq!(_status, StatusCode::OK);

    // Create blob collection (should not be auto-sharded)
    let (_status, _body) = post_json(&router, "/_api/database/test_db/collection", json!({
        "name": "files",
        "type": "blob"
    })).await;
    assert_eq!(_status, StatusCode::OK);

    // Verify collection was created and is not sharded
    let (status, body) = get(&router, "/_api/database/test_db/collection").await;
    assert_eq!(status, StatusCode::OK);

    let collections = body.get("collections").unwrap().as_array().unwrap();
    let files_collection = collections.iter()
        .find(|c| c.get("name").unwrap().as_str().unwrap() == "files")
        .unwrap();

    assert_eq!(files_collection.get("type").unwrap().as_str().unwrap(), "blob");
    assert!(files_collection.get("shardConfig").is_none(),
        "Blob collection should not be auto-sharded");
}

/// Test that collection type can be changed via API
#[tokio::test]
async fn test_blob_collection_type_change_via_api() {
    let (router, _temp_dir) = create_test_app();

    // Create database and document collection
    let (_status, _body) = post_json(&router, "/_api/database/test_convert", json!({})).await;
    assert_eq!(_status, StatusCode::OK);

    let (_status, _body) = post_json(&router, "/_api/database/test_convert/collection", json!({
        "name": "docs",
        "type": "document"
    })).await;
    assert_eq!(_status, StatusCode::OK);

    // Verify it's a document collection
    let (status, body) = get(&router, "/_api/database/test_convert/collection").await;
    assert_eq!(status, StatusCode::OK);

    let collections = body.get("collections").unwrap().as_array().unwrap();
    let docs_collection = collections.iter()
        .find(|c| c.get("name").unwrap().as_str().unwrap() == "docs")
        .unwrap();

    assert_eq!(docs_collection.get("type").unwrap().as_str().unwrap(), "document");

    // Change collection type to blob via properties API
    let (status, _body) = put_json(&router, "/_api/database/test_convert/collection/docs/properties", json!({
        "type": "blob"
    })).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify collection type changed
    let (status, body) = get(&router, "/_api/database/test_convert/collection").await;
    assert_eq!(status, StatusCode::OK);

    let collections = body.get("collections").unwrap().as_array().unwrap();
    let docs_collection = collections.iter()
        .find(|c| c.get("name").unwrap().as_str().unwrap() == "docs")
        .unwrap();

    assert_eq!(docs_collection.get("type").unwrap().as_str().unwrap(), "blob");
}
