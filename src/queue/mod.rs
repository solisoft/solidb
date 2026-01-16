mod cron;
mod jobs;
mod types;

pub use types::{CronJob, Job, JobStatus};

use crate::scripting::{ScriptEngine, ScriptStats};
use crate::storage::StorageEngine;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

pub struct QueueWorker {
    pub(crate) storage: Arc<StorageEngine>,
    pub(crate) script_engine: Arc<ScriptEngine>,
    worker_count: usize,
    notifier: broadcast::Sender<()>,
    pub(crate) claiming_lock: tokio::sync::Mutex<()>,
}

impl QueueWorker {
    pub fn new(storage: Arc<StorageEngine>, stats: Arc<ScriptStats>) -> Self {
        let (notifier, _) = broadcast::channel(100);
        let script_engine = Arc::new(
            ScriptEngine::new(storage.clone(), stats).with_queue_notifier(notifier.clone()),
        );

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

                    worker.check_jobs().await;
                    worker.check_cron_jobs().await;
                }
            });
            workers.push(handle);
        }
    }
}
