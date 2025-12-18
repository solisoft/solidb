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

/// Helper to make a GET request
async fn get(app: &axum::Router, path: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
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

#[tokio::test]
async fn test_truncate_collection() {
    let (app, _dir) = create_test_app();

    // Create collection
    post_json(
        &app,
        "/_api/database/_system/collection",
        json!({"name": "users"}),
    )
    .await;

    // Insert documents
    post_json(
        &app,
        "/_api/database/_system/document/users",
        json!({"name": "Alice", "age": 30}),
    )
    .await;
    post_json(
        &app,
        "/_api/database/_system/document/users",
        json!({"name": "Bob", "age": 25}),
    )
    .await;
    post_json(
        &app,
        "/_api/database/_system/document/users",
        json!({"name": "Charlie", "age": 35}),
    )
    .await;

    // Create an index
    post_json(
        &app,
        "/_api/database/_system/index/users",
        json!({
            "name": "idx_age",
            "field": "age",
            "type": "persistent"
        }),
    )
    .await;

    // Verify documents exist
    let (status, body) = post_json(
        &app,
        "/_api/database/_system/cursor",
        json!({
            "query": "FOR u IN users RETURN u"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 3);

    // Truncate collection
    let (status, body) = put(&app, "/_api/database/_system/collection/users/truncate").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["collection"], "users");
    assert_eq!(body["deleted"], 3);
    assert_eq!(body["status"], "truncated");

    // Verify all documents are gone
    let (status, body) = post_json(
        &app,
        "/_api/database/_system/cursor",
        json!({
            "query": "FOR u IN users RETURN u"
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 0);

    // Verify index still exists
    let (status, body) = get(&app, "/_api/database/_system/index/users").await;
    assert_eq!(status, StatusCode::OK);
    let indexes = body["indexes"].as_array().unwrap();
    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0]["name"], "idx_age");

    // Verify we can still insert documents
    let (status, _) = post_json(
        &app,
        "/_api/database/_system/document/users",
        json!({"name": "Dave", "age": 40}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn test_truncate_nonexistent_collection() {
    let (app, _dir) = create_test_app();

    // Try to truncate non-existent collection
    let (status, _) = put(
        &app,
        "/_api/database/_system/collection/nonexistent/truncate",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
