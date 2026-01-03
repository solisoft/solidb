//! AI Task types for the contribution pipeline
//!
//! This module defines task types that are processed by AI agents
//! as contributions move through the pipeline.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Type of AI task to be processed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AITaskType {
    /// Analyze a contribution request
    AnalyzeContribution,
    /// Generate code for a contribution
    GenerateCode,
    /// Validate generated code
    ValidateCode,
    /// Run tests on generated code
    RunTests,
    /// Prepare for human review
    PrepareReview,
    /// Merge approved changes
    MergeChanges,
}

impl std::fmt::Display for AITaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AITaskType::AnalyzeContribution => write!(f, "analyze_contribution"),
            AITaskType::GenerateCode => write!(f, "generate_code"),
            AITaskType::ValidateCode => write!(f, "validate_code"),
            AITaskType::RunTests => write!(f, "run_tests"),
            AITaskType::PrepareReview => write!(f, "prepare_review"),
            AITaskType::MergeChanges => write!(f, "merge_changes"),
        }
    }
}

/// Status of an AI task
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AITaskStatus {
    /// Task is waiting to be picked up
    #[default]
    Pending,
    /// Task is currently being processed
    Running,
    /// Task completed successfully
    Completed,
    /// Task failed
    Failed,
    /// Task was cancelled
    Cancelled,
}

impl std::fmt::Display for AITaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AITaskStatus::Pending => write!(f, "pending"),
            AITaskStatus::Running => write!(f, "running"),
            AITaskStatus::Completed => write!(f, "completed"),
            AITaskStatus::Failed => write!(f, "failed"),
            AITaskStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// An AI task in the processing queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AITask {
    /// Unique identifier (UUID)
    #[serde(rename = "_key")]
    pub id: String,
    /// The contribution this task belongs to
    pub contribution_id: String,
    /// Type of task
    pub task_type: AITaskType,
    /// Current status
    pub status: AITaskStatus,
    /// Priority (higher = more urgent)
    #[serde(default)]
    pub priority: i32,
    /// When the task was created
    pub created_at: DateTime<Utc>,
    /// When the task was started (if running)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    /// When the task completed (if done)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Number of retry attempts
    #[serde(default)]
    pub retry_count: u32,
    /// Maximum retries allowed
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Input data for the task
    #[serde(default)]
    pub input: Option<Value>,
    /// Output/result from the task
    #[serde(default)]
    pub output: Option<Value>,
    /// Agent ID that claimed this task
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

fn default_max_retries() -> u32 {
    3
}

impl AITask {
    /// Create a new AI task
    pub fn new(contribution_id: String, task_type: AITaskType, priority: i32) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            contribution_id,
            task_type,
            status: AITaskStatus::Pending,
            priority,
            created_at: Utc::now(),
            started_at: None,
            completed_at: None,
            retry_count: 0,
            max_retries: default_max_retries(),
            error: None,
            input: None,
            output: None,
            agent_id: None,
        }
    }

    /// Create an initial analysis task for a new contribution
    pub fn analyze(contribution_id: String, priority: i32) -> Self {
        Self::new(contribution_id, AITaskType::AnalyzeContribution, priority)
    }

    /// Mark the task as running
    pub fn start(&mut self, agent_id: String) {
        self.status = AITaskStatus::Running;
        self.started_at = Some(Utc::now());
        self.agent_id = Some(agent_id);
    }

    /// Mark the task as completed with output
    pub fn complete(&mut self, output: Option<Value>) {
        self.status = AITaskStatus::Completed;
        self.completed_at = Some(Utc::now());
        self.output = output;
    }

    /// Mark the task as failed
    pub fn fail(&mut self, error: String) {
        self.retry_count += 1;
        self.error = Some(error);

        if self.retry_count >= self.max_retries {
            self.status = AITaskStatus::Failed;
            self.completed_at = Some(Utc::now());
        } else {
            // Reset for retry
            self.status = AITaskStatus::Pending;
            self.started_at = None;
            self.agent_id = None;
        }
    }

    /// Check if the task can be retried
    pub fn can_retry(&self) -> bool {
        self.retry_count < self.max_retries
    }
}

/// Response for listing AI tasks
#[derive(Debug, Serialize)]
pub struct ListAITasksResponse {
    pub tasks: Vec<AITask>,
    pub total: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ai_task_creation() {
        let task = AITask::analyze("contrib-123".to_string(), 5);

        assert_eq!(task.contribution_id, "contrib-123");
        assert_eq!(task.task_type, AITaskType::AnalyzeContribution);
        assert_eq!(task.status, AITaskStatus::Pending);
        assert_eq!(task.priority, 5);
        assert!(!task.id.is_empty());
    }

    #[test]
    fn test_ai_task_lifecycle() {
        let mut task = AITask::new(
            "contrib-456".to_string(),
            AITaskType::GenerateCode,
            0,
        );

        assert_eq!(task.status, AITaskStatus::Pending);
        assert!(task.started_at.is_none());

        task.start("agent-001".to_string());
        assert_eq!(task.status, AITaskStatus::Running);
        assert!(task.started_at.is_some());
        assert_eq!(task.agent_id, Some("agent-001".to_string()));

        task.complete(Some(serde_json::json!({"files_generated": 3})));
        assert_eq!(task.status, AITaskStatus::Completed);
        assert!(task.completed_at.is_some());
        assert!(task.output.is_some());
    }

    #[test]
    fn test_ai_task_retry() {
        let mut task = AITask::new(
            "contrib-789".to_string(),
            AITaskType::ValidateCode,
            0,
        );
        task.max_retries = 3;

        // First failure - should retry
        task.fail("Network error".to_string());
        assert_eq!(task.status, AITaskStatus::Pending);
        assert_eq!(task.retry_count, 1);
        assert!(task.can_retry());

        // Second failure - should retry
        task.fail("Timeout".to_string());
        assert_eq!(task.status, AITaskStatus::Pending);
        assert_eq!(task.retry_count, 2);
        assert!(task.can_retry());

        // Third failure - should be final
        task.fail("Service unavailable".to_string());
        assert_eq!(task.status, AITaskStatus::Failed);
        assert_eq!(task.retry_count, 3);
        assert!(!task.can_retry());
    }

    #[test]
    fn test_ai_task_type_serialization() {
        let json = serde_json::to_string(&AITaskType::AnalyzeContribution).unwrap();
        assert_eq!(json, "\"analyze_contribution\"");

        let parsed: AITaskType = serde_json::from_str("\"generate_code\"").unwrap();
        assert_eq!(parsed, AITaskType::GenerateCode);
    }
}
