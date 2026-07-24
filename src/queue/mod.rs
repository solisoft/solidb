mod cron;
mod embeddings;
mod jobs;
mod materialized_views;
pub(crate) mod signing;
mod types;

pub use jobs::validate_job_target;
pub use types::{CronJob, Job, JobStatus, QueueConfig};

use crate::scripting::{ScriptEngine, ScriptStats};
use crate::storage::StorageEngine;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

pub struct QueueWorker {
    pub(crate) storage: Arc<StorageEngine>,
    pub(crate) script_engine: Arc<ScriptEngine>,
    /// Strict client — full TLS verification. Used for any webhook target
    /// on a real public host.
    pub(crate) http_client: reqwest::Client,
    /// Permissive client — accepts invalid/self-signed TLS certs. Used
    /// **only** for webhook targets whose host falls under a reserved
    /// development TLD (`.test`, `.localhost`, `.local`, or the literal
    /// `localhost`). Lets dev setups behind mkcert / Caddy / a local
    /// reverse proxy succeed without putting the root CA in SolidB's
    /// trust store.
    pub(crate) dev_http_client: reqwest::Client,
    worker_count: usize,
    notifier: broadcast::Sender<()>,
    pub(crate) claiming_lock: tokio::sync::Mutex<()>,
    /// Per-node next-due times ("db:view" -> unix secs) for scheduled
    /// materialized-view refreshes. In-memory (reset on restart).
    pub(crate) mv_next_due: std::sync::Mutex<std::collections::HashMap<String, u64>>,
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

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest::Client builds with defaults");

        // Under rustls, danger_accept_invalid_certs(true) installs a verifier that skips
        // hostname verification as well as chain validation, so it alone is equivalent to the
        // native-tls certs+hostnames pair this used to set. danger_accept_invalid_hostnames
        // does not exist outside the native-tls backend — do not re-add it.
        let dev_http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .danger_accept_invalid_certs(true)
            .build()
            .expect("reqwest::Client builds with permissive TLS");

        Self {
            storage,
            script_engine,
            http_client,
            dev_http_client,
            worker_count,
            notifier,
            claiming_lock: tokio::sync::Mutex::new(()),
            mv_next_due: std::sync::Mutex::new(std::collections::HashMap::new()),
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
                            tracing::debug!("Queue Worker {} woke up by notification", i);
                        }
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {
                            tracing::debug!("Queue Worker {} periodic check", i);
                        }
                    }

                    worker.check_jobs().await;
                    worker.check_cron_jobs().await;
                    worker.check_embeddings().await;
                    worker.check_materialized_views().await;
                }
            });
            workers.push(handle);
        }
    }
}
