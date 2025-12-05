//! HTTP API Integration Tests
//! Tests for the REST API endpoints

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use solidb::{create_router, StorageEngine};
use serde_json::{json, Value};
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
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
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
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
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
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    response.status()
}

/// Helper to make a PUT request with JSON body
async fn put_json(app: &axum::Router, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap_or(json!(null));
    (status, json)
}

// ==================== Collection API Tests ====================

#[tokio::test]
async fn test_create_collection() {
    let (app, _dir) = create_test_app();

    let (status, body) = post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "users");
    assert_eq!(body["status"], "created");
}

#[tokio::test]
async fn test_create_duplicate_collection() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    let (status, body) = post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap().contains("already exists"));
}

#[tokio::test]
async fn test_list_collections() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/collection", json!({"name": "products"})).await;

    let (status, body) = get(&app, "/_api/database/_system/collection").await;

    assert_eq!(status, StatusCode::OK);
    let collections = body["collections"].as_array().unwrap();
    assert_eq!(collections.len(), 2);
}

#[tokio::test]
async fn test_delete_collection() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    let status = delete(&app, "/_api/database/_system/collection/users").await;

    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify deleted
    let (_, body) = get(&app, "/_api/database/_system/collection").await;
    let collections = body["collections"].as_array().unwrap();
    assert!(collections.is_empty());
}

// ==================== Document API Tests ====================

#[tokio::test]
async fn test_insert_document() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    let (status, body) = post_json(&app, "/_api/database/_system/document/users", json!({
        "name": "Alice",
        "age": 30
    })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Alice");
    assert_eq!(body["age"], 30);
    assert!(body["_key"].as_str().is_some());
    assert!(body["_id"].as_str().is_some());
}

#[tokio::test]
async fn test_insert_document_with_custom_key() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    let (status, body) = post_json(&app, "/_api/database/_system/document/users", json!({
        "_key": "alice",
        "name": "Alice"
    })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["_key"], "alice");
}

#[tokio::test]
async fn test_get_document() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({
        "_key": "alice",
        "name": "Alice"
    })).await;

    let (status, body) = get(&app, "/_api/database/_system/document/users/alice").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Alice");
}

#[tokio::test]
async fn test_get_nonexistent_document() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    let (status, body) = get(&app, "/_api/database/_system/document/users/nonexistent").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn test_update_document() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({
        "_key": "alice",
        "name": "Alice",
        "age": 30
    })).await;

    let (status, body) = put_json(&app, "/_api/database/_system/document/users/alice", json!({
        "age": 31,
        "city": "Paris"
    })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "Alice"); // Original preserved
    assert_eq!(body["age"], 31);       // Updated
    assert_eq!(body["city"], "Paris"); // Added
}

#[tokio::test]
async fn test_delete_document() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({
        "_key": "alice",
        "name": "Alice"
    })).await;

    let status = delete(&app, "/_api/database/_system/document/users/alice").await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify deleted
    let (status, _) = get(&app, "/_api/database/_system/document/users/alice").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ==================== Query API Tests ====================

#[tokio::test]
async fn test_execute_query() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Alice", "age": 30})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Bob", "age": 25})).await;

    let (status, body) = post_json(&app, "/_api/database/_system/cursor", json!({
        "query": "FOR doc IN users RETURN doc.name"
    })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 2);
    let results = body["result"].as_array().unwrap();
    assert!(results.contains(&json!("Alice")));
    assert!(results.contains(&json!("Bob")));
}

#[tokio::test]
async fn test_execute_query_with_filter() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Alice", "age": 30})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Bob", "age": 25})).await;

    let (status, body) = post_json(&app, "/_api/database/_system/cursor", json!({
        "query": "FOR doc IN users FILTER doc.age > 26 RETURN doc.name"
    })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 1);
    assert_eq!(body["result"][0], "Alice");
}

#[tokio::test]
async fn test_execute_query_with_sort() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Alice", "age": 30})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Bob", "age": 25})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Charlie", "age": 35})).await;

    let (status, body) = post_json(&app, "/_api/database/_system/cursor", json!({
        "query": "FOR doc IN users SORT doc.age DESC RETURN doc.name"
    })).await;

    assert_eq!(status, StatusCode::OK);
    let results = body["result"].as_array().unwrap();
    assert_eq!(results[0], "Charlie");
    assert_eq!(results[1], "Alice");
    assert_eq!(results[2], "Bob");
}

#[tokio::test]
async fn test_execute_invalid_query() {
    let (app, _dir) = create_test_app();

    let (status, body) = post_json(&app, "/_api/database/_system/cursor", json!({
        "query": "INVALID QUERY SYNTAX"
    })).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().is_some());
}

// ==================== Index API Tests ====================

#[tokio::test]
async fn test_create_index() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    let (status, body) = post_json(&app, "/_api/database/_system/index/users", json!({
        "name": "idx_age",
        "field": "age",
        "type": "persistent"
    })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "idx_age");
    assert_eq!(body["field"], "age");
    assert_eq!(body["status"], "created");
}

