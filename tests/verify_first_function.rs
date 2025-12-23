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
async fn test_first_function() {
    let (app, _dir) = create_test_server().await;

    // Test FIRST([1, 2, 3])
    let query = "RETURN FIRST([1, 2, 3])";
    let req = Request::builder()
        .method("POST")
        .uri("/_api/database/testdb/cursor")
        .header("Authorization", "Basic YWRtaW46YWRtaW4=")
        .header("content-type", "application/json")
        .body(Body::from(json!({"query": query}).to_string()))
        .unwrap();
    
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let result: Value = parse_json_response(resp).await;
    
    if let Some(err) = result.get("error") {
        panic!("Query 1 failed: {:?} - {:?}", err, result.get("errorMessage"));
    }
    
    assert_eq!(result["result"][0], 1);

    // Test FIRST([]) -> NULL
    let query2 = "RETURN FIRST([])";
    let req2 = Request::builder()
        .method("POST")
        .uri("/_api/database/testdb/cursor")
        .header("Authorization", "Basic YWRtaW46YWRtaW4=")
        .header("content-type", "application/json")
        .body(Body::from(json!({"query": query2}).to_string()))
        .unwrap();
        
    let resp2 = app.clone().oneshot(req2).await.unwrap();
    let result2: Value = parse_json_response(resp2).await;
    assert!(result2["result"][0].is_null());

    // Test FIRST(FOR u IN users SORT u.age RETURN u.age)
    // Should return 25 (Charlie's age is 25, others > 25 from previous insertion logic? 
    // Wait, users from THIS test execution are clean? No, create_test_server makes fresh DB.
    // I need to insert data first! I forgot to insert data in this test file.
    let users = vec![
        json!({"name": "A", "age": 10}),
        json!({"name": "B", "age": 20}),
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

    let query3 = "RETURN FIRST(FOR u IN users SORT u.age DESC RETURN u.age)";
    let req3 = Request::builder()
        .method("POST")
        .uri("/_api/database/testdb/cursor")
        .header("Authorization", "Basic YWRtaW46YWRtaW4=")
        .header("content-type", "application/json")
        .body(Body::from(json!({"query": query3}).to_string()))
        .unwrap();
        
    let resp3 = app.clone().oneshot(req3).await.unwrap();
    let result3: Value = parse_json_response(resp3).await;
    
    if let Some(err) = result3.get("error") {
        panic!("Query 3 failed: {:?} - {:?}", err, result3.get("errorMessage"));
    }
    
    assert_eq!(result3["result"][0], 20);
}
