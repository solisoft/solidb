use super::signing;
use super::types::{Job, JobStatus};
use super::QueueWorker;
use crate::scripting::ScriptEngine;
use crate::storage::StorageEngine;
use std::sync::Arc;

fn validate_script_path(script_path: &str) -> Result<(), crate::error::DbError> {
    if script_path.is_empty() {
        return Err(crate::error::DbError::BadRequest(
            "Script path cannot be empty".to_string(),
        ));
    }
    if script_path.len() > 512 {
        return Err(crate::error::DbError::BadRequest(
            "Script path exceeds maximum length of 512 characters".to_string(),
        ));
    }
    let re = regex::Regex::new(r"^[A-Za-z0-9_/\-.]+$").unwrap();
    if !re.is_match(script_path) {
        return Err(crate::error::DbError::BadRequest(
            "Script path contains invalid characters".to_string(),
        ));
    }
    Ok(())
}

/// True when `url`'s host is reserved for development by RFC 6761 / 2606 —
/// `.test`, `.localhost`, `.local`, or the literal `localhost`. Webhooks
/// targeting such hosts skip TLS verification so dev setups using mkcert
/// or a local reverse proxy don't need their root CA in SolidB's trust
/// store. Public traffic still goes through the strict client.
pub(crate) fn host_is_dev_tld(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    let host = match parsed.host_str() {
        Some(h) => h.to_ascii_lowercase(),
        None => return false,
    };
    if host == "localhost" {
        return true;
    }
    host.ends_with(".test") || host.ends_with(".localhost") || host.ends_with(".local")
}

pub fn validate_webhook_url(url: &str) -> Result<(), crate::error::DbError> {
    if url.is_empty() {
        return Err(crate::error::DbError::BadRequest(
            "Webhook URL cannot be empty".to_string(),
        ));
    }
    if url.len() > 2048 {
        return Err(crate::error::DbError::BadRequest(
            "Webhook URL exceeds maximum length of 2048 characters".to_string(),
        ));
    }
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| crate::error::DbError::BadRequest(format!("Invalid webhook URL: {}", e)))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(crate::error::DbError::BadRequest(format!(
                "Webhook URL must use http or https, got '{}'",
                other
            )))
        }
    }
    if parsed.username() != "" || parsed.password().is_some() {
        return Err(crate::error::DbError::BadRequest(
            "Webhook URL must not embed credentials".to_string(),
        ));
    }
    let allow_loopback = std::env::var("SOLIDB_ALLOW_WEBHOOK_LOOPBACK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if let Some(host) = parsed.host_str() {
        if allow_loopback && (host == "127.0.0.1" || host == "localhost" || host == "::1") {
            return Ok(());
        }
    }
    crate::server::ssrf::validate_public_url_host(&parsed).map_err(|e| {
        crate::error::DbError::BadRequest(format!("Webhook URL rejected (SSRF): {}", e))
    })?;
    Ok(())
}

/// At enqueue time, exactly one of `script_path` or `webhook_url` must be set.
pub fn validate_job_target(
    script_path: &str,
    webhook_url: Option<&str>,
) -> Result<(), crate::error::DbError> {
    let has_script = !script_path.is_empty();
    let has_webhook = webhook_url.map(|s| !s.is_empty()).unwrap_or(false);
    match (has_script, has_webhook) {
        (true, true) => Err(crate::error::DbError::BadRequest(
            "Job cannot have both script_path and webhook_url; set exactly one".to_string(),
        )),
        (false, false) => Err(crate::error::DbError::BadRequest(
            "Job must have either script_path or webhook_url".to_string(),
        )),
        (true, false) => validate_script_path(script_path),
        (false, true) => validate_webhook_url(webhook_url.unwrap()),
    }
}

/// How many jobs one worker pass may claim per database. Each pass costs one
/// (indexed) candidate query regardless of batch size, so claiming a batch
/// removes the one-scan-per-job ceiling under backlog; per-queue concurrency
/// caps are still enforced claim-by-claim.
const CLAIM_BATCH: usize = 8;

