//! Timeseries Tests
//!
//! Verifies timeseries collection features: pruning and SDBQL time functions.

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
use uuid::Uuid;

fn create_test_app() -> (axum::Router, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    
    let script_stats = Arc::new(ScriptStats::default());
    
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

// Helper to construct UUIDv7 given a timestamp
fn make_uuid_v7(ts_ms: u64) -> String {
    // UUIDv7: 48 bits timestamp | 4 bits ver (7) | 12 bits rand_a | 2 bits var | 62 bits rand_b
    // We create a UUID with ver=7 and rest zeros (except var which should normally be 10.. but for tests zeros work if we only care about time order).
    // Actually UUID crate parses correctly if version bits are correct.
    // (ts << 80) | (7 << 76).
    let u_int = (ts_ms as u128) << 80 | (7 << 76);
    Uuid::from_u128(u_int).to_string()
}

#[tokio::test]
async fn test_timeseries_prune() {
    let (app, _tmp) = create_test_app();
    
    // 1. Create Timeseries Collection
    app.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/_api/database")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ "name": "ts_db" }).to_string())).unwrap()
    ).await.unwrap();

    app.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/_api/database/ts_db/collection")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ 
            "name": "metrics",
            "type": "timeseries"
        }).to_string())).unwrap()
    ).await.unwrap();

    // 2. Insert Data
    // T1: 1000 ms = 1s (Era 1970)
    // T2: 2000 ms
    // T3: 3000 ms
    let k1 = make_uuid_v7(1000);
    let k2 = make_uuid_v7(2000);
    let k3 = make_uuid_v7(3000);
    
    // Insert docs
    for k in [&k1, &k2, &k3] {
        let resp = app.clone().oneshot(Request::builder()
            .method("POST")
            .uri("/_api/database/ts_db/document/metrics")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({ 
                "_key": k, 
                "val": 1 
            }).to_string())).unwrap()
        ).await.unwrap();
        if resp.status() != StatusCode::OK {
             let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
             eprintln!("Insert failed for {}: {:?}", k, std::str::from_utf8(&body));
        }
    }
    
    // Verify count = 3
    let response = app.clone().oneshot(Request::builder()
        .method("GET")
        .uri("/_api/database/ts_db/collection/metrics/count")
        .body(Body::empty()).unwrap()
    ).await.unwrap();
    let json = response_json(response).await;
    eprintln!("Count response: {:?}", json);
    assert_eq!(json["count"], 3);
    
    // 3. Prune older than 2500ms
    // 3. Prune older than 2500ms
    let older_than = "1970-01-01T00:00:02.500Z";
    
    let response = app.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/_api/database/ts_db/collection/metrics/prune")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ 
            "older_than": older_than
        }).to_string())).unwrap()
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    eprintln!("Prune response: {:?}", json);
    assert_eq!(json["deleted"], 2);
    
    // 4. Verify T3 remains
    let response = app.clone().oneshot(Request::builder()
        .method("GET")
        .uri(format!("/_api/database/ts_db/document/metrics/{}", k3))
        .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    
    // Verify T1 gone
    let response = app.clone().oneshot(Request::builder()
        .method("GET")
        .uri(format!("/_api/database/ts_db/document/metrics/{}", k1))
        .body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    
    // Verify count = 1
    let response = app.clone().oneshot(Request::builder()
        .method("GET")
        .uri("/_api/database/ts_db/collection/metrics/count")
        .body(Body::empty()).unwrap()
    ).await.unwrap();
    let json = response_json(response).await;
    assert_eq!(json["count"], 1);
}

#[tokio::test]
async fn test_sdbql_time_bucket() {
    let (app, _tmp) = create_test_app();
    
    // Create DB
    app.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/_api/database")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ "name": "query_db" }).to_string())).unwrap()
    ).await.unwrap();

    // Execute Query
    let query = "RETURN TIME_BUCKET(100500, '1s')";
    let response = app.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/_api/database/query_db/cursor")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ "query": query }).to_string())).unwrap()
    ).await.unwrap();
    
    let json = response_json(response).await;
    eprintln!("Query 1 result: {:?}", json);
    
    if let Some(err) = json.get("error") {
        if err.as_bool().unwrap_or(false) {
             panic!("Query failed: {:?}", json);
        }
    }

    let result = &json["result"][0];
    assert_eq!(result.as_u64().unwrap(), 100000);
    
    // Test ISO String
    let query = "RETURN TIME_BUCKET('1970-01-01T00:00:02.500Z', '1s')";
    let response = app.clone().oneshot(Request::builder()
        .method("POST")
        .uri("/_api/database/query_db/cursor")
        .header("Content-Type", "application/json")
        .body(Body::from(json!({ "query": query }).to_string())).unwrap()
    ).await.unwrap();
    
    let json = response_json(response).await;
    eprintln!("Query 2 result: {:?}", json);
    let result = &json["result"][0];
    // Existing implementation returns ISO8601 string if input is string
    assert_eq!(result.as_str().unwrap(), "1970-01-01T00:00:02+00:00");
}