#[tokio::test]
async fn test_list_indexes() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/index/users", json!({
        "name": "idx_age",
        "field": "age",
        "type": "persistent"
    })).await;

    let (status, body) = get(&app, "/_api/database/_system/index/users").await;

    assert_eq!(status, StatusCode::OK);
    let indexes = body["indexes"].as_array().unwrap();
    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0]["name"], "idx_age");
}

#[tokio::test]
async fn test_delete_index() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/index/users", json!({
        "name": "idx_age",
        "field": "age",
        "type": "persistent"
    })).await;

    let status = delete(&app, "/_api/database/_system/index/users/idx_age").await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Verify deleted
    let (_, body) = get(&app, "/_api/database/_system/index/users").await;
    let indexes = body["indexes"].as_array().unwrap();
    assert!(indexes.is_empty());
}

// ==================== Geo API Tests ====================

#[tokio::test]
async fn test_create_geo_index() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "places"})).await;
    let (status, body) = post_json(&app, "/_api/database/_system/geo/places", json!({
        "name": "idx_location",
        "field": "location"
    })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["name"], "idx_location");
    assert_eq!(body["type"], "geo");
}

#[tokio::test]
async fn test_geo_near_query() {
    let (app, _dir) = create_test_app();

    // Setup
    post_json(&app, "/_api/database/_system/collection", json!({"name": "places"})).await;
    post_json(&app, "/_api/database/_system/document/places", json!({
        "_key": "eiffel",
        "name": "Eiffel Tower",
        "location": {"lat": 48.8584, "lon": 2.2945}
    })).await;
    post_json(&app, "/_api/database/_system/document/places", json!({
        "_key": "louvre",
        "name": "Louvre Museum",
        "location": {"lat": 48.8606, "lon": 2.3376}
    })).await;
    post_json(&app, "/_api/database/_system/geo/places", json!({
        "name": "idx_location",
        "field": "location"
    })).await;

    // Query
    let (status, body) = post_json(&app, "/_api/database/_system/geo/places/location/near", json!({
        "lat": 48.8584,
        "lon": 2.2945,
        "limit": 2
    })).await;

    assert_eq!(status, StatusCode::OK);
    assert!(body["count"].as_u64().unwrap() > 0);

    // First result should be Eiffel Tower (closest)
    let results = body["results"].as_array().unwrap();
    assert_eq!(results[0]["document"]["name"], "Eiffel Tower");
}

#[tokio::test]
async fn test_geo_within_query() {
    let (app, _dir) = create_test_app();

    // Setup
    post_json(&app, "/_api/database/_system/collection", json!({"name": "places"})).await;
    post_json(&app, "/_api/database/_system/document/places", json!({
        "_key": "eiffel",
        "name": "Eiffel Tower",
        "location": {"lat": 48.8584, "lon": 2.2945}
    })).await;
    post_json(&app, "/_api/database/_system/document/places", json!({
        "_key": "london_eye",
        "name": "London Eye",
        "location": {"lat": 51.5033, "lon": -0.1196}
    })).await;
    post_json(&app, "/_api/database/_system/geo/places", json!({
        "name": "idx_location",
        "field": "location"
    })).await;

    // Query within 10km of Eiffel Tower
    let (status, body) = post_json(&app, "/_api/database/_system/geo/places/location/within", json!({
        "lat": 48.8584,
        "lon": 2.2945,
        "radius": 10000
    })).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["count"], 1);
    assert_eq!(body["results"][0]["document"]["name"], "Eiffel Tower");
}

// ==================== Error Handling Tests ====================

#[tokio::test]
async fn test_collection_not_found_error() {
    let (app, _dir) = create_test_app();

    let (status, body) = get(&app, "/_api/database/_system/document/nonexistent/key").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body["error"].as_str().unwrap().contains("not found"));
}

#[tokio::test]
async fn test_invalid_json_body() {
    let (app, _dir) = create_test_app();

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/_system/collection")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("invalid json"))
                .unwrap(),
        )
        .await
        .unwrap();

    // Invalid JSON returns 400 Bad Request
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

// ==================== Explain API Tests ====================

#[tokio::test]
async fn test_explain_simple_query() {
    let (app, _dir) = create_test_app();

    // Setup collection with documents
    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Alice", "age": 30})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Bob", "age": 25})).await;

    let (status, body) = post_json(&app, "/_api/database/_system/explain", json!({
        "query": "FOR doc IN users RETURN doc"
    })).await;

    assert_eq!(status, StatusCode::OK);

    // Check collections info
    assert!(body["collections"].is_array());
    let collections = body["collections"].as_array().unwrap();
    assert_eq!(collections.len(), 1);
    assert_eq!(collections[0]["name"], "users");
    assert_eq!(collections[0]["variable"], "doc");
    assert_eq!(collections[0]["documents_count"], 2);

    // Check timing info exists
    assert!(body["timing"].is_object());
    assert!(body["timing"]["total_us"].is_number());

    // Check documents counts
    assert_eq!(body["documents_scanned"], 2);
    assert_eq!(body["documents_returned"], 2);
}

