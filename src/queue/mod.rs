use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::sync::Arc;
use std::time::{Duration};
use tokio::sync::broadcast;
use crate::storage::StorageEngine;
use crate::scripting::ScriptEngine;
use std::str::FromStr;
use cron::Schedule;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    #[serde(rename = "_key")]
    pub id: String,
    #[serde(rename = "_rev", skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub queue: String,
    #[serde(default)]
    pub priority: i32,
    pub script_path: String,
    pub params: JsonValue,
    pub status: JobStatus,
    pub retry_count: u32,
    pub max_retries: i32,
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_job_id: Option<String>,
    pub run_at: u64, // Unix timestamp (seconds)
    pub created_at: u64, // Unix timestamp (seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>, // Unix timestamp in MILLISECONDS for duration precision
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>, // Unix timestamp in MILLISECONDS for duration precision
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    #[serde(rename = "_key")]
    pub id: String,
    #[serde(rename = "_rev", skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub name: String,
    pub cron_expression: String,
    pub queue: String,
    pub priority: i32,
    #[serde(default = "default_max_retries")]
    pub max_retries: i32,
    pub script_path: String,
    pub params: JsonValue,
    pub last_run: Option<u64>,
    pub next_run: Option<u64>,
    pub created_at: u64,
}

fn default_max_retries() -> i32 {
    3
}

use crate::scripting::ScriptStats;

pub struct QueueWorker {
    storage: Arc<StorageEngine>,
    script_engine: Arc<ScriptEngine>,
    worker_count: usize,
    notifier: broadcast::Sender<()>,
    claiming_lock: tokio::sync::Mutex<()>,
}

