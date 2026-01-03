//! AI Handlers API Integration Tests
//!
//! Tests the AI contribution pipeline HTTP endpoints

use axum::{body::Body, http::{Request, StatusCode}};
use solidb::storage::StorageEngine;
use solidb::server::routes::create_router;
use solidb::scripting::ScriptStats;
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

const TEST_PASSWORD: &str = "testpassword123";

use std::sync::Once;
static INIT: Once = Once::new();

fn init_env() {
    INIT.call_once(|| {
        std::env::set_var("SOLIDB_ADMIN_PASSWORD", TEST_PASSWORD);
    });
}

struct TestContext {
    app: axum::Router,
    token: String,
    #[allow(dead_code)]
    tmp: TempDir,
}

impl TestContext {
    async fn new() -> Self {
        init_env();

        let tmp = TempDir::new().expect("Failed to create temp dir");
        let engine = StorageEngine::new(tmp.path().to_str().unwrap())
            .expect("Failed to create storage engine");
        engine.initialize().expect("Failed to initialize storage engine");
        let script_stats = Arc::new(ScriptStats::default());
        let app = create_router(engine, None, None, None, None, script_stats, 0);

        // Get auth token
        let response = app.clone().oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header("Content-Type", "application/json")
                .body(Body::from(json!({
                    "username": "admin",
                    "password": TEST_PASSWORD
                }).to_string())).unwrap(),
        ).await.unwrap();

        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024*1024).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap_or(json!({}));

        if !status.is_success() {
            eprintln!("Login failed with status {:?}: {:?}", status, json);
        }

        let token = json["token"].as_str().unwrap_or("").to_string();
        if token.is_empty() {
            eprintln!("Warning: Got empty token, login response: {:?}", json);
        }

        // Create test database
        let db_response = app.clone().oneshot(
            Request::builder()
                .method("POST").uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::from(json!({"name": "testdb"}).to_string())).unwrap(),
        ).await.unwrap();

        let db_status = db_response.status();
        if !db_status.is_success() {
            let body = axum::body::to_bytes(db_response.into_body(), 1024*1024).await.unwrap();
            let json: Value = serde_json::from_slice(&body).unwrap_or(json!({}));
            eprintln!("Failed to create testdb: {} - {:?}", db_status, json);
        }

        TestContext { app, token, tmp }
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    async fn post(&self, uri: &str, body: Value) -> (StatusCode, Value) {
        let response = self.app.clone().oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("Content-Type", "application/json")
                .header("Authorization", self.auth_header())
                .body(Body::from(body.to_string())).unwrap(),
        ).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024*1024).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap_or(json!(null));
        if !status.is_success() {
            eprintln!("POST {} failed with {}: {:?}", uri, status, json);
        }
        (status, json)
    }

    async fn get(&self, uri: &str) -> (StatusCode, Value) {
        let response = self.app.clone().oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("Authorization", self.auth_header())
                .body(Body::empty()).unwrap(),
        ).await.unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024*1024).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap_or(json!(null));
        (status, json)
    }
}

// ============================================================================
// Contribution Submission Tests
// ============================================================================

#[tokio::test]
async fn test_submit_contribution_feature() {
    let ctx = TestContext::new().await;

    let (status, json) = ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "feature",
        "description": "Add a new CONTAINS() function to SDBQL"
    })).await;

    assert_eq!(status, StatusCode::OK);
    // Note: submit response has "id" field (not "_key") since it's a dedicated response struct
    assert!(json["id"].is_string(), "Response should contain contribution id");
    assert_eq!(json["status"], "success", "Response status should be 'success'");
    assert!(json["message"].is_string(), "Response should contain message");
}

#[tokio::test]
async fn test_submit_contribution_bugfix() {
    let ctx = TestContext::new().await;

    let (status, json) = ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "bugfix",
        "description": "Fix null handling in SORT function",
        "context": {
            "related_collections": ["test_data"],
            "priority": "high"
        }
    })).await;

    assert_eq!(status, StatusCode::OK);
    assert!(json["id"].is_string());
    assert_eq!(json["status"], "success");
}

#[tokio::test]
async fn test_submit_contribution_invalid_type() {
    let ctx = TestContext::new().await;

    let (status, _json) = ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "invalid_type",
        "description": "Test invalid type"
    })).await;

    // Axum returns 422 Unprocessable Entity for deserialization errors
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn test_submit_contribution_missing_description() {
    let ctx = TestContext::new().await;

    let (status, _json) = ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "feature"
    })).await;

    // Axum returns 422 Unprocessable Entity for missing required fields
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ============================================================================
// List Contributions Tests
// ============================================================================

