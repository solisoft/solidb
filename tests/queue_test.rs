//! Integration tests for Queue Management system

use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::process::{Child, Command};
use std::thread::sleep;
use std::time::Duration;

const BASE_URL: &str = "http://localhost:16746";

struct TestServer {
    process: Child,
}

impl TestServer {
    fn start() -> Self {
        // Clean up any existing data
        let _ = std::fs::remove_dir_all("./test_queue_data");

        let process = Command::new("cargo")
            .args(["run", "--", "--port", "16746", "--data-dir", "./test_queue_data"])
            .env("SOLIDB_ADMIN_PASSWORD", "admin")
            .spawn()
            .expect("Failed to start server");

        // Wait for server to start
        sleep(Duration::from_secs(30));

        TestServer { process }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = std::fs::remove_dir_all("./test_queue_data");
    }
}

fn get_auth_token(client: &Client) -> String {
    let resp = client
        .post(&format!("{}/auth/login", BASE_URL))
        .json(&json!({"username": "admin", "password": "admin"}))
        .send()
        .expect("Login failed");
    
    let body: Value = resp.json().expect("Failed to parse login response");
    body["token"].as_str().expect("No token in response").to_string()
}

#[test]
fn test_enqueue_and_process_job() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // 1. Create a collection to store the result of the job
    client
        .post(&format!("{}/_api/database/_system/collection", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({"name": "results"}))
        .send()
        .expect("Failed to create collection");

    // 2. Create a script that the job will execute
    client
        .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Process Job",
            "path": "job_processor",
            "methods": ["POST"],
            "code": r#"
                local results = db:collection("results")
                results:insert({
                    job_id = request.path,
                    data = request.body,
                    processed_at = solidb.now()
                })
                return { success = true }
            "#
        }))
        .send()
        .expect("Failed to create script");

    // 3. Enqueue a job via API
    let enqueue_resp = client
        .post(&format!("{}/_api/database/_system/queues/default/enqueue", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "script": "job_processor",
            "params": {"test": "data"},
            "priority": 10
        }))
        .send()
        .expect("Failed to enqueue job");

    assert_eq!(enqueue_resp.status(), 200, "Enqueue failed: {:?}", enqueue_resp.text());
    let enqueue_result: Value = enqueue_resp.json().unwrap();
    let job_id = enqueue_result["job_id"].as_str().unwrap();

    // 4. Wait for the job to be processed (it's background)
    let mut processed = false;
    for _ in 0..20 {
        sleep(Duration::from_secs(1));
        
        // Check job status via API
        let status_resp = client
            .get(&format!("{}/_api/database/_system/queues/default/jobs", BASE_URL))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .expect("Failed to list jobs");
        
        if status_resp.status() != 200 {
             continue;
        }

        let status_result: Value = status_resp.json().unwrap();
        let jobs = status_result["jobs"].as_array().unwrap();
        if let Some(job) = jobs.iter().find(|j| j["_key"] == job_id) {
            if job["status"] == "completed" {
                processed = true;
                break;
            }
        }
    }

    assert!(processed, "Job was not processed in time");

    // 5. Verify the result in the collection
    let results_resp = client
        .post(&format!("{}/_api/database/_system/cursor", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "query": "FOR r IN results FILTER r.data.test == 'data' RETURN r"
        }))
        .send()
        .expect("Failed to query results");

    let results_result: Value = results_resp.json().expect("Failed to parse results");
    assert_eq!(results_result["result"].as_array().unwrap().len(), 1);
}

#[test]
fn test_lua_enqueue() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // 1. Create a script that enqueues another job
    client
        .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Enqueuer",
            "path": "enqueuer",
            "methods": ["POST"],
            "code": r#"
                local job_id = db:enqueue("default", "target", { foo = "bar" }, { priority = 5 })
                return { job_id = job_id }
            "#
        }))
        .send()
        .expect("Failed to create enqueuer script");

    // 2. Create the target script
    client
        .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Target",
            "path": "target",
            "methods": ["POST"],
            "code": r#"
                -- Does nothing
                return { ok = true }
            "#
        }))
        .send()
        .expect("Failed to create target script");

    // 3. Trigger the enqueuer
    let resp = client
        .post(&format!("{}/api/custom/_system/enqueuer", BASE_URL))
        .send()
        .expect("Failed to trigger enqueuer");

    assert_eq!(resp.status(), 200, "Triggering enqueuer failed: {:?}", resp.text());
    let result: Value = resp.json().unwrap();
    let job_id = result["job_id"].as_str().unwrap();

    // 4. Verify job existence in queue
    let mut found = false;
    for _ in 0..10 {
        sleep(Duration::from_secs(1));
        let status_resp = client
            .get(&format!("{}/_api/database/_system/queues/default/jobs", BASE_URL))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .expect("Failed to list jobs");
        
        if status_resp.status() != 200 {
            continue;
        }

        let status_result: Value = status_resp.json().unwrap();
        let jobs = status_result["jobs"].as_array().unwrap();
        if jobs.iter().any(|j| j["_key"] == job_id) {
            found = true;
            break;
        }
    }
    assert!(found, "Job not found in queue");
}

