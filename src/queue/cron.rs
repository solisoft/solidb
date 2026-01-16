use super::types::{CronJob, Job, JobStatus};
use super::QueueWorker;
use cron::Schedule;
use std::str::FromStr;

impl QueueWorker {
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
                        if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                            cron_job.next_run = Some(next.timestamp() as u64);
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
                    // Calculate next run
                    let mut new_next_run = None;
                    if let Ok(schedule) = Schedule::from_str(&cron_job.cron_expression) {
                        if let Some(next) = schedule.upcoming(chrono::Utc).next() {
                            new_next_run = Some(next.timestamp() as u64);
                        }
                    }

                    // Prepare updated cron_job state for the claim
                    let mut claimed_cron = cron_job.clone();
                    claimed_cron.last_run = Some(now);
                    claimed_cron.next_run = new_next_run;

                    // Attempt atomic claim via revision check
                    let rev = cron_job.revision.clone().unwrap_or_default();
                    let claim_result = if let Ok(updated_val) = serde_json::to_value(&claimed_cron)
                    {
                        cron_coll.update_with_rev(&cron_job.id, &rev, updated_val)
                    } else {
                        Err(crate::error::DbError::ExecutionError(
                            "Failed to serialize cron job".to_string(),
                        ))
                    };

                    // Only spawn if we won the claim
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

                        // Ensure _jobs collection exists
                        if db.get_collection("_jobs").is_err() {
                            let _ = db.create_collection("_jobs".to_string(), None);
                        }

                        if let Ok(jobs_coll) = db.get_collection("_jobs") {
                            if let Ok(val) = serde_json::to_value(&new_job) {
                                if let Err(e) = jobs_coll.insert(val) {
                                    tracing::error!(
                                        "Failed to insert job for cron '{}': {:?}",
                                        cron_job.name,
                                        e
                                    );
                                } else {
                                    tracing::info!(
                                        "Cron job '{}' spawned job {} in queue '{}'",
                                        cron_job.name,
                                        new_job.id,
                                        new_job.queue
                                    );
                                }
                            }
                        }
                    } else {
                        tracing::debug!(
                            "Cron job '{}' already claimed by another node, skipping",
                            cron_job.name
                        );
                    }
                }
            }
        }
    }
}
