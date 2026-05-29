use axum::{
    extract::{Path, State},
    response::Json,
};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

use super::handlers::AppState;
use crate::error::DbError;
use crate::queue::{Job, JobStatus};
use std::str::FromStr;

/// Parse a `run_at` value that may arrive as either:
///  - a Unix-seconds integer (legacy, from the original REST contract), or
///  - an ISO-8601/RFC-3339 string (what the Soli-lang client sends).
fn parse_run_at(val: &JsonValue) -> Result<u64, DbError> {
    if let Some(n) = val.as_u64() {
        return Ok(n);
    }
    if let Some(s) = val.as_str() {
        let parsed = chrono::DateTime::parse_from_rfc3339(s)
            .map_err(|e| DbError::BadRequest(format!("Invalid run_at timestamp '{}': {}", s, e)))?;
        let ts = parsed.timestamp();
        if ts < 0 {
            return Err(DbError::BadRequest(
                "run_at must not be before the epoch".to_string(),
            ));
        }
        return Ok(ts as u64);
    }
    Err(DbError::BadRequest(
        "run_at must be a Unix-seconds integer or an RFC-3339 string".to_string(),
    ))
}

#[derive(Debug, Serialize)]
pub struct QueueStats {
    pub name: String,
    pub pending: usize,
    pub running: usize,
    pub completed: usize,
    pub failed: usize,
    pub total: usize,
}

#[derive(Debug, Serialize)]
pub struct ListJobsResponse {
    pub jobs: Vec<Job>,
    pub total: usize,
}

use axum::extract::Query;

#[derive(Deserialize)]
pub struct ListJobsQuery {
    pub status: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub async fn list_queues_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
) -> Result<Json<Vec<QueueStats>>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    if db.get_collection("_jobs").is_err() {
        return Ok(Json(Vec::new()));
    }

    let jobs_coll = db.get_collection("_jobs")?;
    let mut stats_map: HashMap<String, QueueStats> = HashMap::new();

    for doc in jobs_coll.scan(None) {
        let job: Job = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted job data".to_string()))?;

        let entry = stats_map.entry(job.queue.clone()).or_insert(QueueStats {
            name: job.queue.clone(),
            pending: 0,
            running: 0,
            completed: 0,
            failed: 0,
            total: 0,
        });

        entry.total += 1;
        match job.status {
            JobStatus::Pending => entry.pending += 1,
            JobStatus::Running => entry.running += 1,
            JobStatus::Completed => entry.completed += 1,
            JobStatus::Failed => entry.failed += 1,
        }
    }

    let mut stats: Vec<QueueStats> = stats_map.into_values().collect();
    stats.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(stats))
}

pub async fn list_jobs_handler(
    State(state): State<AppState>,
    Path((db_name, queue_name)): Path<(String, String)>,
    Query(query): Query<ListJobsQuery>,
) -> Result<Json<ListJobsResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let jobs_coll = db.get_collection("_jobs")?;

    let mut jobs = Vec::new();
    let filter_status = query.status.as_deref().map(|s| s.to_lowercase());

    for doc in jobs_coll.scan(None) {
        let job: Job = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted job data".to_string()))?;

        if job.queue == queue_name {
            if let Some(ref status_str) = filter_status {
                let job_status_str = serde_json::to_string(&job.status)
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string();

                if job_status_str.to_lowercase() != *status_str {
                    continue;
                }
            }
            jobs.push(job);
        }
    }

    // Sort by created_at desc
    jobs.sort_by_key(|j| std::cmp::Reverse(j.created_at));

    let total = jobs.len();
    let limit = query.limit.unwrap_or(50);
    let offset = query.offset.unwrap_or(0);

    let jobs = jobs.into_iter().skip(offset).take(limit).collect();

    Ok(Json(ListJobsResponse { jobs, total }))
}

