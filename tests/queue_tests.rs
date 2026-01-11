//! Queue Worker Tests
//!
//! Verifies job processing logic:
//! - Enqueueing a job
//! - Execution via QueueWorker
//! - Status updates (Pending -> Running -> Completed)
//! - Error handling and retries

use serde_json::json;
use solidb::queue::{Job, JobStatus, QueueWorker};
use solidb::scripting::ScriptStats;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;

fn create_test_env() -> (Arc<StorageEngine>, Arc<QueueWorker>, TempDir) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = Arc::new(
        StorageEngine::new(tmp_dir.path().to_str().unwrap())
            .expect("Failed to create storage engine"),
    );

    // Create DB
    engine.create_database("testdb".to_string()).unwrap();
    let db = engine.get_database("testdb").unwrap();

    // Create system collections
    db.create_collection("_scripts".to_string(), None).unwrap();
    db.create_collection("_jobs".to_string(), None).unwrap();
    db.create_collection("_cron_jobs".to_string(), None)
        .unwrap();
    db.create_collection("logs".to_string(), None).unwrap(); // For script side effects

    let stats = Arc::new(ScriptStats::default());
    let worker = Arc::new(QueueWorker::new(engine.clone(), stats));

    (engine, worker, tmp_dir)
}

#[tokio::test]
async fn test_job_execution_success() {
    let (engine, worker, _tmp) = create_test_env();
    let db = engine.get_database("testdb").unwrap();

    // 1. Register a script
    let scripts = db.get_collection("_scripts").unwrap();
    let script_code = r#"
        local logs = db:collection("logs")
        logs:insert({ message = "Job executed", params = request.body })
        return { success = true }
    "#;

    scripts
        .insert(json!({
            "_key": "test_script_key",
            "name": "test_script",
            "path": "test_worker",
            "methods": ["POST"],
            "code": script_code,
            "database": "testdb",
            "created_at": "2023-01-01T00:00:00Z",
            "updated_at": "2023-01-01T00:00:00Z",
            "description": "Test script",
            "collection": null
        }))
        .unwrap();

    // 2. Enqueue a job
    let jobs = db.get_collection("_jobs").unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let job_id = uuid::Uuid::new_v4().to_string();
    let job = Job {
        id: job_id.clone(),
        revision: None,
        queue: "default".to_string(),
        priority: 0,
        script_path: "test_worker".to_string(),
        params: json!({ "foo": "bar" }),
        status: JobStatus::Pending,
        retry_count: 0,
        max_retries: 3,
        last_error: None,
        cron_job_id: None,
        run_at: now, // Ready now
        created_at: now,
        started_at: None,
        completed_at: None,
    };

    jobs.insert(serde_json::to_value(&job).unwrap()).unwrap();

    // 3. Run Worker (Manual check)
    worker.check_jobs().await;

    // 4. Poll for completion
    let mut success = false;
    for i in 0..50 {
        // Wait up to 5 seconds
        let doc = jobs.get(&job_id).unwrap();
        let updated_job: Job = serde_json::from_value(doc.to_value()).unwrap();

        if updated_job.status == JobStatus::Completed {
            success = true;
            break;
        }
        if updated_job.status == JobStatus::Failed {
            panic!("Job failed with error: {:?}", updated_job.last_error);
        }

        if i % 10 == 0 {
            println!(
                "Waiting... status={:?}, retry={}",
                updated_job.status, updated_job.retry_count
            );
            if let Some(err) = &updated_job.last_error {
                println!("Last error: {}", err);
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    assert!(success, "Job did not complete in time");

    // 5. Verify Side Effect
    let logs = db.get_collection("logs").unwrap();
    assert_eq!(logs.count(), 1);
    let log_doc = logs.scan(None).pop().unwrap();
    assert_eq!(log_doc.data["message"], "Job executed");
    assert_eq!(log_doc.data["params"]["foo"], "bar");
}

#[tokio::test]
async fn test_cron_scheduling() {
    let (engine, worker, _tmp) = create_test_env();
    let db = engine.get_database("testdb").unwrap();
    let cron_jobs = db.get_collection("_cron_jobs").unwrap();
    let jobs = db.get_collection("_jobs").unwrap();

    // 1. Create a Cron Job
    // Schedule: Runs every second (* * * * * * ?) - using quartz format if supported, or standard cron
    // The code uses `cron::Schedule`, which typically supports 6 or 7 fields.
    // Let's use a schedule that is definitely in the past/now for immediate triggering?
    // "0/1 * * * * * *" (Every second)

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let cron_job = solidb::queue::CronJob {
        id: "cron_1".to_string(),
        revision: None,
        name: "Test Cron".to_string(),
        cron_expression: "0/1 * * * * * *".to_string(), // Every second
        queue: "default".to_string(),
        priority: 10,
        max_retries: 3,
        script_path: "test_worker".to_string(),
        params: json!({}),
        last_run: None,
        next_run: Some(now - 1), // Force it to be due
        created_at: now,
    };

    cron_jobs
        .insert(serde_json::to_value(&cron_job).unwrap())
        .unwrap();

    // 2. Run Cron Check
    worker.check_cron_jobs().await;

    // 3. Verify Job Spawned
    let spawned_jobs = jobs.scan(None);
    assert!(spawned_jobs.len() > 0, "Cron job should have spawned a job");

    let job_doc = &spawned_jobs[0];
    assert_eq!(job_doc.data["cron_job_id"], "cron_1");
    assert_eq!(job_doc.data["status"], "pending");

    // 4. Verify Cron Job Updated
    let updated_cron_doc = cron_jobs.get("cron_1").unwrap();
    let updated_cron: solidb::queue::CronJob =
        serde_json::from_value(updated_cron_doc.to_value()).unwrap();

    assert!(updated_cron.last_run.is_some());
    assert!(updated_cron.next_run.unwrap() > now);
}
