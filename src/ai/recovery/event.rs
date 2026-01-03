//! Recovery events
//!
//! Types for tracking recovery actions and audit logging.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Collection name for recovery events
pub const RECOVERY_EVENTS_COLLECTION: &str = "_ai_recovery_events";

/// Type of recovery action taken
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryActionType {
    /// Task was recovered from stalled state
    TaskRecovered,
    /// Task was reassigned to different agent
    TaskReassigned,
    /// Task was cancelled after max retries
    TaskCancelled,
    /// Agent circuit breaker opened
    CircuitOpened,
    /// Agent circuit breaker closed
    CircuitClosed,
    /// Agent marked as unhealthy
    AgentUnhealthy,
    /// Agent recovered (back online)
    AgentRecovered,
    /// Contribution pipeline restarted
    PipelineRestarted,
    /// Contribution marked as stuck
    ContributionStuck,
    /// Manual intervention triggered
    ManualIntervention,
}

impl std::fmt::Display for RecoveryActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecoveryActionType::TaskRecovered => write!(f, "task_recovered"),
            RecoveryActionType::TaskReassigned => write!(f, "task_reassigned"),
            RecoveryActionType::TaskCancelled => write!(f, "task_cancelled"),
            RecoveryActionType::CircuitOpened => write!(f, "circuit_opened"),
            RecoveryActionType::CircuitClosed => write!(f, "circuit_closed"),
            RecoveryActionType::AgentUnhealthy => write!(f, "agent_unhealthy"),
            RecoveryActionType::AgentRecovered => write!(f, "agent_recovered"),
            RecoveryActionType::PipelineRestarted => write!(f, "pipeline_restarted"),
            RecoveryActionType::ContributionStuck => write!(f, "contribution_stuck"),
            RecoveryActionType::ManualIntervention => write!(f, "manual_intervention"),
        }
    }
}

/// Severity level of recovery event
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum RecoverySeverity {
    /// Informational - no action needed
    Info,
    /// Warning - may need attention
    Warning,
    /// Error - automatic recovery attempted
    Error,
    /// Critical - requires attention
    Critical,
}

impl std::fmt::Display for RecoverySeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecoverySeverity::Info => write!(f, "info"),
            RecoverySeverity::Warning => write!(f, "warning"),
            RecoverySeverity::Error => write!(f, "error"),
            RecoverySeverity::Critical => write!(f, "critical"),
        }
    }
}

/// A recovery event record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryEvent {
    /// Unique identifier
    #[serde(rename = "_key")]
    pub id: String,

    /// Type of recovery action
    pub action_type: RecoveryActionType,

    /// Severity level
    pub severity: RecoverySeverity,

    /// Description of what happened
    pub description: String,

    /// Related task ID (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,

    /// Related agent ID (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,

    /// Related contribution ID (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contribution_id: Option<String>,

    /// Whether recovery was successful
    pub success: bool,

    /// Error message if recovery failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Additional context
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub context: serde_json::Value,

    /// When the event occurred
    pub created_at: DateTime<Utc>,
}

impl RecoveryEvent {
    /// Create a new recovery event
    pub fn new(
        action_type: RecoveryActionType,
        severity: RecoverySeverity,
        description: String,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            action_type,
            severity,
            description,
            task_id: None,
            agent_id: None,
            contribution_id: None,
            success: true,
            error: None,
            context: serde_json::Value::Null,
            created_at: Utc::now(),
        }
    }

    /// Set task ID
    pub fn with_task(mut self, task_id: &str) -> Self {
        self.task_id = Some(task_id.to_string());
        self
    }

    /// Set agent ID
    pub fn with_agent(mut self, agent_id: &str) -> Self {
        self.agent_id = Some(agent_id.to_string());
        self
    }

    /// Set contribution ID
    pub fn with_contribution(mut self, contribution_id: &str) -> Self {
        self.contribution_id = Some(contribution_id.to_string());
        self
    }

    /// Mark as failed with error
    pub fn failed(mut self, error: String) -> Self {
        self.success = false;
        self.error = Some(error);
        self
    }

    /// Add context data
    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = context;
        self
    }

    /// Create task recovered event
    pub fn task_recovered(task_id: &str, description: String) -> Self {
        Self::new(
            RecoveryActionType::TaskRecovered,
            RecoverySeverity::Warning,
            description,
        )
        .with_task(task_id)
    }

    /// Create task reassigned event
    pub fn task_reassigned(task_id: &str, from_agent: &str, to_agent: Option<&str>) -> Self {
        let desc = match to_agent {
            Some(to) => format!("Task reassigned from {} to {}", from_agent, to),
            None => format!("Task unassigned from {} (awaiting new agent)", from_agent),
        };
        Self::new(RecoveryActionType::TaskReassigned, RecoverySeverity::Info, desc)
            .with_task(task_id)
            .with_agent(from_agent)
    }

    /// Create circuit opened event
    pub fn circuit_opened(agent_id: &str, failures: u32, failure_rate: f64) -> Self {
        Self::new(
            RecoveryActionType::CircuitOpened,
            RecoverySeverity::Warning,
            format!(
                "Circuit breaker opened: {} consecutive failures, {:.1}% failure rate",
                failures,
                failure_rate * 100.0
            ),
        )
        .with_agent(agent_id)
    }

    /// Create circuit closed event
    pub fn circuit_closed(agent_id: &str) -> Self {
        Self::new(
            RecoveryActionType::CircuitClosed,
            RecoverySeverity::Info,
            "Circuit breaker closed - agent recovered".to_string(),
        )
        .with_agent(agent_id)
    }

    /// Create agent unhealthy event
    pub fn agent_unhealthy(agent_id: &str, reason: &str) -> Self {
        Self::new(
            RecoveryActionType::AgentUnhealthy,
            RecoverySeverity::Error,
            format!("Agent marked unhealthy: {}", reason),
        )
        .with_agent(agent_id)
    }

    /// Create contribution stuck event
    pub fn contribution_stuck(contribution_id: &str, status: &str, duration_mins: u64) -> Self {
        Self::new(
            RecoveryActionType::ContributionStuck,
            RecoverySeverity::Error,
            format!(
                "Contribution stuck in '{}' status for {} minutes",
                status, duration_mins
            ),
        )
        .with_contribution(contribution_id)
    }
}

