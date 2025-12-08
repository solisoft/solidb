use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::{create_router, StorageEngine};
use tempfile::TempDir;
use tower::util::ServiceExt;

/// Helper to create a test app
fn create_test_app() -> (axum::Router, TempDir) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    storage.initialize().expect("Failed to initialize storage");
    let router = create_router(storage, None);
    (router, temp_dir)
}

/// Helper to make a POST request with JSON body
async fn post_json(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path).header("Authorization", "Basic YWRtaW46YWRtaW4=")
                .header(header::CONTENT_TYPE, "application/json")
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

/// Helper to make a PUT request
async fn put(app: &axum::Router, path: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(path).header("Authorization", "Basic YWRtaW46YWRtaW4=")
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

/// Helper to make a DELETE request
async fn delete(app: &axum::Router, path: &str) -> StatusCode {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(path).header("Authorization", "Basic YWRtaW46YWRtaW4=")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    response.status()
}

#[tokio::test]
async fn test_cursor_pagination_basic() {
    let (app, _dir) = create_test_app();

    // Create collection and insert 250 documents
    post_json(
        &app,
        "/_api/database/_system/collection",
        json!({"name": "items"}),
    )
    .await;
    for i in 0..250 {
        post_json(
            &app,
            "/_api/database/_system/document/items",
            json!({"id": i, "value": format!("item{}", i)}),
        )
        .await;
    }

    // Query with default batch size (100)
    let (status, body) = post_json(
        &app,
        "/_api/database/_system/cursor",
        json!({
            "query": "FOR item IN items RETURN item"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 250);
    assert_eq!(body["result"].as_array().unwrap().len(), 100); // First batch
    assert_eq!(body["hasMore"], true);
    assert!(body["id"].is_string());
    assert_eq!(body["cached"], false);

    let cursor_id = body["id"].as_str().unwrap();

    // Get second batch
    let (status, body) = put(&app, &format!("/_api/cursor/{}", cursor_id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"].as_array().unwrap().len(), 100); // Second batch
    assert_eq!(body["hasMore"], true);
    assert_eq!(body["cached"], true);

    // Get third batch (remaining 50)
    let (status, body) = put(&app, &format!("/_api/cursor/{}", cursor_id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"].as_array().unwrap().len(), 50); // Last batch
    assert_eq!(body["hasMore"], false);
    assert!(body["id"].is_null());
}

#[tokio::test]
async fn test_cursor_custom_batch_size() {
    let (app, _dir) = create_test_app();

    post_json(
        &app,
        "/_api/database/_system/collection",
        json!({"name": "items"}),
    )
    .await;
    for i in 0..50 {
        post_json(
            &app,
            "/_api/database/_system/document/items",
            json!({"id": i}),
        )
        .await;
    }

    // Query with custom batch size
    let (status, body) = post_json(
        &app,
        "/_api/database/_system/cursor",
        json!({
            "query": "FOR item IN items RETURN item",
            "batchSize": 10
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 50);
    assert_eq!(body["result"].as_array().unwrap().len(), 10);
    assert_eq!(body["hasMore"], true);
}

#[tokio::test]
async fn test_cursor_small_result_set() {
    let (app, _dir) = create_test_app();

    post_json(
        &app,
        "/_api/database/_system/collection",
        json!({"name": "items"}),
    )
    .await;
    for i in 0..5 {
        post_json(
            &app,
            "/_api/database/_system/document/items",
            json!({"id": i}),
        )
        .await;
    }

    // Query with default batch size (100) - should return all results without cursor
    let (status, body) = post_json(
        &app,
        "/_api/database/_system/cursor",
        json!({
            "query": "FOR item IN items RETURN item"
        }),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 5);
    assert_eq!(body["result"].as_array().unwrap().len(), 5);
    assert_eq!(body["hasMore"], false);
    assert!(body["id"].is_null());
}

#[tokio::test]
async fn test_cursor_delete() {
    let (app, _dir) = create_test_app();

    post_json(
        &app,
        "/_api/database/_system/collection",
        json!({"name": "items"}),
    )
    .await;
    for i in 0..200 {
        post_json(
            &app,
            "/_api/database/_system/document/items",
            json!({"id": i}),
        )
        .await;
    }

    // Create cursor
    let (_, body) = post_json(
        &app,
        "/_api/database/_system/cursor",
        json!({
            "query": "FOR item IN items RETURN item"
        }),
    )
    .await;

    let cursor_id = body["id"].as_str().unwrap();

    // Delete cursor
    let status = delete(&app, &format!("/_api/cursor/{}", cursor_id)).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Try to use deleted cursor
    let (status, _) = put(&app, &format!("/_api/cursor/{}", cursor_id)).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_cursor_invalid_id() {
    let (app, _dir) = create_test_app();

    // Try to use non-existent cursor
    let (status, _) = put(&app, "/_api/cursor/invalid-cursor-id").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
