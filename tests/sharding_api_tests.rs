//! Sharding API Tests
//!
//! Verifies sharding configuration and status endpoints.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use solidb::storage::StorageEngine;
use solidb::server::routes::create_router;
use solidb::scripting::ScriptStats;
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

fn create_test_app() -> (axum::Router, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    
    let script_stats = Arc::new(ScriptStats::default());
    
    // No cluster manager or coordinator for basic API tests
    let router = create_router(
        engine,
        None,
        None,
        None,
        None,
        script_stats,
        0
    );
    
    (router, tmp_dir)
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024*1024).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn test_get_sharding_not_sharded() {
    let (app, _tmp) = create_test_app();
    
    // 1. Create DB and Collection
    app.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/_api/database")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ "name": "shard_db" }).to_string())).unwrap()
    ).await.unwrap();

    app.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/_api/database/shard_db/collection")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ "name": "normal_col" }).to_string())).unwrap()
    ).await.unwrap();

    // 2. Get Sharding Details
    let response = app.clone().oneshot(Request::builder()
        .method("GET")
        .uri("/_api/database/shard_db/collection/normal_col/sharding")
        .body(Body::empty()).unwrap()
    ).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    
    assert_eq!(json["sharded"], false);
    assert!(json["shards"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_enable_sharding() {
    let (app, _tmp) = create_test_app();
    
    // Setup
    app.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/_api/database")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ "name": "shard_db_2" }).to_string())).unwrap()
    ).await.unwrap();

    app.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/_api/database/shard_db_2/collection")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ "name": "sharded_col" }).to_string())).unwrap()
    ).await.unwrap();

    // 3. Update Properties to Enable Sharding
    // Without a coordinator, we might be limited in what we can set (capped to 1 node likely)
    let response = app.clone().oneshot(Request::builder()
        .method("PUT")
        .uri("/_api/database/shard_db_2/collection/sharded_col/properties")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ 
            "num_shards": 1,
            "replication_factor": 1
        }).to_string())).unwrap()
    ).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    // The Update endpoint returns 'shardConfig' object, not 'sharded' boolean
    assert!(json["shardConfig"]["num_shards"].as_u64().unwrap() >= 1);

    // 4. Verify via Sharding Details
    let response = app.clone().oneshot(Request::builder()
        .method("GET")
        .uri("/_api/database/shard_db_2/collection/sharded_col/sharding")
        .body(Body::empty()).unwrap()
    ).await.unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    
    assert_eq!(json["sharded"], true);
    let shards = json["shards"].as_array().unwrap();
    assert_eq!(shards.len(), 1);
    assert_eq!(shards[0]["shard_id"], 0);
    assert_eq!(shards[0]["status"], "healthy"); // Local fallback logic
}