pub async fn cancel_job_handler(
    State(state): State<AppState>,
    Path((db_name, job_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let jobs_coll = db.get_collection("_jobs")?;

    // We only allow deleting jobs that are not running
    let doc = jobs_coll.get(&job_id)?;
    let job: Job = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted job data".to_string()))?;

    if job.status == JobStatus::Running {
        return Err(DbError::BadRequest(
            "Cannot cancel a running job".to_string(),
        ));
    }

    jobs_coll.delete(&job_id)?;

    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn run_now_job_handler(
    State(state): State<AppState>,
    Path((db_name, job_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let jobs_coll = db.get_collection("_jobs")?;

    let doc = jobs_coll.get(&job_id)?;
    let mut job: Job = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted job data".to_string()))?;

    tracing::info!(
        "Run now request for job {} in db {}, current status: {:?}",
        job_id,
        db_name,
        job.status
    );

    if job.status == JobStatus::Running {
        return Err(DbError::BadRequest("Cannot run a running job".to_string()));
    }

    if job.status == JobStatus::Completed {
        return Err(DbError::BadRequest(
            "Cannot run a completed job".to_string(),
        ));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    job.status = JobStatus::Pending;
    job.run_at = now;
    job.last_error = None;

    let rev = job.revision.clone().unwrap_or_default();
    let doc_val = serde_json::to_value(&job).unwrap();
    jobs_coll.update_with_rev(&job_id, &rev, doc_val)?;

    tracing::info!("Job {} updated to pending, run_at set to {}", job_id, now);

    if let Some(ref worker) = state.queue_worker {
        tracing::info!("Sending notification to queue worker");
        let _ = worker.notifier().send(());
    }

    Ok(Json(serde_json::json!({ "success": true })))
}

#[derive(Debug, Deserialize)]
pub struct EnqueueRequest {
    /// Lua script path inside the DB's `_scripts` collection. One of `script`
    /// or `webhook_url` must be set, never both.
    #[serde(default)]
    pub script: Option<String>,
    /// Outbound HTTP webhook URL. When set, the worker POSTs `params` to this
    /// URL with `X-Webhook-Signature` (HMAC-SHA256) and standard event headers.
    #[serde(default, alias = "callback_url")]
    pub webhook_url: Option<String>,
    /// Per-job webhook secret; overrides the `SOLI_WEBHOOK_SECRET` env var.
    #[serde(default)]
    pub webhook_secret: Option<String>,
    /// Extra HTTP headers to attach to the outbound webhook request.
    #[serde(default)]
    pub webhook_headers: Option<HashMap<String, String>>,
    /// Accepted as an alias for `params` for compatibility with the Soli-lang
    /// client, which historically sent `args`.
    #[serde(default, alias = "args")]
    pub params: Option<JsonValue>,
    pub priority: Option<i32>,
    pub max_retries: Option<u32>,
    /// Unix-seconds integer or RFC-3339 string.
    pub run_at: Option<JsonValue>,
}

// CRON JOB HANDLERS

#[derive(Debug, Deserialize)]
pub struct CreateCronJobRequest {
    pub name: String,
    pub cron_expression: String,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default, alias = "callback_url")]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub webhook_secret: Option<String>,
    #[serde(default)]
    pub webhook_headers: Option<HashMap<String, String>>,
    #[serde(default, alias = "args")]
    pub params: Option<JsonValue>,
    pub priority: Option<i32>,
    pub queue: Option<String>,
    pub max_retries: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCronJobRequest {
    pub name: Option<String>,
    pub cron_expression: Option<String>,
    #[serde(default)]
    pub script: Option<String>,
    #[serde(default, alias = "callback_url")]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub webhook_secret: Option<String>,
    #[serde(default)]
    pub webhook_headers: Option<HashMap<String, String>>,
    #[serde(default, alias = "args")]
    pub params: Option<JsonValue>,
    pub priority: Option<i32>,
    pub queue: Option<String>,
    pub max_retries: Option<i32>,
}

pub async fn list_cron_jobs_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
) -> Result<Json<Vec<crate::queue::CronJob>>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    if db.get_collection("_cron_jobs").is_err() {
        return Ok(Json(Vec::new()));
    }
    let cron_coll = db.get_collection("_cron_jobs")?;

    let mut jobs = Vec::new();
    for doc in cron_coll.scan(None) {
        let job: crate::queue::CronJob = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted cron job data".to_string()))?;
        jobs.push(job);
    }
    Ok(Json(jobs))
}

pub async fn create_cron_job_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<CreateCronJobRequest>,
) -> Result<Json<crate::queue::CronJob>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    if db.get_collection("_cron_jobs").is_err() {
        db.create_collection("_cron_jobs".to_string(), None)?;
    }
    let cron_coll = db.get_collection("_cron_jobs")?;

    // Validate cron expression
    if cron::Schedule::from_str(&req.cron_expression).is_err() {
        return Err(DbError::BadRequest("Invalid cron expression".to_string()));
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let script_path = req.script.unwrap_or_default();
    crate::queue::validate_job_target(&script_path, req.webhook_url.as_deref())?;

    let cron_job = crate::queue::CronJob {
        id: uuid::Uuid::new_v4().to_string(),
        revision: None,
        name: req.name,
        cron_expression: req.cron_expression,
        queue: req.queue.unwrap_or_else(|| "default".to_string()),
        priority: req.priority.unwrap_or(0),
        max_retries: req.max_retries.unwrap_or(3),
        script_path,
        webhook_url: req.webhook_url,
        webhook_secret: req.webhook_secret,
        webhook_headers: req.webhook_headers,
        params: req.params.unwrap_or(JsonValue::Null),
        last_run: None,
        next_run: None, // Will be calculated by worker
        created_at: now,
    };

    let doc_val = serde_json::to_value(&cron_job).unwrap();
    cron_coll.insert(doc_val)?;

    Ok(Json(cron_job))
}

pub async fn update_cron_job_handler(
    State(state): State<AppState>,
    Path((db_name, job_id)): Path<(String, String)>,
    Json(req): Json<UpdateCronJobRequest>,
) -> Result<Json<crate::queue::CronJob>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let cron_coll = db.get_collection("_cron_jobs")?;

    let doc = cron_coll.get(&job_id)?;
    let mut cron_job: crate::queue::CronJob = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted cron job".to_string()))?;

    if let Some(name) = req.name {
        cron_job.name = name;
    }
    if let Some(cron) = req.cron_expression {
        if cron::Schedule::from_str(&cron).is_err() {
            return Err(DbError::BadRequest("Invalid cron expression".to_string()));
        }
        cron_job.cron_expression = cron;
        // Reset next run to trigger recalculation
        cron_job.next_run = None;
    }
    if let Some(script) = req.script {
        // Switching to a script clears any prior webhook target.
        crate::queue::validate_job_target(&script, None)?;
        cron_job.script_path = script;
        cron_job.webhook_url = None;
        cron_job.webhook_secret = None;
        cron_job.webhook_headers = None;
    }
    if let Some(url) = req.webhook_url {
        // Switching to a webhook clears the script target.
        crate::queue::validate_job_target("", Some(&url))?;
        cron_job.script_path = String::new();
        cron_job.webhook_url = Some(url);
        if req.webhook_secret.is_some() {
            cron_job.webhook_secret = req.webhook_secret;
        }
        if req.webhook_headers.is_some() {
            cron_job.webhook_headers = req.webhook_headers;
        }
    }
    if let Some(params) = req.params {
        cron_job.params = params;
    }
    if let Some(p) = req.priority {
        cron_job.priority = p;
    }

    let rev = cron_job.revision.clone().unwrap_or_default();
    let doc_val = serde_json::to_value(&cron_job).unwrap();
    cron_coll.update_with_rev(&job_id, &rev, doc_val)?;

    Ok(Json(cron_job))
}

pub async fn delete_cron_job_handler(
    State(state): State<AppState>,
    Path((db_name, job_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let cron_coll = db.get_collection("_cron_jobs")?;
    cron_coll.delete(&job_id)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

pub async fn enqueue_job_handler(
    State(state): State<AppState>,
    Path((db_name, queue_name)): Path<(String, String)>,
    Json(req): Json<EnqueueRequest>,
) -> Result<Json<serde_json::Value>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    if db.get_collection("_jobs").is_err() {
        db.create_collection("_jobs".to_string(), None)?;
    }

    let jobs_coll = db.get_collection("_jobs")?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let script_path = req.script.unwrap_or_default();
    crate::queue::validate_job_target(&script_path, req.webhook_url.as_deref())?;

    let run_at = match req.run_at {
        Some(v) => parse_run_at(&v)?,
        None => now,
    };

    let job_id = uuid::Uuid::new_v4().to_string();
    let job = Job {
        id: job_id.clone(),
        revision: None,
        queue: queue_name,
        priority: req.priority.unwrap_or(0),
        script_path,
        webhook_url: req.webhook_url,
        webhook_secret: req.webhook_secret,
        webhook_headers: req.webhook_headers,
        params: req.params.unwrap_or(JsonValue::Null),
        status: JobStatus::Pending,
        retry_count: 0,
        max_retries: req.max_retries.unwrap_or(20) as i32,
        last_error: None,
        cron_job_id: None,
        run_at,
        created_at: now,
        started_at: None,
        completed_at: None,
    };

    let doc_val = serde_json::to_value(&job).unwrap();
    jobs_coll.insert(doc_val)?;

    if let Some(ref worker) = state.queue_worker {
        let _ = worker.notifier().send(());
    }

    Ok(Json(serde_json::json!({ "job_id": job_id })))
}