#[tokio::test]
async fn test_list_contributions_empty() {
    let ctx = TestContext::new().await;

    let (status, json) = ctx.get("/_api/database/testdb/ai/contributions").await;

    assert_eq!(status, StatusCode::OK);
    assert!(json["contributions"].is_array());
    assert_eq!(json["total"], 0);
}

#[tokio::test]
async fn test_list_contributions_with_data() {
    let ctx = TestContext::new().await;

    // Submit two contributions
    ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "feature",
        "description": "First feature",
            })).await;

    ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "bugfix",
        "description": "A bugfix",
            })).await;

    let (status, json) = ctx.get("/_api/database/testdb/ai/contributions").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["total"], 2);
    assert_eq!(json["contributions"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_list_contributions_with_limit() {
    let ctx = TestContext::new().await;

    // Submit three contributions
    for i in 0..3 {
        ctx.post("/_api/database/testdb/ai/contributions", json!({
            "type": "feature",
            "description": format!("Feature {}", i)
        })).await;
    }

    let (status, json) = ctx.get("/_api/database/testdb/ai/contributions?limit=2").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["contributions"].as_array().unwrap().len(), 2);
}

// ============================================================================
// Get Contribution Tests
// ============================================================================

#[tokio::test]
async fn test_get_contribution() {
    let ctx = TestContext::new().await;

    // Submit a contribution
    let (_, submit_json) = ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "feature",
        "description": "Test feature"
    })).await;
    let contrib_id = submit_json["id"].as_str().unwrap();

    // Get the contribution
    let (status, json) = ctx.get(&format!("/_api/database/testdb/ai/contributions/{}", contrib_id)).await;

    assert_eq!(status, StatusCode::OK);
    // Note: Contribution struct uses "_key" as id field name
    assert_eq!(json["_key"], contrib_id);
    assert_eq!(json["description"], "Test feature");
    assert_eq!(json["status"], "submitted");
}

#[tokio::test]
async fn test_get_contribution_not_found() {
    let ctx = TestContext::new().await;

    let (status, _json) = ctx.get("/_api/database/testdb/ai/contributions/nonexistent-id").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

// ============================================================================
// AI Tasks Tests
// ============================================================================

#[tokio::test]
async fn test_list_tasks_empty() {
    let ctx = TestContext::new().await;

    let (status, json) = ctx.get("/_api/database/testdb/ai/tasks").await;

    assert_eq!(status, StatusCode::OK);
    assert!(json["tasks"].is_array());
    assert_eq!(json["total"], 0);
}

#[tokio::test]
async fn test_task_created_on_contribution() {
    let ctx = TestContext::new().await;

    // Submit a contribution - should create an analysis task
    ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "feature",
        "description": "New feature"    })).await;

    // List tasks
    let (status, json) = ctx.get("/_api/database/testdb/ai/tasks").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["total"], 1);

    let task = &json["tasks"][0];
    assert_eq!(task["task_type"], "analyze_contribution");
    assert_eq!(task["status"], "pending");
}

#[tokio::test]
async fn test_claim_task() {
    let ctx = TestContext::new().await;

    // Submit a contribution
    ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "feature",
        "description": "Test feature"
    })).await;

    // Get the task ID from the tasks list (note: field is "_key" not "id")
    let (_, tasks_json) = ctx.get("/_api/database/testdb/ai/tasks?status=pending").await;
    let task_id = tasks_json["tasks"][0]["_key"].as_str().unwrap();

    // Claim the task
    let (status, json) = ctx.post(
        &format!("/_api/database/testdb/ai/tasks/{}/claim", task_id),
        json!({"agent_id": "test-agent-001"})
    ).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "running");
    assert_eq!(json["agent_id"], "test-agent-001");
}

#[tokio::test]
async fn test_complete_task_creates_next_task() {
    let ctx = TestContext::new().await;

    // Submit a contribution
    ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "feature",
        "description": "Test feature"
    })).await;

    // Get the task ID from the tasks list (note: field is "_key" not "id")
    let (_, tasks_json) = ctx.get("/_api/database/testdb/ai/tasks?status=pending").await;
    let task_id = tasks_json["tasks"][0]["_key"].as_str().unwrap().to_string();

    // Claim the task
    ctx.post(
        &format!("/_api/database/testdb/ai/tasks/{}/claim", task_id),
        json!({"agent_id": "analyzer-001"})
    ).await;

    // Complete the task with low-risk analysis
    let (status, json) = ctx.post(
        &format!("/_api/database/testdb/ai/tasks/{}/complete", task_id),
        json!({
            "output": {
                "risk_score": 0.2,
                "requires_review": false,
                "affected_files": ["src/test.rs"]
            }
        })
    ).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["task"]["status"], "completed");

    // Check that a GenerateCode task was created
    let (_, tasks_json) = ctx.get("/_api/database/testdb/ai/tasks?status=pending").await;

    let pending_tasks = tasks_json["tasks"].as_array().unwrap();
    assert!(pending_tasks.iter().any(|t| t["task_type"] == "generate_code"));
}

