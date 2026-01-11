//! Blob Distribution and Replication Tests
//!
//! Tests for the blob storage and replication endpoints.
//! Verifies:
//! - Blob upload handling (multipart)
//! - Blob chunk retrieval
//! - Internal replication handlers

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

fn create_test_app() -> (axum::Router, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");

    let script_stats = Arc::new(ScriptStats::default());

    let router = create_router(engine, None, None, None, None, script_stats, 0);

    (router, tmp_dir)
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn create_multipart_body(boundary: &str, parts: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
    let mut body = Vec::new();
    for (name, data) in parts {
        body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", name).as_bytes(),
        );
        body.extend_from_slice(&data);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{}--\r\n", boundary).as_bytes());
    body
}

#[tokio::test]
async fn test_upload_and_retrieve_blob() {
    let (app, _tmp) = create_test_app();
    let boundary = "------------------------Boundary123";

    // 1. Create DB and Blob Collection
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "name": "blob_db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/blob_db/collection")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({ "name": "images", "type": "blob" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // 2. Upload Blob (Internal Endpoint)
    // We simulate an internal upload which normally comes from ShardCoordinator
    // POST /_internal/blob/upload/:db/:collection

    let metadata = json!({ "_key": "test_image.png", "content_type": "image/png" }).to_string();
    let chunk_data = b"Hello Blob World".to_vec();

    let parts = vec![
        ("metadata", metadata.into_bytes()),
        ("chunk_0", chunk_data.clone()),
    ];

    let body_bytes = create_multipart_body(boundary, parts);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_internal/blob/upload/blob_db/images")
                .header(
                    "Content-Type",
                    format!("multipart/form-data; boundary={}", boundary),
                )
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["_key"], "test_image.png");

    // 3. Retrieve Blob Chunk
    // GET /_internal/blob/replicate/:db/:collection/:key/chunk/:chunk_idx

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_internal/blob/replicate/blob_db/images/test_image.png/chunk/0")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    assert_eq!(body, chunk_data.as_slice());
}

#[tokio::test]
async fn test_blob_replication_endpoint() {
    let (app, _tmp) = create_test_app();
    let boundary = "------------------------BoundaryReplication";

    // Setup
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "name": "rep_db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/rep_db/collection")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({ "name": "files", "type": "blob" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // POST /_internal/blob/replicate/:db/:collection/:key
    let metadata = json!({ "_key": "file1", "size": 100 }).to_string();
    let chunk0 = vec![0u8; 10];
    let chunk1 = vec![1u8; 10];

    let parts = vec![
        ("metadata", metadata.into_bytes()),
        ("chunk_0", chunk0.clone()),
        ("chunk_1", chunk1.clone()),
    ];

    let body_bytes = create_multipart_body(boundary, parts);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_internal/blob/replicate/rep_db/files/file1")
                .header(
                    "Content-Type",
                    format!("multipart/form-data; boundary={}", boundary),
                )
                .body(Body::from(body_bytes))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // Check Status
    let json = response_json(response).await;
    assert_eq!(json["chunks_received"], 2);
}
