//! Transaction Handlers Tests
//!
//! Comprehensive tests for server/transaction_handlers.rs including:
//! - Begin/commit/rollback transaction lifecycle
//! - Error handling

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

    let router = create_router(engine, None, None, None, None, script_stats, None, None, 0);

    let token =
        AuthService::create_jwt_with_roles("test_admin", Some(vec!["admin".to_string()]), None)
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
    serde_json::from_slice(&body).unwrap_or(json!({"error": "Invalid JSON"}))
}

async fn setup_db(app: &axum::Router, db_name: &str, token: &str) {
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(token))
                .body(Body::from(json!({ "name": db_name }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
}

// ============================================================================
// Transaction Lifecycle Tests
// ============================================================================

#[tokio::test]
async fn test_begin_transaction() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, "txdb", &token).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/txdb/transaction/begin")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json["id"].is_string());
    assert_eq!(json["status"], "active");
}

#[tokio::test]
async fn test_begin_transaction_with_isolation_level() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, "txdb", &token).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/txdb/transaction/begin")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({ "isolationLevel": "serializable" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json["id"].is_string());
    assert_eq!(json["isolationLevel"], "Serializable");
}

#[tokio::test]
async fn test_commit_transaction() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, "txdb", &token).await;

    // Begin transaction
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/txdb/transaction/begin")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let json = response_json(response).await;
    let tx_id = json["id"].as_str().unwrap();

    // Commit transaction
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/_api/database/txdb/transaction/{}/commit", tx_id))
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["status"], "committed");
}

#[tokio::test]
async fn test_rollback_transaction() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, "txdb", &token).await;

    // Begin transaction
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/txdb/transaction/begin")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let json = response_json(response).await;
    let tx_id = json["id"].as_str().unwrap();

    // Rollback transaction
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!(
                    "/_api/database/txdb/transaction/{}/rollback",
                    tx_id
                ))
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["status"], "aborted");
}

// ============================================================================
// Isolation Level Tests
// ============================================================================

#[tokio::test]
async fn test_various_isolation_levels() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, "txdb", &token).await;

    for level in [
        "read_uncommitted",
        "read_committed",
        "repeatable_read",
        "serializable",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_api/database/txdb/transaction/begin")
                    .header("Content-Type", "application/json")
                    .header("Authorization", auth_header(&token))
                    .body(Body::from(json!({ "isolationLevel": level }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "Failed for level: {}",
            level
        );
        let json = response_json(response).await;
        let tx_id = json["id"].as_str().unwrap();

        // Rollback to clean up
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&format!(
                        "/_api/database/txdb/transaction/{}/rollback",
                        tx_id
                    ))
                    .header("Authorization", auth_header(&token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn test_invalid_isolation_level() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, "txdb", &token).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/txdb/transaction/begin")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({ "isolationLevel": "invalid_level" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status() == StatusCode::BAD_REQUEST
            || response.status() == StatusCode::UNPROCESSABLE_ENTITY
    );
}

// ============================================================================
// Error Handling Tests
// ============================================================================

#[tokio::test]
async fn test_invalid_transaction_id_format() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, "txdb", &token).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/txdb/transaction/not-a-number/commit")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert!(
        response.status() == StatusCode::BAD_REQUEST
            || response.status() == StatusCode::UNPROCESSABLE_ENTITY
    );
}

#[tokio::test]
async fn test_begin_transaction_in_nonexistent_db() {
    let (app, _tmp, token) = create_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/nodb/transaction/begin")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// Note: Double commit behavior is implementation-dependent and not tested here

#[tokio::test]
async fn test_multiple_transactions() {
    let (app, _tmp, token) = create_test_app();

    setup_db(&app, "txdb", &token).await;

    // Begin multiple transactions
    let mut tx_ids = Vec::new();
    for _ in 0..3 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/_api/database/txdb/transaction/begin")
                    .header("Content-Type", "application/json")
                    .header("Authorization", auth_header(&token))
                    .body(Body::from(json!({}).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        tx_ids.push(json["id"].as_str().unwrap().to_string());
    }

    // All should have unique IDs
    let unique_ids: std::collections::HashSet<_> = tx_ids.iter().collect();
    assert_eq!(unique_ids.len(), 3);

    // Rollback all
    for tx_id in tx_ids {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(&format!(
                        "/_api/database/txdb/transaction/{}/rollback",
                        tx_id
                    ))
                    .header("Authorization", auth_header(&token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
