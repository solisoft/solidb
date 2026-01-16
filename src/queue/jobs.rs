use super::types::{Job, JobStatus};
use super::QueueWorker;
use crate::scripting::ScriptEngine;
use crate::storage::StorageEngine;
use std::sync::Arc;

impl QueueWorker {
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

            let executor =
                crate::sdbql::QueryExecutor::with_database(&self.storage, db_name.clone());
            let result = match executor.execute(&query_ast) {
                Ok(res) => res,
                Err(e) => {
                    tracing::error!("Worker error in db {}: {}", db_name, e);
                    continue;
                }
            };

            let job_val = match result.first() {
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
                continue;
            }

            // Execute
            let worker_storage = self.storage.clone();
            let worker_engine = self.script_engine.clone();
            let job_id = job.id.clone();
            let db_name_task = db_name.clone();

            tokio::spawn(async move {
                let mut job_to_update = job;
                match Self::execute_job(
                    &worker_storage,
                    &worker_engine,
                    &job_to_update,
                    &db_name_task,
                )
                .await
                {
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
                        let completed_millis = completed_now * 1000
                            + (std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .subsec_millis() as u64);
                        tracing::error!("Job {} failed in db {}: {}", job_id, db_name_task, e);
                        job_to_update.retry_count += 1;
                        job_to_update.last_error = Some(e.to_string());

                        if job_to_update.retry_count < job_to_update.max_retries as u32 {
                            job_to_update.status = JobStatus::Pending;
                            job_to_update.started_at = None;
                            let delay = 10 * (2u64.pow(job_to_update.retry_count));
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
        }
    }

    pub(crate) async fn execute_job(
        storage: &Arc<StorageEngine>,
        engine: &Arc<ScriptEngine>,
        job: &Job,
        db_name: &str,
    ) -> Result<(), crate::error::DbError> {
        let _db = storage.get_database(db_name)?;

        // Find script by path
        let query_str = format!(
            "FOR s IN _scripts FILTER s.path == '{}' RETURN s",
            job.script_path
        );
        let query_ast = crate::sdbql::parse(&query_str)
            .map_err(|e| crate::error::DbError::BadRequest(e.to_string()))?;

        let executor = crate::sdbql::QueryExecutor::with_database(storage, db_name.to_string());
        let result = executor.execute(&query_ast)?;

        let script_val = result.first().ok_or_else(|| {
            crate::error::DbError::DocumentNotFound(format!(
                "Script not found: {}",
                job.script_path
            ))
        })?;
        let script: crate::scripting::Script =
            serde_json::from_value(script_val.clone()).map_err(|_| {
                crate::error::DbError::InternalError("Corrupted script data".to_string())
            })?;

        let context = crate::scripting::ScriptContext {
            method: "POST".to_string(),
            path: job.id.clone(),
            query_params: std::collections::HashMap::new(),
            headers: std::collections::HashMap::new(),
            body: Some(job.params.clone()),
            params: std::collections::HashMap::new(),
            is_websocket: false,
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
            return Err(crate::error::DbError::InternalError(format!(
                "Script returned error status: {}",
                res.status
            )));
        }

        Ok(())
    }
}
