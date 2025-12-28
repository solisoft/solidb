use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::{create_router, StorageEngine};
use solidb::scripting::ScriptStats;
use std::sync::Arc;
use tempfile::TempDir;
use tower::util::ServiceExt;

// ==================== Helper Functions ====================

/// Helper to create a test app
fn create_test_app() -> (axum::Router, TempDir) {
    // Set admin password for tests
    std::env::set_var("SOLIDB_ADMIN_PASSWORD", "admin");
    
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let storage = StorageEngine::new(temp_dir.path()).expect("Failed to create storage");
    storage.initialize().expect("Failed to initialize storage");
    let router = create_router(storage, None, None, None, None, Arc::new(ScriptStats::default()), 0);
    (router, temp_dir)
}

/// Helper to make a POST request with JSON body and optional token
async fn post_json(app: &axum::Router, path: &str, body: Value, token: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
        
    if let Some(t) = token {
        builder = builder.header("Authorization", format!("Bearer {}", t));
    }

    let response = app
        .clone()
        .oneshot(
            builder
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

/// Helper to make a GET request with optional token
async fn get(app: &axum::Router, path: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("GET")
        .uri(path);

    if let Some(t) = token {
        builder = builder.header("Authorization", format!("Bearer {}", t));
    }

    let response = app
        .clone()
        .oneshot(
            builder
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

/// Helper to make a PUT request with JSON body and optional token
async fn put_json(app: &axum::Router, path: &str, body: Value, token: Option<&str>) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("PUT")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");

    if let Some(t) = token {
        builder = builder.header("Authorization", format!("Bearer {}", t));
    }

    let response = app
        .clone()
        .oneshot(
            builder
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

// ==================== Tests ====================

#[tokio::test]
async fn test_update_collection_properties() {
    let (app, _dir) = create_test_app();
    let db_name = "_system";
    let coll_name = "test_config_coll";

    // 0. Login to get token (initializes default admin if missing)
    let (status, body) = post_json(
        &app,
        "/auth/login",
        json!({
            "username": "admin",
            "password": "admin"
        }),
        None
    ).await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().expect("Login failed, no token").to_string();
    let token_ref = Some(token.as_str());

    // 1. Create collection (default, non-sharded)
    let (status, body) = post_json(
        &app,
        &format!("/_api/database/{}/collection", db_name),
        json!({"name": coll_name}),
        token_ref
    ).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "created");

    // 2. Verify initial state 
    let (status, body) = get(
        &app,
        &format!("/_api/database/{}/collection", db_name),
        token_ref
    ).await;
    assert_eq!(status, StatusCode::OK);
    
    // Check it exists and check assumption about initial config (should be missing or default)
    let collections = body["collections"].as_array().unwrap();
    let coll_info = collections.iter().find(|c| c["name"] == coll_name).expect("Collection not found");
    // println!("Initial config: {:?}", coll_info["shardConfig"]); 

    // 3. Update properties: Set shards to 2, replication to 2
    let (status, body) = put_json(
        &app,
        &format!("/_api/database/{}/collection/{}/properties", db_name, coll_name),
        json!({
            "numShards": 2,
            "replicationFactor": 2
        }),
        token_ref
    ).await;

    assert_eq!(status, StatusCode::OK);
    // Backend returns "updated_rebalancing" if shards changed, or "updated" otherwise. 
    // Since we go from implicit 1 (or 0?) to 2, it should invoke rebalance logic if it detects change.
    // If original was null, code defaults to default config (shards=3). 
    // So if we set to 2, it changes from 3 to 2? Or does it read existing?
    // Implementation: `collection.get_shard_config().unwrap_or_else(|| CollectionShardConfig::default())`
    // Default is 3 shards.
    // So if we update to 2, it changes.
    // HOWEVER, if we created it without options, does `get_shard_config` return None?
    // Handler `create_collection`: only sets config if `req.num_shards` is Some.
    // So for default collection, it is None.
    // So `unwrap_or_else` gives default (3 shards).
    // So updating to 2 means 3 -> 2. Change detected.
    
    assert!(body["status"].as_str().unwrap().contains("updated"));
    assert_eq!(body["shardConfig"]["num_shards"], 2);
    assert_eq!(body["shardConfig"]["replication_factor"], 2);

    // 4. Verify persistence
    let (status, body) = get(
        &app,
        &format!("/_api/database/{}/collection", db_name),
        token_ref
    ).await;
    assert_eq!(status, StatusCode::OK);
    
    let collections = body["collections"].as_array().unwrap();
    let coll_info = collections.iter().find(|c| c["name"] == coll_name).expect("Collection not found");
    
    let config = &coll_info["shardConfig"];
    assert_eq!(config["num_shards"], 2);
    assert_eq!(config["replication_factor"], 2);
}
