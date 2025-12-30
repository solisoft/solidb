//! Queue Handlers API Tests

use axum::{body::Body, http::{Request, StatusCode}};
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
    let router = create_router(engine, None, None, None, None, script_stats, 0);
    (router, tmp_dir)
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024*1024).await.unwrap();
    serde_json::from_slice(&body).unwrap_or(json!(null))
}

async fn setup_test_db(app: &axum::Router) {
    let _ = app.clone().oneshot(
        Request::builder()
            .method("POST").uri("/_api/database")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({"name": "testdb"}).to_string())).unwrap(),
    ).await.unwrap();
}

// ============================================================================
// List Queues Tests
// ============================================================================

#[tokio::test]
async fn test_list_queues_empty() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    let response = app.oneshot(
        Request::builder().method("GET").uri("/_api/database/testdb/queues")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json.is_array());
    assert_eq!(json.as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_list_queues_with_jobs() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    // Enqueue jobs to create queues
    let _ = app.clone().oneshot(
        Request::builder().method("POST").uri("/_api/database/testdb/queues/orders/enqueue")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({"script": "process_order"}).to_string())).unwrap(),
    ).await.unwrap();
    
    let _ = app.clone().oneshot(
        Request::builder().method("POST").uri("/_api/database/testdb/queues/emails/enqueue")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({"script": "send_email"}).to_string())).unwrap(),
    ).await.unwrap();
    
    let response = app.oneshot(
        Request::builder().method("GET").uri("/_api/database/testdb/queues")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json.as_array().unwrap().len(), 2);
}

// ============================================================================
// Enqueue Job Tests
// ============================================================================

#[tokio::test]
async fn test_enqueue_job_basic() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    let response = app.oneshot(
        Request::builder().method("POST").uri("/_api/database/testdb/queues/default/enqueue")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({"script": "my_script", "params": {"key": "value"}}).to_string())).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json["job_id"].is_string());
}

#[tokio::test]
async fn test_enqueue_job_with_priority() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    let response = app.oneshot(
        Request::builder().method("POST").uri("/_api/database/testdb/queues/priority_queue/enqueue")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({"script": "urgent_task", "priority": 100, "max_retries": 5}).to_string())).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_enqueue_job_nonexistent_db() {
    let (app, _tmp) = create_test_app();
    
    let response = app.oneshot(
        Request::builder().method("POST").uri("/_api/database/nonexistent/queues/default/enqueue")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({"script": "my_script"}).to_string())).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// List Jobs Tests
// ============================================================================

#[tokio::test]
async fn test_list_jobs_with_pagination() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    for i in 0..5 {
        let _ = app.clone().oneshot(
            Request::builder().method("POST").uri("/_api/database/testdb/queues/batch/enqueue")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({"script": format!("script_{}", i)}).to_string())).unwrap(),
        ).await.unwrap();
    }
    
    let response = app.clone().oneshot(
        Request::builder().method("GET").uri("/_api/database/testdb/queues/batch/jobs?limit=2")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["jobs"].as_array().unwrap().len(), 2);
    assert_eq!(json["total"], 5);
}

// ============================================================================
// Cancel Job Tests
// ============================================================================

#[tokio::test]
async fn test_cancel_pending_job() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    let enqueue_response = app.clone().oneshot(
        Request::builder().method("POST").uri("/_api/database/testdb/queues/cancelable/enqueue")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({"script": "cancelable_script"}).to_string())).unwrap(),
    ).await.unwrap();
    
    let enqueue_json = response_json(enqueue_response).await;
    let job_id = enqueue_json["job_id"].as_str().unwrap();
    
    let response = app.clone().oneshot(
        Request::builder().method("DELETE")
            .uri(&format!("/_api/database/testdb/queues/jobs/{}", job_id))
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["success"], true);
}

#[tokio::test]
async fn test_cancel_nonexistent_job() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    // Create jobs collection first
    let _ = app.clone().oneshot(
        Request::builder().method("POST").uri("/_api/database/testdb/queues/dummy/enqueue")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({"script": "dummy"}).to_string())).unwrap(),
    ).await.unwrap();
    
    let response = app.oneshot(
        Request::builder().method("DELETE").uri("/_api/database/testdb/queues/jobs/nonexistent-id")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ============================================================================
// Cron Jobs Tests
// ============================================================================

#[tokio::test]
async fn test_list_cron_jobs_empty() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    let response = app.oneshot(
        Request::builder().method("GET").uri("/_api/database/testdb/cron")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json.is_array());
}

#[tokio::test]
async fn test_create_cron_job() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    let response = app.clone().oneshot(
        Request::builder().method("POST").uri("/_api/database/testdb/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({
                "name": "Daily Report",
                "cron_expression": "0 0 0 * * * *",
                "script": "generate_report",
                "priority": 10,
                "queue": "reports"
            }).to_string())).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert!(json["_key"].is_string());
    assert_eq!(json["name"], "Daily Report");
}

#[tokio::test]
async fn test_create_cron_job_invalid_expression() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    let response = app.oneshot(
        Request::builder().method("POST").uri("/_api/database/testdb/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({
                "name": "Bad Job",
                "cron_expression": "invalid",
                "script": "some_script"
            }).to_string())).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_cron_job() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    let create_response = app.clone().oneshot(
        Request::builder().method("POST").uri("/_api/database/testdb/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({
                "name": "Original",
                "cron_expression": "0 0 * * * * *",
                "script": "original_script"
            }).to_string())).unwrap(),
    ).await.unwrap();
    
    let create_json = response_json(create_response).await;
    let job_id = create_json["_key"].as_str().unwrap();
    
    let response = app.oneshot(
        Request::builder().method("PUT")
            .uri(&format!("/_api/database/testdb/cron/{}", job_id))
            .header("Content-Type", "application/json")
            .body(Body::from(json!({"name": "Updated", "priority": 50}).to_string())).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    assert_eq!(json["name"], "Updated");
}

#[tokio::test]
async fn test_delete_cron_job() {
    let (app, _tmp) = create_test_app();
    setup_test_db(&app).await;
    
    let create_response = app.clone().oneshot(
        Request::builder().method("POST").uri("/_api/database/testdb/cron")
            .header("Content-Type", "application/json")
            .body(Body::from(json!({
                "name": "To Delete",
                "cron_expression": "0 0 * * * * *",
                "script": "delete_me"
            }).to_string())).unwrap(),
    ).await.unwrap();
    
    let create_json = response_json(create_response).await;
    let job_id = create_json["_key"].as_str().unwrap();
    
    let response = app.oneshot(
        Request::builder().method("DELETE")
            .uri(&format!("/_api/database/testdb/cron/{}", job_id))
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_queue_operations_nonexistent_database() {
    let (app, _tmp) = create_test_app();
    
    let response = app.oneshot(
        Request::builder().method("GET").uri("/_api/database/nonexistent/queues")
            .body(Body::empty()).unwrap(),
    ).await.unwrap();
    
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
