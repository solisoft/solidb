use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use solidb::server::create_router;
use solidb::StorageEngine;
use tempfile::TempDir;
use tower::ServiceExt;

async fn create_test_server() -> (axum::Router, TempDir) {
    std::env::set_var("SOLIDB_ADMIN_PASSWORD", "admin");
    let temp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(temp_dir.path()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("testdb".to_string()).unwrap();
    let db = engine.get_database("testdb").unwrap();
    db.create_collection("users".to_string(), None).unwrap();
    let router = create_router(engine, None, None, None, None, 0);
    (router, temp_dir)
}

async fn parse_json_response(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn test_limit_bind_var() {
    let (app, _dir) = create_test_server().await;

    // Insert 5 users
    for i in 1..=5 {
        let req = Request::builder()
            .method("POST")
            .uri("/_api/database/testdb/document/users")
            .header("Authorization", "Basic YWRtaW46YWRtaW4=")
            .header("content-type", "application/json")
            .body(Body::from(json!({"name": "User", "i": i}).to_string()))
            .unwrap();
        app.clone().oneshot(req).await.unwrap();
    }

    // Query with LIMIT @limit
    let query = json!({
        "query": "FOR u IN users SORT u.i ASC LIMIT @limit RETURN u.i",
        "bindVars": {
            "limit": 3
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/_api/database/testdb/cursor")
        .header("Authorization", "Basic YWRtaW46YWRtaW4=")
        .header("content-type", "application/json")
        .body(Body::from(query.to_string()))
        .unwrap();
    
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let result: Value = parse_json_response(resp).await;
    
    if let Some(err) = result.get("error") {
        panic!("Query failed: {:?} - {:?}", err, result.get("errorMessage"));
    }
    
    let res_arr = result["result"].as_array().expect("Result array missing");
    assert_eq!(res_arr.len(), 3);
    assert_eq!(res_arr[0], 1);
    assert_eq!(res_arr[2], 3);
}
