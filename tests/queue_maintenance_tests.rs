//! `_jobs` retention and lease recovery (audit M5).

use serde_json::json;
use solidb::queue::{Job, JobStatus, QueueWorker};
use solidb::scripting::ScriptStats;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn job(id: &str, status: JobStatus, started_at: Option<u64>, completed_at: Option<u64>) -> Job {
    Job {
        id: id.to_string(),
        revision: None,
        queue: "default".to_string(),
        priority: 0,
        script_path: "noop".to_string(),
        webhook_url: None,
        webhook_secret: None,
        webhook_headers: None,
        params: json!({}),
        status,
        retry_count: 0,
        max_retries: 3,
        last_error: None,
        cron_job_id: None,
        run_at: 0,
        created_at: 0,
        started_at,
        completed_at,
    }
}

#[tokio::test]
async fn sweep_prunes_old_terminal_jobs_and_requeues_orphans() {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    storage.create_database("testdb".to_string()).unwrap();
    let db = storage.get_database("testdb").unwrap();
    db.create_collection("_jobs".to_string(), None).unwrap();
    let jobs = db.get_collection("_jobs").unwrap();

    let now = now_ms();
    let day = 24 * 3600 * 1000;
    let rows = [
        job(
            "old_done",
            JobStatus::Completed,
            Some(now - 3 * day),
            Some(now - 3 * day),
        ),
        job(
            "old_failed",
            JobStatus::Failed,
            Some(now - 3 * day),
            Some(now - 3 * day),
        ),
        job(
            "new_done",
            JobStatus::Completed,
            Some(now - 1000),
            Some(now - 1000),
        ),
        job("old_pending", JobStatus::Pending, None, None),
        job("orphan", JobStatus::Running, Some(now - day), None),
        job("fresh_running", JobStatus::Running, Some(now - 1000), None),
    ];
    for r in &rows {
        jobs.insert(serde_json::to_value(r).unwrap()).unwrap();
    }
    // A Soli framework row: it uses `state`, never `status`, and must survive.
    jobs.insert(json!({"_key": "soli_row", "state": "done", "completed_at": 0}))
        .unwrap();

    let worker = QueueWorker::new(storage.clone(), Arc::new(ScriptStats::default()));
    // One-day retention, ten-minute lease.
    worker.sweep_jobs_with(24 * 3600, 600).await;

    assert!(jobs.get("old_done").is_err());
    assert!(jobs.get("old_failed").is_err());
    assert!(jobs.get("new_done").is_ok());
    assert!(jobs.get("old_pending").is_ok());
    assert!(jobs.get("soli_row").is_ok());

    let orphan: Job = serde_json::from_value(jobs.get("orphan").unwrap().to_value()).unwrap();
    assert_eq!(orphan.status, JobStatus::Pending);
    assert_eq!(orphan.retry_count, 1);
    assert!(orphan.started_at.is_none());
    assert!(orphan.last_error.is_some());

    let fresh: Job = serde_json::from_value(jobs.get("fresh_running").unwrap().to_value()).unwrap();
    assert_eq!(fresh.status, JobStatus::Running);
}

#[tokio::test]
async fn orphan_out_of_retries_is_failed_not_requeued() {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    storage.create_database("testdb".to_string()).unwrap();
    let db = storage.get_database("testdb").unwrap();
    db.create_collection("_jobs".to_string(), None).unwrap();
    let jobs = db.get_collection("_jobs").unwrap();

    let mut j = job(
        "doomed",
        JobStatus::Running,
        Some(now_ms() - 3_600_000),
        None,
    );
    j.retry_count = 2; // max_retries is 3
    jobs.insert(serde_json::to_value(&j).unwrap()).unwrap();

    let worker = QueueWorker::new(storage.clone(), Arc::new(ScriptStats::default()));
    worker.sweep_jobs_with(0, 600).await;

    let after: Job = serde_json::from_value(jobs.get("doomed").unwrap().to_value()).unwrap();
    assert_eq!(after.status, JobStatus::Failed);
    assert!(after.completed_at.is_some());
}