// ============================================================================
// AI Agents Tests
// ============================================================================

#[tokio::test]
async fn test_register_agent() {
    let ctx = TestContext::new().await;

    let (status, json) = ctx.post("/_api/database/testdb/ai/agents", json!({
        "name": "test-analyzer",
        "agent_type": "analyzer",
        "capabilities": ["rust", "lua"]
    })).await;

    assert_eq!(status, StatusCode::OK);
    // Agent struct uses "_key" as id field
    assert!(json["_key"].is_string());
    assert_eq!(json["name"], "test-analyzer");
    assert_eq!(json["agent_type"], "analyzer");
    assert_eq!(json["status"], "idle");
}

#[tokio::test]
async fn test_list_agents() {
    let ctx = TestContext::new().await;

    // Register two agents
    ctx.post("/_api/database/testdb/ai/agents", json!({
        "name": "analyzer-1",
        "agent_type": "analyzer"
    })).await;

    ctx.post("/_api/database/testdb/ai/agents", json!({
        "name": "coder-1",
        "agent_type": "coder"
    })).await;

    let (status, json) = ctx.get("/_api/database/testdb/ai/agents").await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["total"], 2);
}

// ============================================================================
// Contribution Review Tests
// ============================================================================

#[tokio::test]
async fn test_reject_contribution() {
    let ctx = TestContext::new().await;

    // Submit a contribution
    let (_, submit_json) = ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "feature",
        "description": "Test feature"    })).await;
    let contrib_id = submit_json["id"].as_str().unwrap();

    // Reject the contribution
    let (status, json) = ctx.post(
        &format!("/_api/database/testdb/ai/contributions/{}/reject", contrib_id),
        json!({
            "reviewer": "admin@example.com",
            "reason": "Not aligned with project goals"
        })
    ).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["status"], "rejected");
}

// ============================================================================
// Pipeline Flow Tests
// ============================================================================

#[tokio::test]
async fn test_full_pipeline_low_risk() {
    let ctx = TestContext::new().await;

    // 1. Submit contribution
    let (_, submit_json) = ctx.post("/_api/database/testdb/ai/contributions", json!({
        "type": "documentation",
        "description": "Update README"
    })).await;
    let contrib_id = submit_json["id"].as_str().unwrap();

    // Helper to get pending task of specific type (uses snake_case for task types)
    async fn get_pending_task(ctx: &TestContext, task_type: &str) -> Option<String> {
        let (_, json) = ctx.get("/_api/database/testdb/ai/tasks?status=pending").await;
        json["tasks"].as_array()
            .and_then(|tasks| tasks.iter().find(|t| t["task_type"] == task_type))
            .and_then(|t| t["_key"].as_str().map(String::from))
    }

    // Helper to claim and complete a task
    async fn process_task(ctx: &TestContext, task_id: &str, output: Value) {
        // Claim
        ctx.post(
            &format!("/_api/database/testdb/ai/tasks/{}/claim", task_id),
            json!({"agent_id": "test-agent"})
        ).await;

        // Complete
        ctx.post(
            &format!("/_api/database/testdb/ai/tasks/{}/complete", task_id),
            json!({"output": output})
        ).await;
    }

    // 2. Complete analyze_contribution task
    let task_id = get_pending_task(&ctx, "analyze_contribution").await.unwrap();
    process_task(&ctx, &task_id, json!({
        "risk_score": 0.1,
        "requires_review": false,
        "affected_files": ["README.md"]
    })).await;

    // 3. Complete generate_code task
    let task_id = get_pending_task(&ctx, "generate_code").await.unwrap();
    process_task(&ctx, &task_id, json!({
        "files": [{"path": "README.md", "content": "# Updated"}]
    })).await;

    // 4. Complete validate_code task
    let task_id = get_pending_task(&ctx, "validate_code").await.unwrap();
    process_task(&ctx, &task_id, json!({
        "passed": true,
        "stages": []
    })).await;

    // 5. Complete run_tests task
    let task_id = get_pending_task(&ctx, "run_tests").await.unwrap();
    process_task(&ctx, &task_id, json!({
        "passed": true,
        "tests_run": 5,
        "tests_passed": 5
    })).await;

    // 6. Low-risk should auto-approve and create merge_changes task
    let task_id = get_pending_task(&ctx, "merge_changes").await.unwrap();
    process_task(&ctx, &task_id, json!({})).await;

    // 7. Verify final status is merged (lowercase)
    let (_, json) = ctx.get(&format!("/_api/database/testdb/ai/contributions/{}", contrib_id)).await;
    assert_eq!(json["status"], "merged");
}
