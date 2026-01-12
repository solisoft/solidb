//! Script Handlers Tests
//!
//! Comprehensive tests for server/script_handlers.rs including:
//! - Script CRUD operations (create, list, get, update, delete)
//! - Script stats endpoint
//! - Helper function unit tests

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

    let router = create_router(engine, None, None, None, None, script_stats, None, 0);

    (router, tmp_dir)
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap_or(
        json!({"error": "Invalid JSON", "body": String::from_utf8_lossy(&body).to_string()}),
    )
}

async fn setup_db(app: &axum::Router, db_name: &str) {
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({ "name": db_name }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
}

// ============================================================================
// Script Management Tests
// ============================================================================

#[tokio::test]
async fn test_create_script() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/scriptdb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "hello",
                        "path": "/hello",
                        "methods": ["GET"],
                        "code": "return { message = 'Hello World' }"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["name"], "hello");
    assert!(json["id"].is_string());
}

#[tokio::test]
async fn test_create_script_with_description() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/scriptdb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "greet",
                        "path": "/greet",
                        "methods": ["POST"],
                        "code": "return { greeting = 'Hi' }",
                        "description": "A greeting endpoint"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["name"], "greet");
}

#[tokio::test]
async fn test_list_scripts() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    // Create some scripts
    for (name, path) in [("script1", "/path1"), ("script2", "/path2")] {
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_api/database/scriptdb/scripts")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": name,
                            "path": path,
                            "methods": ["GET"],
                            "code": "return {}"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/database/scriptdb/scripts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let scripts = json["scripts"].as_array().unwrap();
    assert!(scripts.len() >= 2);
}

#[tokio::test]
async fn test_get_script() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    // Create script
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/scriptdb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "myapi",
                        "path": "/myapi",
                        "methods": ["GET"],
                        "code": "return { value = 42 }"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let json = response_json(response).await;
    let script_id = json["id"].as_str().unwrap().to_string();

    // Get script
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/_api/database/scriptdb/scripts/{}", script_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["name"], "myapi");
    assert!(json["code"].as_str().unwrap().contains("42"));
}

#[tokio::test]
async fn test_get_nonexistent_script() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/database/scriptdb/scripts/nonexistent123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_script() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    // Create script
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/scriptdb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "updateme",
                        "path": "/updateme",
                        "methods": ["GET"],
                        "code": "return { version = 1 }"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let json = response_json(response).await;
    let script_id = json["id"].as_str().unwrap().to_string();

    // Update script
    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(&format!("/_api/database/scriptdb/scripts/{}", script_id))
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "updateme",
                        "path": "/updateme",
                        "methods": ["POST"],
                        "code": "return { version = 2 }",
                        "description": "Updated script"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["methods"][0], "POST");
    assert!(json["code"].as_str().unwrap().contains("version = 2"));
}

#[tokio::test]
async fn test_delete_script() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    // Create script
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/scriptdb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "todelete",
                        "path": "/todelete",
                        "methods": ["GET"],
                        "code": "return {}"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let json = response_json(response).await;
    let script_id = json["id"].as_str().unwrap().to_string();

    // Delete script
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(&format!("/_api/database/scriptdb/scripts/{}", script_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["deleted"], true);

    // Verify deleted
    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(&format!("/_api/database/scriptdb/scripts/{}", script_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_nonexistent_script() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/_api/database/scriptdb/scripts/nonexistent123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Script Stats Tests
// ============================================================================

#[tokio::test]
async fn test_get_script_stats() {
    let (app, _tmp) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/scripts/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    // Stats has active_scripts, active_ws, total_scripts_executed, total_ws_connections
    assert!(json.get("active_scripts").is_some());
    assert!(json.get("total_scripts_executed").is_some());
}

// ============================================================================
// Script Validation Tests
// ============================================================================

#[tokio::test]
async fn test_create_script_missing_required_fields() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    // Missing 'code' field
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/scriptdb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "incomplete",
                        "path": "/incomplete",
                        "methods": ["GET"]
                        // missing "code"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return error for missing code
    assert!(
        response.status() == StatusCode::BAD_REQUEST
            || response.status() == StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn test_create_script_various_methods() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    for method in ["GET", "POST", "PUT", "DELETE"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_api/database/scriptdb/scripts")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        json!({
                            "name": format!("script_{}", method.to_lowercase()),
                            "path": format!("/test_{}", method.to_lowercase()),
                            "methods": [method],
                            "code": "return {}"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Failed for method: {}",
            method
        );
    }
}

#[tokio::test]
async fn test_create_script_with_multiple_methods() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/scriptdb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "multi_method",
                        "path": "/multi",
                        "methods": ["GET", "POST", "PUT"],
                        "code": "return { method = request.method }"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["methods"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn test_create_script_with_path_params() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/scriptdb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "user_by_id",
                        "path": "/users/:id",
                        "methods": ["GET"],
                        "code": "return { id = request.params.id }"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["path"], "/users/:id");
}

// ============================================================================
// Script in Nonexistent Database
// ============================================================================

#[tokio::test]
async fn test_create_script_in_nonexistent_db() {
    let (app, _tmp) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/nodb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "test",
                        "path": "/test",
                        "methods": ["GET"],
                        "code": "return {}"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_list_scripts_in_nonexistent_db() {
    let (app, _tmp) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/database/nodb/scripts")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Duplicate Script Prevention
// ============================================================================

#[tokio::test]
async fn test_create_duplicate_script_path() {
    let (app, _tmp) = create_test_app();

    setup_db(&app, "scriptdb").await;

    // Create first script
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/scriptdb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "first",
                        "path": "/samepath",
                        "methods": ["GET"],
                        "code": "return { version = 1 }"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Try to create duplicate
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/scriptdb/scripts")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    json!({
                        "name": "second",
                        "path": "/samepath",
                        "methods": ["POST"],
                        "code": "return { version = 2 }"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should fail - duplicate path in same scope
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