impl QueueWorker {
    pub fn new(storage: Arc<StorageEngine>, stats: Arc<ScriptStats>) -> Self {
        let (notifier, _) = broadcast::channel(100);
        let script_engine = Arc::new(ScriptEngine::new(storage.clone(), stats)
            .with_queue_notifier(notifier.clone()));
        
        let worker_count = std::env::var("QUEUE_WORKERS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);

        Self {
            storage,
            script_engine,
            worker_count,
            notifier,
            claiming_lock: tokio::sync::Mutex::new(()),
        }
    }

    pub fn notifier(&self) -> broadcast::Sender<()> {
        self.notifier.clone()
    }

    pub async fn start(self: Arc<Self>) {
        tracing::info!("Starting QueueWorker with {} workers", self.worker_count);
        
        let mut workers = Vec::new();
        for i in 0..self.worker_count {
            let worker = self.clone();
            let mut rx = self.notifier.subscribe();
            
            let handle = tokio::spawn(async move {
                tracing::info!("Queue Worker {} started", i);
                loop {
                    tokio::select! {
                        _ = rx.recv() => {
                            // Woke up by notification
                        }
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {
                            // Periodic check
                        }
                    }
                    
                    // Call the check_jobs method
                    worker.check_jobs().await;
                    // Call the check_cron_jobs method (only 1 worker needs to do this really, but for now strict concurrency control is handled via atomic writes/comparisons or just let them race if harmless. 
                    // Better: use the same claiming lock!
                    worker.check_cron_jobs().await;
                }
            });
            workers.push(handle);
        }
    }

    pub async fn check_jobs(&self) {
        let _lock = match self.claiming_lock.try_lock() {
            Ok(lock) => lock,
            Err(_) => return, // Already checking
        };

        let databases = self.storage.list_databases();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        for db_name in databases {
            let db = match self.storage.get_database(&db_name) {
                Ok(db) => db,
                Err(_) => continue,
            };

            let jobs_coll = match db.get_collection("_jobs") {
                Ok(coll) => coll,
                Err(_) => continue,
            };

            // Query for candidates in this specific database
            let query_str = format!(
                "FOR j IN _jobs FILTER j.status == 'pending' AND j.run_at <= {} SORT j.priority DESC LIMIT 1 RETURN j",
                now
            );
            
            let query_ast = match crate::sdbql::parse(&query_str) {
                Ok(q) => q,
                Err(e) => {
                    tracing::error!("Failed to parse worker query: {}", e);
                    continue;
                }
            };

            let executor = crate::sdbql::QueryExecutor::with_database(&self.storage, db_name.clone());
            let result = match executor.execute(&query_ast) {
                Ok(res) => res,
                Err(e) => {
                    tracing::error!("Worker error in db {}: {}", db_name, e);
                    continue;
                }
            };

            let job_val = match result.get(0) {
                Some(val) => val,
                None => continue,
            };

            let mut job: Job = match serde_json::from_value(job_val.clone()) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!("Corrupted job data in db {}: {}", db_name, e);
                    continue;
                }
            };

            // Claim job
            let rev = job.revision.clone().unwrap_or_default();
            job.status = JobStatus::Running;
            let now_millis = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            job.started_at = Some(now_millis);
            let doc_val = serde_json::to_value(&job).unwrap();
            if let Err(_e) = jobs_coll.update_with_rev(&job.id, &rev, doc_val) {
                // If this fails (e.g. ConflictError), it means another worker claimed it first
                continue;
            }

            // Execute
            let worker_storage = self.storage.clone();
            let worker_engine = self.script_engine.clone();
            let job_id = job.id.clone();
            let db_name_task = db_name.clone();

            tokio::spawn(async move {
                let mut job_to_update = job;
                match Self::execute_job(&worker_storage, &worker_engine, &job_to_update, &db_name_task).await {
                    Ok(_) => {
                        let completed_millis = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64;
                        job_to_update.status = JobStatus::Completed;
                        job_to_update.completed_at = Some(completed_millis);
                        job_to_update.last_error = None;
                    }
                    Err(e) => {
                        let completed_now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_secs();
                        let completed_millis = completed_now * 1000 + (std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .subsec_millis() as u64);
                        tracing::error!("Job {} failed in db {}: {}", job_id, db_name_task, e);
                        job_to_update.retry_count += 1;
                        job_to_update.last_error = Some(e.to_string());
                        
                        if job_to_update.retry_count < job_to_update.max_retries as u32 {
                            job_to_update.status = JobStatus::Pending;
                            job_to_update.started_at = None; // Reset for retry
                            // Exponential backoff: 10 * 2^retry_count seconds
                            let delay = 10 * (2u64.pow(job_to_update.retry_count));
                            // Cap at 24 hours
                            let delay = std::cmp::min(delay, 24 * 3600);
                            job_to_update.run_at = completed_now + delay;
                        } else {
                            job_to_update.status = JobStatus::Failed;
                            job_to_update.completed_at = Some(completed_millis);
                        }
                    }
                }

                // Update back to DB
                if let Ok(db) = worker_storage.get_database(&db_name_task) {
                    if let Ok(coll) = db.get_collection("_jobs") {
                        let final_val = serde_json::to_value(&job_to_update).unwrap();
                        let _ = coll.update(&job_id, final_val);
                    }
                }
            });

            // We found a job in this DB, but we only process one candidate per check_jobs call?
            // Actually we are in a loop over databases. We could process one per DB per loop.
            // Given we have a worker pool, this is fine.
        }
    }

    pub async fn check_cron_jobs(&self) {
        let _lock = match self.claiming_lock.try_lock() {
            Ok(lock) => lock,
            Err(_) => return, // Already checking
        };

        let databases = self.storage.list_databases();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        for db_name in databases {
            let db = match self.storage.get_database(&db_name) {
                Ok(db) => db,
                Err(_) => continue,
            };

            // Ensure _cron_jobs collection exists
            if db.get_collection("_cron_jobs").is_err() {
                 let _ = db.create_collection("_cron_jobs".to_string(), None);
            }

            let cron_coll = match db.get_collection("_cron_jobs") {
                Ok(coll) => coll,
                Err(_) => continue,
            };

            // Scan all cron jobs
            for doc in cron_coll.scan(None) {
                 let mut cron_job: CronJob = match serde_json::from_value(doc.to_value()) {
                    Ok(j) => j,
                    Err(_) => continue, 
                 };

                 let mut should_run = false;
                 // First run calculation if needed
                 if cron_job.next_run.is_none() {
                     if let Ok(schedule) = Schedule::from_str(&cron_job.cron_expression) {
                         // Find the next occurrence after now
                         if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                             cron_job.next_run = Some(next.timestamp() as u64);
                             // Save the initial next_run calculation immediately so it's visible in UI
                             let rev = cron_job.revision.clone().unwrap_or_default();
                             if let Ok(updated_val) = serde_json::to_value(&cron_job) {
                                 let _ = cron_coll.update_with_rev(&cron_job.id, &rev, updated_val);
                             }
                             should_run = false; 
                         }
                     }
                 } 
                 
                 if let Some(next_run) = cron_job.next_run {
                     if next_run <= now {
                         should_run = true;
                     }
                 }

                 if should_run {
                     // CLUSTER-SAFE: Claim the cron job FIRST via optimistic locking
                     // Only the node that wins the revision check will spawn the job
                     
                     // 1. Calculate next run
                     let mut new_next_run = None;
                     if let Ok(schedule) = Schedule::from_str(&cron_job.cron_expression) {
                         if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                             new_next_run = Some(next.timestamp() as u64);
                         }
                     }

                     // 2. Prepare updated cron_job state for the claim
                     let mut claimed_cron = cron_job.clone();
                     claimed_cron.last_run = Some(now);
                     claimed_cron.next_run = new_next_run;

                     // 3. Attempt atomic claim via revision check
                     let rev = cron_job.revision.clone().unwrap_or_default();
                     let claim_result = if let Ok(updated_val) = serde_json::to_value(&claimed_cron) {
                         cron_coll.update_with_rev(&cron_job.id, &rev, updated_val)
                     } else {
                         Err(crate::error::DbError::ExecutionError("Failed to serialize cron job".to_string()))
                     };

                     // 4. Only spawn if we won the claim (no revision conflict)
                     if claim_result.is_ok() {
                         tracing::info!(
                             "Cron job '{}' claimed successfully, spawning job for script '{}'",
                             cron_job.name,
                             cron_job.script_path
                         );
                         
                         let new_job = Job {
                            id: uuid::Uuid::new_v4().to_string(),
                            revision: None,
                            queue: cron_job.queue.clone(),
                            priority: cron_job.priority,
                            script_path: cron_job.script_path.clone(),
                            params: cron_job.params.clone(),
                            status: JobStatus::Pending,
                            retry_count: 0,
                            max_retries: cron_job.max_retries, 
                            last_error: None,
                            cron_job_id: Some(cron_job.id.clone()),
                            run_at: now,
                            created_at: now,
                            started_at: None,
                            completed_at: None,
                         };
                         
                         // Ensure _jobs collection exists before inserting
                         if db.get_collection("_jobs").is_err() {
                             let _ = db.create_collection("_jobs".to_string(), None);
                         }
                         
                         if let Ok(jobs_coll) = db.get_collection("_jobs") {
                             if let Ok(val) = serde_json::to_value(&new_job) {
                                 if let Err(e) = jobs_coll.insert(val) {
                                     tracing::error!("Failed to insert job for cron '{}': {:?}", cron_job.name, e);
                                 } else {
                                     tracing::info!("Cron job '{}' spawned job {} in queue '{}'", cron_job.name, new_job.id, new_job.queue);
                                 }
                             }
                         }
                     } else {
                         // Another node already claimed this cron job - skip
                         tracing::debug!(
                             "Cron job '{}' already claimed by another node, skipping",
                             cron_job.name
                         );
                     }
                 }
             }
         }
    }


    async fn execute_job(
        storage: &Arc<StorageEngine>,
        engine: &Arc<ScriptEngine>,
        job: &Job,
        db_name: &str,
    ) -> Result<(), crate::error::DbError> {
        let _db = storage.get_database(db_name)?;
        
        // Find script by path
        let query_str = format!("FOR s IN _scripts FILTER s.path == '{}' RETURN s", job.script_path);
        let query_ast = crate::sdbql::parse(&query_str)
            .map_err(|e| crate::error::DbError::BadRequest(e.to_string()))?;
            
        let executor = crate::sdbql::QueryExecutor::with_database(storage, db_name.to_string());
        let result = executor.execute(&query_ast)?;
        
        let script_val = result.get(0).ok_or_else(|| crate::error::DbError::DocumentNotFound(format!("Script not found: {}", job.script_path)))?;
        let script: crate::scripting::Script = serde_json::from_value(script_val.clone())
            .map_err(|_| crate::error::DbError::InternalError("Corrupted script data".to_string()))?;

        let context = crate::scripting::ScriptContext {
            method: "POST".to_string(), // Background jobs are always POST-like
            path: job.id.clone(),
            query_params: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            body: Some(job.params.clone()),
            params: std::collections::HashMap::new(),
            is_websocket: false,
            // Queue jobs run as system/admin user
            user: crate::scripting::ScriptUser {
                username: "_system".to_string(),
                roles: vec!["admin".to_string()],
                authenticated: true,
                scoped_databases: None,
                exp: None,
            },
        };

        let res = engine.execute(&script, db_name, &context).await?;
        
        if res.status >= 400 {
            return Err(crate::error::DbError::InternalError(format!("Script returned error status: {}", res.status)));
        }

        Ok(())
    }
}
