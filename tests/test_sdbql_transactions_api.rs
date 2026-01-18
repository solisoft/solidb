//! SDBQL Transaction API Tests
//!
//! Verifies:
//! - Transaction lifecycle (Begin, Commit, Rollback)
//! - Transactional SDBQL execution
//! - Isolation (Visibility)

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
    engine
        .initialize()
        .expect("Failed to initialize storage engine");

    let script_stats = Arc::new(ScriptStats::default());

    let router = create_router(engine, None, None, None, None, script_stats, None, 0);

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
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn test_sdbql_transaction_commit() {
    let (app, _tmp, token) = create_test_app();

    // 1. Setup DB and Collection
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "tx_db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "users" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // 2. Begin Transaction
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db/transaction/begin")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({ "isolation": "read_committed" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let tx_id = json["id"].as_str().unwrap().to_string();

    // 3. Execute Transactional SDBQL Insert
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/_api/database/tx_db/transaction/{}/query", tx_id))
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "INSERT { name: 'Alice' } INTO users"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 4. Verify NOT visible outside transaction
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "FOR u IN users RETURN u"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let json = response_json(response).await;
    let result = json["result"].as_array().unwrap();
    assert_eq!(result.len(), 0, "Data should not be visible before commit");

    // 5. Commit
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!(
                    "/_api/database/tx_db/transaction/{}/commit",
                    tx_id
                ))
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 6. Verify visible NOW
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "FOR u IN users RETURN u"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let json = response_json(response).await;
    let result = json["result"].as_array().unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["name"], "Alice");
}

#[tokio::test]
async fn test_sdbql_transaction_rollback() {
    let (app, _tmp, token) = create_test_app();

    // Setup
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "tx_db_rb" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db_rb/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "items" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Begin
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db_rb/transaction/begin")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    let json = response_json(response).await;
    let tx_id = json["id"].as_str().unwrap().to_string();

    // Insert
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!(
                    "/_api/database/tx_db_rb/transaction/{}/query",
                    tx_id
                ))
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "INSERT { item: 'temp' } INTO items"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Rollback
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!(
                    "/_api/database/tx_db_rb/transaction/{}/rollback",
                    tx_id
                ))
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify empty
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db_rb/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "FOR i IN items RETURN i"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let json = response_json(response).await;
    let result = json["result"].as_array().unwrap();
    assert_eq!(result.len(), 0, "Data should be gone after rollback");
}