/// Statistics from a recovery cycle
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecoveryCycleStats {
    /// Tasks recovered from stalled state
    pub tasks_recovered: u32,
    /// Tasks reassigned to new agents
    pub tasks_reassigned: u32,
    /// Tasks cancelled after max retries
    pub tasks_cancelled: u32,
    /// Circuit breakers opened
    pub circuits_opened: u32,
    /// Circuit breakers closed
    pub circuits_closed: u32,
    /// Agents marked unhealthy
    pub agents_unhealthy: u32,
    /// Contributions detected as stuck
    pub contributions_stuck: u32,
    /// Errors during recovery
    pub errors: Vec<String>,
    /// Duration of the cycle in milliseconds
    pub duration_ms: u64,
}

/// Response for listing recovery events
#[derive(Debug, Serialize)]
pub struct ListRecoveryEventsResponse {
    pub events: Vec<RecoveryEvent>,
    pub total: usize,
}

/// Query for filtering recovery events
#[derive(Debug, Clone, Deserialize, Default)]
pub struct RecoveryEventQuery {
    pub action_type: Option<RecoveryActionType>,
    pub severity: Option<RecoverySeverity>,
    pub agent_id: Option<String>,
    pub task_id: Option<String>,
    pub contribution_id: Option<String>,
    pub success: Option<bool>,
    pub limit: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recovery_event_creation() {
        let event = RecoveryEvent::new(
            RecoveryActionType::TaskRecovered,
            RecoverySeverity::Warning,
            "Task recovered from stalled state".to_string(),
        );

        assert_eq!(event.action_type, RecoveryActionType::TaskRecovered);
        assert_eq!(event.severity, RecoverySeverity::Warning);
        assert!(event.success);
    }

    #[test]
    fn test_event_builder() {
        let event = RecoveryEvent::task_recovered("task-123", "Recovered from timeout".to_string())
            .with_agent("agent-456")
            .with_contribution("contrib-789");

        assert_eq!(event.task_id, Some("task-123".to_string()));
        assert_eq!(event.agent_id, Some("agent-456".to_string()));
        assert_eq!(event.contribution_id, Some("contrib-789".to_string()));
    }

    #[test]
    fn test_event_failed() {
        let event = RecoveryEvent::new(
            RecoveryActionType::TaskReassigned,
            RecoverySeverity::Info,
            "Reassignment".to_string(),
        )
        .failed("No available agents".to_string());

        assert!(!event.success);
        assert_eq!(event.error, Some("No available agents".to_string()));
    }

    #[test]
    fn test_action_type_display() {
        assert_eq!(
            RecoveryActionType::TaskRecovered.to_string(),
            "task_recovered"
        );
        assert_eq!(RecoveryActionType::CircuitOpened.to_string(), "circuit_opened");
    }

    #[test]
    fn test_severity_ordering() {
        assert!(RecoverySeverity::Info < RecoverySeverity::Warning);
        assert!(RecoverySeverity::Warning < RecoverySeverity::Error);
        assert!(RecoverySeverity::Error < RecoverySeverity::Critical);
    }
}
