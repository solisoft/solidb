use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

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
    pub run_at: u64,     // Unix timestamp (seconds)
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
