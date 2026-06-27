use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::HashMap;

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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub script_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_headers: Option<HashMap<String, String>>,
    pub params: JsonValue,
    pub status: JobStatus,
    pub retry_count: u32,
    pub max_retries: i32,
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_job_id: Option<String>,
    pub run_at: u64,     // Unix timestamp (seconds)
    pub created_at: u64, // Unix timestamp (seconds)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>, // Unix timestamp in MILLISECONDS for duration precision
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>, // Unix timestamp in MILLISECONDS for duration precision
}

impl Job {
    /// True iff this job dispatches via webhook (URL POST) rather than a Lua script.
    pub fn is_webhook(&self) -> bool {
        self.webhook_url
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub script_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_headers: Option<HashMap<String, String>>,
    pub params: JsonValue,
    pub last_run: Option<u64>,
    pub next_run: Option<u64>,
    pub created_at: u64,
}

fn default_max_retries() -> i32 {
    3
}

/// Per-queue settings, persisted in the `_queue_config` collection keyed by
/// queue name. A queue with no config row behaves as the defaults below
/// (running, unlimited concurrency, priority 0), so configuration is purely
/// additive — existing queues keep working untouched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueConfig {
    /// Queue name; stored as the document `_key`.
    #[serde(rename = "_key")]
    pub name: String,
    #[serde(rename = "_rev", skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// When true the worker claims no jobs from this queue (already-running
    /// jobs finish; new ones wait until the queue is resumed).
    #[serde(default)]
    pub paused: bool,
    /// Max jobs from this queue allowed to run concurrently. `0` means
    /// unlimited (the historical behavior).
    #[serde(default)]
    pub concurrency: u32,
    /// Priority assigned to jobs enqueued on this queue when the enqueue
    /// request omits an explicit priority.
    #[serde(default)]
    pub default_priority: i32,
    pub updated_at: u64,
}
