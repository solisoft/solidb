mod embeddings;
mod jobs;
mod materialized_views;
pub(crate) mod signing;
mod types;

pub use jobs::validate_job_target;
pub use types::{Job, JobStatus};

use crate::scripting::{ScriptEngine, ScriptStats};
use crate::storage::StorageEngine;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;

/// Background worker for database-internal scheduled work.
///
/// SolidB no longer exposes a client-facing job or cron queue — application
/// background jobs and cron live in the Soli framework, which runs them in its
/// own process. What remains here is the work the database itself owns:
///
/// * **trigger dispatch** — a trigger fires by inserting a row into `_jobs`,
///   which this worker claims and executes (a stored Lua script, or a signed
///   outbound webhook). `check_jobs` is that dispatcher.
/// * **embedding generation** for vector indexes (`check_embeddings`).
/// * **materialized-view refresh** (`check_materialized_views`).
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
    notifier: broadcast::Sender<()>,
    pub(crate) claiming_lock: tokio::sync::Mutex<()>,
    /// Per-node next-due times ("db:view" -> unix secs) for scheduled
    /// materialized-view refreshes. In-memory (reset on restart).
    pub(crate) mv_next_due: std::sync::Mutex<std::collections::HashMap<String, u64>>,
    /// Bounds concurrently executing jobs across every database (Audit P9).
    /// Sized by `SOLIDB_QUEUE_MAX_CONCURRENCY`, default four per core.
    pub(crate) job_permits: Arc<tokio::sync::Semaphore>,
    /// (database, job key) of jobs this process is executing right now, so
    /// lease recovery never requeues a job that is merely slow.
    pub(crate) in_flight: Arc<std::sync::Mutex<std::collections::HashSet<(String, String)>>>,
    /// When `_jobs` retention / lease recovery last ran.
    last_job_sweep: std::sync::Mutex<Option<std::time::Instant>>,
    /// Per "(db, collection, index)" unix-seconds before which embedding is
    /// not retried after a failure. It used to be one process-wide deadline,
    /// so one tenant's bad provider config stalled every tenant (Audit P9).
    pub(crate) embed_backoff: std::sync::Mutex<std::collections::HashMap<String, u64>>,
}

/// How often the `_jobs` retention / lease sweep runs.
const JOB_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// `SOLIDB_QUEUE_MAX_CONCURRENCY`, default 4 × available cores.
fn queue_max_concurrency() -> usize {
    std::env::var("SOLIDB_QUEUE_MAX_CONCURRENCY")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
                .saturating_mul(4)
        })
        .min(tokio::sync::Semaphore::MAX_PERMITS)
}

impl QueueWorker {
    pub fn new(storage: Arc<StorageEngine>, stats: Arc<ScriptStats>) -> Self {
        let (notifier, _) = broadcast::channel(100);
        let script_engine = Arc::new(
            ScriptEngine::new(storage.clone(), stats).with_queue_notifier(notifier.clone()),
        );

        // `redirect(none)`: the SSRF guard only ever sees the configured URL,
        // so a public host answering `302 Location: http://127.0.0.1:...` or
        // a cloud metadata address would be followed with no second check.
        // The Lua `fetch` client has always set this; the webhook client did
        // not, and reqwest's default follows up to 10 hops.
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest::Client builds with defaults");

        // Under rustls, danger_accept_invalid_certs(true) installs a verifier that skips
        // hostname verification as well as chain validation, so it alone is equivalent to the
        // native-tls certs+hostnames pair this used to set. danger_accept_invalid_hostnames
        // does not exist outside the native-tls backend — do not re-add it.
        let dev_http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(true)
            .build()
            .expect("reqwest::Client builds with permissive TLS");

        Self {
            storage,
            script_engine,
            http_client,
            dev_http_client,
            notifier,
            claiming_lock: tokio::sync::Mutex::new(()),
            mv_next_due: std::sync::Mutex::new(std::collections::HashMap::new()),
            job_permits: Arc::new(tokio::sync::Semaphore::new(queue_max_concurrency())),
            in_flight: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
            last_job_sweep: std::sync::Mutex::new(None),
            embed_backoff: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    pub fn notifier(&self) -> broadcast::Sender<()> {
        self.notifier.clone()
    }

    /// Run the `_jobs` sweep on the first loop pass (start-up) and then at
    /// most once per [`JOB_SWEEP_INTERVAL`].
    async fn maybe_sweep_jobs(&self) {
        {
            let mut last = match self.last_job_sweep.lock() {
                Ok(l) => l,
                Err(_) => return,
            };
            if last.is_some_and(|t| t.elapsed() < JOB_SWEEP_INTERVAL) {
                return;
            }
            *last = Some(std::time::Instant::now());
        }
        self.sweep_jobs().await;
    }

    /// Run the maintenance loop: claim due jobs, sweep pending embeddings,
    /// refresh scheduled materialized views.
    ///
    /// One loop, deliberately. This used to spawn `QUEUE_WORKERS` (default 4)
    /// identical loops, but nothing was gained by it: claimed jobs execute on
    /// `tokio::spawn`, so their concurrency comes from the runtime, and all
    /// three sweeps below take `claiming_lock` — so every worker past the
    /// first woke on the same tick only to lose the `try_lock` and go back to
    /// sleep. The knob's one real effect was on `check_embeddings`, which had
    /// no such guard and so enumerated every collection in the instance once
    /// per worker, every five seconds.
    pub async fn start(self: Arc<Self>) {
        tracing::info!("Starting QueueWorker");

        let mut rx = self.notifier.subscribe();
        loop {
            tokio::select! {
                _ = rx.recv() => {
                    tracing::debug!("Queue worker woke up by notification");
                }
                _ = tokio::time::sleep(Duration::from_secs(5)) => {
                    tracing::debug!("Queue worker periodic check");
                }
            }

            self.maybe_sweep_jobs().await;
            self.check_jobs().await;
            self.check_embeddings().await;
            self.check_materialized_views().await;
        }
    }
}
