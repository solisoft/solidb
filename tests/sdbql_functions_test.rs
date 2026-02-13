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
    serde_json::from_slice(&body)
        .unwrap_or(json!({"raw": String::from_utf8_lossy(&body).to_string()}))
}

async fn create_db(app: axum::Router, token: &str, name: &str) {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/_api/database")
            .header("Content-Type", "application/json")
            .header("Authorization", auth_header(token))
            .body(Body::from(json!({"name": name}).to_string()))
            .unwrap(),
    )
    .await
    .unwrap();
}

async fn run_query(app: axum::Router, token: &str, db: &str, query: &str) -> Value {
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(&format!("/_api/database/{}/cursor", db))
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(token))
                .body(Body::from(json!({"query": query}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    response_json(response).await
}

#[tokio::test]
async fn test_geo_within() {
    let (app, _tmp, token) = create_test_app();
    create_db(app.clone(), &token, "testdb").await;

    // Test point inside square
    // Square: (0,0) -> (10,0) -> (10,10) -> (0,10)
    let query_inside = r#"
        RETURN GEO_WITHIN(
            {lat: 5, lon: 5},
            [[0,0], [10,0], [10,10], [0,10]]
        )
    "#;
    let result = run_query(app.clone(), &token, "testdb", query_inside).await;
    let rows = result.get("result").unwrap().as_array().unwrap();
    assert_eq!(rows[0], json!(true));

    // Test point outside
    let query_outside = r#"
        RETURN GEO_WITHIN(
            {lat: 15, lon: 5},
            [[0,0], [10,0], [10,10], [0,10]]
        )
    "#;
    let result = run_query(app.clone(), &token, "testdb", query_outside).await;
    let rows = result.get("result").unwrap().as_array().unwrap();
    assert_eq!(rows[0], json!(false));
}

#[tokio::test]
async fn test_highlight() {
    let (app, _tmp, token) = create_test_app();
    create_db(app.clone(), &token, "testdb").await;

    let query = r#"
        RETURN HIGHLIGHT("The quick brown fox", ["quick", "FOX"])
    "#;
    let result = run_query(app.clone(), &token, "testdb", query).await;
    let rows = result.get("result").unwrap().as_array().unwrap();
    // Expect case-insensitive match, wrapping with <b>
    assert_eq!(rows[0], json!("The <b>quick</b> brown <b>fox</b>"));

    // Test overlapping/sub-matches
    let query2 = r#"
        RETURN HIGHLIGHT("banana", ["ana"])
    "#;
    let result2 = run_query(app.clone(), &token, "testdb", query2).await;
    let rows2 = result2.get("result").unwrap().as_array().unwrap();
    // Implementation finds first match "ana" at index 1 -> b<b>ana</b>na
    assert_eq!(rows2[0], json!("b<b>ana</b>na"));
}

#[tokio::test]
async fn test_human_time() {
    let (app, _tmp, token) = create_test_app();
    create_db(app.clone(), &token, "testdb").await;

    // Test "just now" (within 60s)
    let query_now = "RETURN HUMAN_TIME(DATE_NOW())";
    let result = run_query(app.clone(), &token, "testdb", query_now).await;
    let rows = result.get("result").unwrap().as_array().unwrap();
    assert_eq!(rows[0], json!("just now"));

    // Test "minutes ago"
    // Subtract 5 minutes = 300 seconds
    let query_mins = "RETURN HUMAN_TIME(DATE_SUBTRACT(DATE_NOW(), 5, 'minutes'))";
    let result = run_query(app.clone(), &token, "testdb", query_mins).await;
    let rows = result.get("result").unwrap().as_array().unwrap();
    assert_eq!(rows[0], json!("5 minutes ago"));

    // Test "hours ago"
    let query_hours = "RETURN HUMAN_TIME(DATE_SUBTRACT(DATE_NOW(), 2, 'hours'))";
    let result = run_query(app.clone(), &token, "testdb", query_hours).await;
    let rows = result.get("result").unwrap().as_array().unwrap();
    assert_eq!(rows[0], json!("2 hours ago"));

    // Test "days ago"
    let query_days = "RETURN HUMAN_TIME(DATE_SUBTRACT(DATE_NOW(), 3, 'days'))";
    let result = run_query(app.clone(), &token, "testdb", query_days).await;
    let rows = result.get("result").unwrap().as_array().unwrap();
    assert_eq!(rows[0], json!("3 days ago"));

    // Test future
    let query_future = "RETURN HUMAN_TIME(DATE_ADD(DATE_NOW(), 1, 'hour'))";
    let result = run_query(app.clone(), &token, "testdb", query_future).await;
    let rows = result.get("result").unwrap().as_array().unwrap();
    assert_eq!(rows[0], json!("in the future"));
}
