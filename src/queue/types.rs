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
