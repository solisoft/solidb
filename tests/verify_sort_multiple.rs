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
    let router = create_router(engine, None, None, None, None, std::sync::Arc::new(solidb::scripting::ScriptStats::default()), 0);
    (router, temp_dir)
}

async fn parse_json_response(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn test_sort_multiple_fields() {
    let (app, _dir) = create_test_server().await;

    // Insert test data
    let users = vec![
        json!({"name": "Alice", "age": 30}),
        json!({"name": "Bob", "age": 30}),
        json!({"name": "Charlie", "age": 25}),
        json!({"name": "Dave", "age": 35}),
    ];

    for user in users {
        let req = Request::builder()
            .method("POST")
            .uri("/_api/database/testdb/document/users")
            .header("Authorization", "Basic YWRtaW46YWRtaW4=")
            .header("content-type", "application/json")
            .body(Body::from(user.to_string()))
            .unwrap();
        app.clone().oneshot(req).await.unwrap();
    }

    // Test 1: Age ASC, Name DESC
    // Expected: Charlie (25), Bob (30), Alice (30), Dave (35)
    let query1 = "FOR u IN users SORT u.age ASC, u.name DESC RETURN u.name";
    let req1 = Request::builder()
        .method("POST")
        .uri("/_api/database/testdb/cursor")
        .header("Authorization", "Basic YWRtaW46YWRtaW4=")
        .header("content-type", "application/json")
        .body(Body::from(json!({"query": query1}).to_string()))
        .unwrap();
    
    let resp1 = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);
    let result1: Value = parse_json_response(resp1).await;
    
    if let Some(err) = result1.get("error") {
        panic!("Query 1 failed: {:?} - {:?}", err, result1.get("errorMessage"));
    }

    let names1: Vec<&str> = result1["result"].as_array().expect("Query 1 result missing")
        .iter().map(|v| v.as_str().unwrap()).collect();
        
    assert_eq!(names1, vec!["Charlie", "Bob", "Alice", "Dave"]);

    // Test 2: Age ASC, Name ASC
    // Expected: Charlie (25), Alice (30), Bob (30), Dave (35)
    let query2 = "FOR u IN users SORT u.age ASC, u.name ASC RETURN u.name";
    let req2 = Request::builder()
        .method("POST")
        .uri("/_api/database/testdb/cursor")
        .header("Authorization", "Basic YWRtaW46YWRtaW4=")
        .header("content-type", "application/json")
        .body(Body::from(json!({"query": query2}).to_string()))
        .unwrap();
    
    let resp2 = app.clone().oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let result2: Value = parse_json_response(resp2).await;
    
    if let Some(err) = result2.get("error") {
        panic!("Query 2 failed: {:?} - {:?}", err, result2.get("errorMessage"));
    }

    let names2: Vec<&str> = result2["result"].as_array().expect("Query 2 result missing")
        .iter().map(|v| v.as_str().unwrap()).collect();
        
    assert_eq!(names2, vec!["Charlie", "Alice", "Bob", "Dave"]);
}
