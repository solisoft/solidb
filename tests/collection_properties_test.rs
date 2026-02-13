//! Collection Properties Tests
//!
//! Verifies updating collection properties (replication, sharding) and enforcement of constraints.

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

    // No coordinator -> 1 healthy node assumed
    let router = create_router(engine, None, None, None, None, script_stats, None, None, 0);

    // Create a JWT token for authentication
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
async fn test_update_replication_factor_capped() {
    let (app, _tmp, token) = create_test_app();

    // 1. Create DB and Collection
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "prop_db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/prop_db/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "test_col" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // 2. Update Properties: RF=5 (Should be capped to 1)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/_api/database/prop_db/collection/test_col/properties")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "replication_factor": 5,
                        "num_shards": 2 // Also capped to 1
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;

    let config = &json["shardConfig"];
    assert_eq!(
        config["replication_factor"], 1,
        "Replication factor should be capped to 1 node"
    );
    assert_eq!(
        config["num_shards"], 1,
        "Num shards should be capped to 1 node"
    );
}

#[tokio::test]
async fn test_update_collection_type() {
    let (app, _tmp, token) = create_test_app();

    // 1. Setup
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "type_db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/type_db/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "edge_col" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // 2. Update Type to "edge" (Wait, default is document?)
    // Actually create collection defaults to document.
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/_api/database/type_db/collection/edge_col/properties")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "type": "edge"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 3. Verify via Get (Sharding endpoint return type)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/database/type_db/collection/edge_col/sharding")
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let json = response_json(response).await;
    assert_eq!(json["type"], "edge");
}