#[tokio::test]
async fn test_explain_with_filter() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Alice", "age": 30})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Bob", "age": 25})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Charlie", "age": 35})).await;

    let (status, body) = post_json(&app, "/_api/database/_system/explain", json!({
        "query": "FOR doc IN users FILTER doc.age > 26 RETURN doc"
    })).await;

    assert_eq!(status, StatusCode::OK);

    // Check filter info
    assert!(body["filters"].is_array());
    let filters = body["filters"].as_array().unwrap();
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0]["documents_before"], 3);
    assert_eq!(filters[0]["documents_after"], 2); // Alice (30) and Charlie (35)

    assert_eq!(body["documents_scanned"], 3);
    assert_eq!(body["documents_returned"], 2);
}

#[tokio::test]
async fn test_explain_with_sort() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Alice", "age": 30})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Bob", "age": 25})).await;

    let (status, body) = post_json(&app, "/_api/database/_system/explain", json!({
        "query": "FOR doc IN users SORT doc.age DESC RETURN doc"
    })).await;

    assert_eq!(status, StatusCode::OK);

    // Check sort info
    assert!(body["sort"].is_object());
    assert_eq!(body["sort"]["field"], "doc.age");
    assert_eq!(body["sort"]["direction"], "DESC");
    assert!(body["sort"]["time_us"].is_number());
}

#[tokio::test]
async fn test_explain_with_limit() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Alice", "age": 30})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Bob", "age": 25})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Charlie", "age": 35})).await;

    let (status, body) = post_json(&app, "/_api/database/_system/explain", json!({
        "query": "FOR doc IN users LIMIT 2 RETURN doc"
    })).await;

    assert_eq!(status, StatusCode::OK);

    // Check limit info
    assert!(body["limit"].is_object());
    assert_eq!(body["limit"]["offset"], 0);
    assert_eq!(body["limit"]["count"], 2);

    assert_eq!(body["documents_returned"], 2);
}

#[tokio::test]
async fn test_explain_with_let_subquery() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Alice", "age": 30, "active": true})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Bob", "age": 25, "active": true})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Charlie", "age": 35, "active": false})).await;

    let (status, body) = post_json(&app, "/_api/database/_system/explain", json!({
        "query": "LET activeUsers = (FOR u IN users FILTER u.active == true RETURN u) FOR item IN activeUsers RETURN item.name"
    })).await;

    assert_eq!(status, StatusCode::OK);

    // Check LET bindings info
    assert!(body["let_bindings"].is_array());
    let let_bindings = body["let_bindings"].as_array().unwrap();
    assert_eq!(let_bindings.len(), 1);
    assert_eq!(let_bindings[0]["variable"], "activeUsers");
    assert_eq!(let_bindings[0]["is_subquery"], true);

    // Alice and Bob are active
    assert_eq!(body["documents_returned"], 2);
}

#[tokio::test]
async fn test_explain_timing_breakdown() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Alice", "age": 30})).await;

    let (status, body) = post_json(&app, "/_api/database/_system/explain", json!({
        "query": "FOR doc IN users FILTER doc.age > 20 SORT doc.name LIMIT 10 RETURN doc"
    })).await;

    assert_eq!(status, StatusCode::OK);

    // Check all timing fields exist
    let timing = &body["timing"];
    assert!(timing["total_us"].is_number());
    assert!(timing["let_clauses_us"].is_number());
    assert!(timing["collection_scan_us"].is_number());
    assert!(timing["filter_us"].is_number());
    assert!(timing["sort_us"].is_number());
    assert!(timing["limit_us"].is_number());
    assert!(timing["return_projection_us"].is_number());

    // Total should be >= sum of parts (approximately)
    let total = timing["total_us"].as_u64().unwrap();
    assert!(total > 0);
}

#[tokio::test]
async fn test_explain_with_bind_vars() {
    let (app, _dir) = create_test_app();

    post_json(&app, "/_api/database/_system/collection", json!({"name": "users"})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Alice", "age": 30})).await;
    post_json(&app, "/_api/database/_system/document/users", json!({"name": "Bob", "age": 25})).await;

    let (status, body) = post_json(&app, "/_api/database/_system/explain", json!({
        "query": "FOR doc IN users FILTER doc.age > @minAge RETURN doc",
        "bindVars": {
            "minAge": 28
        }
    })).await;

    assert_eq!(status, StatusCode::OK);

    // Filter should reduce from 2 to 1 (only Alice age 30)
    let filters = body["filters"].as_array().unwrap();
    assert_eq!(filters[0]["documents_before"], 2);
    assert_eq!(filters[0]["documents_after"], 1);
    assert_eq!(body["documents_returned"], 1);
}

#[tokio::test]
async fn test_explain_invalid_query() {
    let (app, _dir) = create_test_app();

    let (status, body) = post_json(&app, "/_api/database/_system/explain", json!({
        "query": "INVALID QUERY SYNTAX"
    })).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"].as_str().is_some());
}