#[test]
fn test_create_cron_job() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // 1. Create the target script that the cron will execute
    client
        .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Cron Target",
            "path": "cron_target",
            "methods": ["POST"],
            "code": r#"
                return { executed = true }
            "#
        }))
        .send()
        .expect("Failed to create cron target script");

    // 2. Create a cron job with a valid 7-field expression (every 30 seconds)
    let cron_resp = client
        .post(&format!("{}/_api/database/_system/cron", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Test Cron Job",
            "cron_expression": "0/30 * * * * * *",
            "script": "cron_target",
            "params": {"from_cron": true},
            "priority": 5
        }))
        .send()
        .expect("Failed to create cron job");

    assert_eq!(cron_resp.status(), 200, "Create cron job failed: {:?}", cron_resp.text());
    let cron_result: Value = cron_resp.json().unwrap();
    assert!(cron_result["id"].as_str().is_some(), "Cron job should have an ID");
    assert_eq!(cron_result["name"], "Test Cron Job");
    assert!(cron_result["next_run"].as_u64().is_some(), "Cron job should have next_run set");

    // 3. List cron jobs and verify it exists
    let list_resp = client
        .get(&format!("{}/_api/database/_system/cron", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .expect("Failed to list cron jobs");

    assert_eq!(list_resp.status(), 200);
    let list_result: Value = list_resp.json().unwrap();
    let cron_jobs = list_result.as_array().unwrap();
    assert_eq!(cron_jobs.len(), 1);
    assert_eq!(cron_jobs[0]["name"], "Test Cron Job");
}

#[test]
fn test_cron_job_spawns_job() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // 1. Create target script
    client
        .post(&format!("{}/_api/database/_system/scripts", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Cron Exec",
            "path": "cron_exec",
            "methods": ["POST"],
            "code": r#"
                return { ran = true }
            "#
        }))
        .send()
        .expect("Failed to create script");

    // 2. Create a cron job that runs every second (for testing)
    let cron_resp = client
        .post(&format!("{}/_api/database/_system/cron", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Frequent Cron",
            "cron_expression": "* * * * * * *",  // Every second
            "script": "cron_exec",
            "params": {},
            "priority": 1
        }))
        .send()
        .expect("Failed to create cron job");

    assert_eq!(cron_resp.status(), 200);

    // 3. Wait for the cron to fire and spawn a job
    let mut job_found = false;
    for _ in 0..15 {
        sleep(Duration::from_secs(1));

        let jobs_resp = client
            .get(&format!("{}/_api/database/_system/queues/default/jobs", BASE_URL))
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .expect("Failed to list jobs");

        if jobs_resp.status() != 200 {
            continue;
        }

        let jobs_result: Value = jobs_resp.json().unwrap();
        let jobs = jobs_result["jobs"].as_array().unwrap();
        
        // Look for a job with script_path = "cron_exec"
        if jobs.iter().any(|j| j["script_path"] == "cron_exec") {
            job_found = true;
            break;
        }
    }

    assert!(job_found, "Cron job did not spawn any jobs within 15 seconds");

    // 4. Verify cron job's last_run was updated
    let list_resp = client
        .get(&format!("{}/_api/database/_system/cron", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .expect("Failed to list cron jobs");

    let list_result: Value = list_resp.json().unwrap();
    let cron_jobs = list_result.as_array().unwrap();
    let cron = cron_jobs.iter().find(|c| c["name"] == "Frequent Cron").unwrap();
    assert!(cron["last_run"].as_u64().is_some(), "Cron job last_run should be set after execution");
}

#[test]
fn test_delete_cron_job() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // 1. Create a cron job
    let cron_resp = client
        .post(&format!("{}/_api/database/_system/cron", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "To Delete",
            "cron_expression": "0 0 * * * * *",  // Every hour
            "script": "nonexistent",
            "params": {},
            "priority": 0
        }))
        .send()
        .expect("Failed to create cron job");

    assert_eq!(cron_resp.status(), 200);
    let cron_result: Value = cron_resp.json().unwrap();
    let cron_id = cron_result["id"].as_str().unwrap();

    // 2. Delete the cron job
    let delete_resp = client
        .delete(&format!("{}/_api/database/_system/cron/{}", BASE_URL, cron_id))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .expect("Failed to delete cron job");

    assert_eq!(delete_resp.status(), 200, "Delete failed: {:?}", delete_resp.text());

    // 3. Verify it's gone
    let list_resp = client
        .get(&format!("{}/_api/database/_system/cron", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .expect("Failed to list cron jobs");

    let list_result: Value = list_resp.json().unwrap();
    let cron_jobs = list_result.as_array().unwrap();
    assert!(!cron_jobs.iter().any(|c| c["id"] == cron_id), "Cron job should be deleted");
}

#[test]
fn test_invalid_cron_expression_rejected() {
    let _server = TestServer::start();
    let client = Client::new();
    let token = get_auth_token(&client);

    // Try to create a cron job with invalid expression (5-field instead of 7)
    let cron_resp = client
        .post(&format!("{}/_api/database/_system/cron", BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .json(&json!({
            "name": "Invalid Cron",
            "cron_expression": "*/5 * * * *",  // Invalid: only 5 fields
            "script": "test",
            "params": {},
            "priority": 0
        }))
        .send()
        .expect("Failed to send request");

    assert_eq!(cron_resp.status(), 400, "Should reject invalid cron expression");
    let error_result: Value = cron_resp.json().unwrap();
    assert!(error_result["error"].as_str().unwrap().contains("Invalid cron expression"));
}