fn default_webhook_secret() -> Option<String> {
    std::env::var("SOLI_WEBHOOK_SECRET")
        .ok()
        .or_else(|| std::env::var("SOLI_JOBS_SECRET").ok())
}

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

        tracing::debug!("Checking for pending jobs at timestamp {}", now);

        for db_name in databases {
            let db = match self.storage.get_database(&db_name) {
                Ok(db) => db,
                Err(_) => continue,
            };

            let jobs_coll = match db.get_collection("_jobs") {
                Ok(coll) => coll,
                Err(_) => continue,
            };

            Self::ensure_status_index(&jobs_coll);

            // Trigger dispatch claims the highest-priority due jobs. There is
            // no per-queue pause/concurrency gating any more: it was only
            // configurable through the client-facing queue API, which is gone.
            let query_str = format!(
                "FOR j IN _jobs FILTER j.status == 'pending' AND j.run_at <= {} SORT j.priority DESC LIMIT {} RETURN j",
                now, CLAIM_BATCH
            );

            tracing::debug!("Query for db {}: {}", db_name, query_str);

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

            if result.is_empty() {
                continue;
            }

            tracing::info!(
                "Found {} pending job(s) to claim in db {}",
                result.len(),
                db_name
            );

            for job_val in result {
                let mut job: Job = match serde_json::from_value(job_val) {
                    Ok(j) => j,
                    Err(e) => {
                        tracing::error!("Corrupted job data in db {}: {}", db_name, e);
                        continue;
                    }
                };

                // Claim job (CAS on revision — losing a race just skips it)
                let rev = job.revision.clone().unwrap_or_default();
                job.status = JobStatus::Running;
                let now_millis = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64;
                job.started_at = Some(now_millis);
                let doc_val = serde_json::to_value(&job).unwrap();
                if let Err(e) = jobs_coll.update_with_rev(&job.id, &rev, doc_val) {
                    tracing::warn!("Failed to claim job {}: {}", job.id, e);
                    continue;
                }
                tracing::info!("Claimed job {} in db {}", job.id, db_name);

                // Execute
                let worker_storage = self.storage.clone();
                let worker_engine = self.script_engine.clone();
                let worker_http = self.http_client.clone();
                let worker_dev_http = self.dev_http_client.clone();
                let worker_notifier = self.notifier();
                let job_id = job.id.clone();
                let db_name_task = db_name.clone();

                tokio::spawn(async move {
                    let mut job_to_update = job;
                    match Self::execute_job(
                        &worker_storage,
                        &worker_engine,
                        &worker_http,
                        &worker_dev_http,
                        &job_to_update,
                        &db_name_task,
                    )
                    .await
                    {
                        Ok(_) => {
                            let completed_millis = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis()
                                as u64;
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
                            // A permanently unsupported operation (a Lua script
                            // action on a node started with --no-lua) fails the
                            // same way on every attempt, so retrying only burns
                            // the backoff schedule.
                            let permanent =
                                matches!(e, crate::error::DbError::OperationNotSupported(_));
                            job_to_update.retry_count += 1;
                            job_to_update.last_error = Some(e.to_string());

                            if !permanent
                                && job_to_update.retry_count < job_to_update.max_retries as u32
                            {
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

                    // A slot just freed up — wake the workers so a queue that
                    // was sitting at its concurrency cap (or a retry
                    // rescheduled to `now`) gets claimed immediately instead
                    // of after the poll.
                    let _ = worker_notifier.send(());
                });
            }
        }
    }

    /// Ensure `_jobs.status` is indexed. Every claim pass and the per-queue
    /// running-count query filter on `status`; without an index each one is
    /// a full scan of the entire job history. Cheap to call once per pass —
    /// index metadata is served from the in-memory cache.
    fn ensure_status_index(jobs_coll: &crate::storage::Collection) {
        if jobs_coll.get_index_for_field("status").is_none() {
            if let Err(e) = jobs_coll.create_index(
                "idx_jobs_status".to_string(),
                vec!["status".to_string()],
                crate::storage::IndexType::Persistent,
                false,
            ) {
                tracing::warn!("Failed to create _jobs status index: {}", e);
            }
        }
    }

    /// Remaining claimable slots per queue for this pass: paused queues map
    /// to 0, capped queues to `cap - running`. Queues without a config row
    /// are absent (unlimited). An empty map means no queue imposes a
    /// restriction — the common case (no `_queue_config` collection).
    pub(crate) async fn execute_job(
        storage: &Arc<StorageEngine>,
        engine: &Arc<ScriptEngine>,
        http: &reqwest::Client,
        dev_http: &reqwest::Client,
        job: &Job,
        db_name: &str,
    ) -> Result<(), crate::error::DbError> {
        if job.is_webhook() {
            Self::execute_webhook(http, dev_http, job).await
        } else {
            Self::execute_script(storage, engine, job, db_name).await
        }
    }

    async fn execute_script(
        storage: &Arc<StorageEngine>,
        engine: &Arc<ScriptEngine>,
        job: &Job,
        db_name: &str,
    ) -> Result<(), crate::error::DbError> {
        validate_script_path(&job.script_path)?;

        tracing::info!("Executing job {} with script {}", job.id, job.script_path);

        let _db = storage.get_database(db_name)?;

        let query_str = "FOR s IN _scripts FILTER s.path == @script_path RETURN s";
        let query_ast = crate::sdbql::parse(query_str)
            .map_err(|e| crate::error::DbError::BadRequest(e.to_string()))?;

        let mut bind_vars = crate::sdbql::BindVars::new();
        bind_vars.insert(
            "script_path".to_string(),
            serde_json::json!(job.script_path),
        );

        let executor = crate::sdbql::QueryExecutor::with_database_and_bind_vars(
            storage,
            db_name.to_string(),
            bind_vars,
        );
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

    async fn execute_webhook(
        http: &reqwest::Client,
        dev_http: &reqwest::Client,
        job: &Job,
    ) -> Result<(), crate::error::DbError> {
        let url = job.webhook_url.as_deref().unwrap_or("");
        validate_webhook_url(url)?;

        // Use the permissive client only for development-reserved TLDs.
        // Everything else stays on the strict client with full TLS checks.
        let allow_insecure_tls = std::env::var("SOLIDB_ALLOW_INSECURE_WEBHOOK_TLS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let client = if allow_insecure_tls && host_is_dev_tld(url) {
            tracing::debug!("Using permissive TLS client for dev host {}", url);
            dev_http
        } else {
            http
        };

        tracing::info!("Firing webhook for job {} to {}", job.id, url);

        let body_bytes = serde_json::to_vec(&job.params).map_err(|e| {
            crate::error::DbError::InternalError(format!("Webhook payload serialize failed: {}", e))
        })?;

        let mut request = client
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("X-Webhook-Event", "job")
            .header("X-Webhook-Delivery", job.id.as_str());

        let secret = job.webhook_secret.clone().or_else(default_webhook_secret);
        if let Some(secret) = secret.as_deref() {
            let sig = signing::sign(&body_bytes, secret);
            request = request.header("X-Webhook-Signature", sig);
        }

        if let Some(headers) = &job.webhook_headers {
            for (k, v) in headers.iter() {
                request = request.header(k.as_str(), v.as_str());
            }
        }

        let response = request.body(body_bytes).send().await.map_err(|e| {
            crate::error::DbError::InternalError(format!("Webhook transport failed: {}", e))
        })?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let snippet: String = body.chars().take(200).collect();
            return Err(crate::error::DbError::InternalError(format!(
                "Webhook returned status {}: {}",
                status.as_u16(),
                snippet
            )));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queue::{Job, JobStatus};
    use std::collections::HashMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// `SOLIDB_ALLOW_WEBHOOK_LOOPBACK` is process-global while tests run in
    /// parallel threads. Without this lock, the test that clears the flag can
    /// land between another test's `set_var` and its delivery: the delivery
    /// then fails SSRF validation, never connects, and the mock server waits
    /// on `accept()` forever. Every test that touches the flag takes it.
    static WEBHOOK_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Wait for the mock server, but fail the test rather than hang if the
    /// webhook never arrived.
    async fn mock_result(server: tokio::task::JoinHandle<String>) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(10), server)
            .await
            .expect("mock server never received the webhook request")
            .expect("mock server task panicked")
    }

    fn make_job(url: Option<&str>, script: &str) -> Job {
        Job {
            id: "job-1".to_string(),
            revision: None,
            queue: "default".to_string(),
            priority: 0,
            script_path: script.to_string(),
            webhook_url: url.map(|s| s.to_string()),
            webhook_secret: None,
            webhook_headers: None,
            params: serde_json::json!({"hello": "world"}),
            status: JobStatus::Pending,
            retry_count: 0,
            max_retries: 3,
            last_error: None,
            cron_job_id: None,
            run_at: 0,
            created_at: 0,
            started_at: None,
            completed_at: None,
        }
    }

    /// Bind 127.0.0.1:0, accept one connection, read the request bytes until
    /// headers+body received, send back the given response, and return the raw
    /// request as a string for assertions.
    async fn mock_once(
        listener: TcpListener,
        response: &'static str,
    ) -> Result<String, std::io::Error> {
        let (mut sock, _) = listener.accept().await?;
        let mut buf = vec![0u8; 8192];
        let mut total = Vec::new();
        loop {
            let n = sock.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            total.extend_from_slice(&buf[..n]);
            // For Content-Length-style POSTs, we wait until the body is fully read.
            if let Some(idx) = find_double_crlf(&total) {
                let header_str = std::str::from_utf8(&total[..idx]).unwrap_or("");
                let content_length = parse_content_length(header_str).unwrap_or(0);
                if total.len() >= idx + 4 + content_length {
                    break;
                }
            }
        }
        sock.write_all(response.as_bytes()).await?;
        sock.flush().await?;
        Ok(String::from_utf8_lossy(&total).to_string())
    }

    fn find_double_crlf(buf: &[u8]) -> Option<usize> {
        buf.windows(4).position(|w| w == b"\r\n\r\n")
    }

    fn parse_content_length(headers: &str) -> Option<usize> {
        for line in headers.split("\r\n") {
            if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                return rest.trim().parse().ok();
            }
        }
        None
    }

    #[test]
    fn validate_target_rejects_both() {
        let err = validate_job_target("some/script", Some("http://x.test/"))
            .expect_err("both targets should fail");
        assert!(format!("{}", err).contains("cannot have both"));
    }

    #[test]
    fn validate_target_rejects_neither() {
        let err = validate_job_target("", None).expect_err("no target should fail");
        assert!(format!("{}", err).contains("must have either"));
    }

    #[test]
    fn validate_target_accepts_script_only() {
        validate_job_target("hello", None).expect("script-only is valid");
    }

    #[test]
    fn validate_target_accepts_webhook_only() {
        validate_job_target("", Some("https://example.test/hook")).expect("webhook-only is valid");
    }

    #[test]
    fn host_is_dev_tld_recognises_reserved_dev_tlds() {
        // Reserved-by-RFC dev TLDs all match.
        assert!(super::host_is_dev_tld("https://bonfire.solisoft.test/hook"));
        assert!(super::host_is_dev_tld("http://app.localhost/hook"));
        assert!(super::host_is_dev_tld("http://localhost/hook"));
        assert!(super::host_is_dev_tld("https://server.local/hook"));
        // Public hosts stay strict.
        assert!(!super::host_is_dev_tld("https://example.com/hook"));
        assert!(!super::host_is_dev_tld("https://api.example.org/hook"));
        // Garbage URLs default to strict (false).
        assert!(!super::host_is_dev_tld("not-a-url"));
    }

    #[test]
    fn validate_webhook_url_rejects_non_http() {
        let err = validate_webhook_url("file:///etc/passwd").expect_err("file:// must fail");
        assert!(format!("{}", err).contains("http or https"));
    }

    #[tokio::test]
    async fn validate_webhook_url_rejects_private_and_metadata() {
        let _guard = WEBHOOK_ENV_LOCK.lock().await;
        std::env::remove_var("SOLIDB_ALLOW_WEBHOOK_LOOPBACK");
        assert!(validate_webhook_url("http://127.0.0.1/hook").is_err());
        assert!(validate_webhook_url("http://169.254.169.254/latest").is_err());
        assert!(validate_webhook_url("http://10.0.0.5/hook").is_err());
        assert!(validate_webhook_url("http://localhost/hook").is_err());
    }

    #[test]
    fn validate_webhook_url_rejects_credentials_in_url() {
        let err =
            validate_webhook_url("http://user:pw@example.test/hook").expect_err("creds must fail");
        assert!(format!("{}", err).contains("credentials"));
    }

    #[tokio::test]
    async fn execute_webhook_posts_signed_payload() {
        let _guard = WEBHOOK_ENV_LOCK.lock().await;
        std::env::set_var("SOLIDB_ALLOW_WEBHOOK_LOOPBACK", "1");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{}/hook", port);

        let mut job = make_job(Some(&url), "");
        job.webhook_secret = Some("s3cret".to_string());
        let mut headers = HashMap::new();
        headers.insert("X-Custom".to_string(), "yes".to_string());
        job.webhook_headers = Some(headers);

        let server = tokio::spawn(async move {
            mock_once(listener, "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap()
        });

        let http = reqwest::Client::new();
        let dev_http = reqwest::Client::new();
        super::super::QueueWorker::execute_webhook(&http, &dev_http, &job)
            .await
            .expect("webhook should succeed");

        let raw = mock_result(server).await;
        assert!(raw.contains("POST /hook"), "request line, got:\n{}", raw);
        assert!(
            raw.to_ascii_lowercase()
                .contains("content-type: application/json"),
            "missing JSON content-type:\n{}",
            raw
        );
        assert!(
            raw.to_ascii_lowercase().contains("x-webhook-event: job"),
            "missing event header:\n{}",
            raw
        );
        assert!(
            raw.to_ascii_lowercase()
                .contains("x-webhook-delivery: job-1"),
            "missing delivery id:\n{}",
            raw
        );
        assert!(
            raw.to_ascii_lowercase().contains("x-webhook-signature: "),
            "missing signature header:\n{}",
            raw
        );
        assert!(
            raw.to_ascii_lowercase().contains("x-custom: yes"),
            "missing custom header:\n{}",
            raw
        );
        // Body present.
        assert!(raw.contains(r#"{"hello":"world"}"#), "body wrong:\n{}", raw);

        // Pull out the signature header and verify it matches HMAC of the body.
        let sig_line = raw
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("x-webhook-signature:"))
            .unwrap();
        let sig = sig_line.split_once(':').unwrap().1.trim();
        let expected = super::signing::sign(br#"{"hello":"world"}"#, "s3cret");
        assert_eq!(sig, expected, "signature mismatch");
    }

    #[tokio::test]
    async fn execute_webhook_propagates_non_2xx_as_error() {
        let _guard = WEBHOOK_ENV_LOCK.lock().await;
        std::env::set_var("SOLIDB_ALLOW_WEBHOOK_LOOPBACK", "1");
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{}/hook", port);

        let job = make_job(Some(&url), "");
        let server = tokio::spawn(async move {
            mock_once(
                listener,
                "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\n\r\nboom",
            )
            .await
            .unwrap()
        });

        let http = reqwest::Client::new();
        let dev_http = reqwest::Client::new();
        let err = super::super::QueueWorker::execute_webhook(&http, &dev_http, &job)
            .await
            .expect_err("500 should be an error");
        let _ = mock_result(server).await;
        assert!(
            format!("{}", err).contains("500"),
            "expected error to mention 500, got: {}",
            err
        );
    }
}
