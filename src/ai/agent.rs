//! AI Agent infrastructure for the contribution pipeline
//!
//! This module provides the foundation for AI agents that process
//! contributions through the pipeline stages.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Type of AI agent
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AgentType {
    /// Analyzes contribution requests and determines scope
    Analyzer,
    /// Generates code based on specifications
    Coder,
    /// Creates and runs tests for generated code
    Tester,
    /// Reviews code for quality and best practices
    Reviewer,
    /// Integrates approved changes
    Integrator,
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentType::Analyzer => write!(f, "analyzer"),
            AgentType::Coder => write!(f, "coder"),
            AgentType::Tester => write!(f, "tester"),
            AgentType::Reviewer => write!(f, "reviewer"),
            AgentType::Integrator => write!(f, "integrator"),
        }
    }
}

/// Status of an AI agent
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    /// Agent is available to accept tasks
    #[default]
    Idle,
    /// Agent is currently processing a task
    Busy,
    /// Agent is temporarily unavailable
    Offline,
    /// Agent encountered an error
    Error,
}

/// Represents a registered AI agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Unique identifier (UUID)
    #[serde(rename = "_key")]
    pub id: String,
    /// Human-readable name
    #[serde(default = "default_name")]
    pub name: String,
    /// Type of agent
    #[serde(default = "default_agent_type")]
    pub agent_type: AgentType,
    /// Current status
    #[serde(default)]
    pub status: AgentStatus,
    /// Webhook URL for task notifications (optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Capabilities this agent provides
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Configuration for the agent
    #[serde(default)]
    pub config: Option<Value>,
    /// When the agent was registered
    #[serde(default = "Utc::now")]
    pub registered_at: DateTime<Utc>,
    /// Last heartbeat timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_heartbeat: Option<DateTime<Utc>>,
    /// Current task being processed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<String>,
    /// Total tasks processed
    #[serde(default)]
    pub tasks_completed: u64,
    /// Total tasks failed
    #[serde(default)]
    pub tasks_failed: u64,
}

fn default_name() -> String {
    "Unnamed Agent".to_string()
}

fn default_agent_type() -> AgentType {
    AgentType::Analyzer
}

impl Agent {
    /// Create a new agent
    pub fn new(name: String, agent_type: AgentType, capabilities: Vec<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            agent_type,
            status: AgentStatus::Idle,
            url: None,
            capabilities,
            config: None,
            registered_at: Utc::now(),
            last_heartbeat: Some(Utc::now()),
            current_task_id: None,
            tasks_completed: 0,
            tasks_failed: 0,
        }
    }

    /// Create a new agent with URL
    pub fn new_with_url(
        name: String,
        agent_type: AgentType,
        capabilities: Vec<String>,
        url: Option<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            agent_type,
            status: AgentStatus::Idle,
            url,
            capabilities,
            config: None,
            registered_at: Utc::now(),
            last_heartbeat: Some(Utc::now()),
            current_task_id: None,
            tasks_completed: 0,
            tasks_failed: 0,
        }
    }

    /// Update heartbeat timestamp
    pub fn heartbeat(&mut self) {
        self.last_heartbeat = Some(Utc::now());
    }

    /// Check if agent is healthy (heartbeat within threshold)
    pub fn is_healthy(&self, timeout_seconds: i64) -> bool {
        if let Some(last) = self.last_heartbeat {
            let elapsed = Utc::now().signed_duration_since(last);
            elapsed.num_seconds() < timeout_seconds
        } else {
            false
        }
    }

    /// Mark agent as busy with a task
    pub fn start_task(&mut self, task_id: String) {
        self.status = AgentStatus::Busy;
        self.current_task_id = Some(task_id);
        self.heartbeat();
    }

    /// Mark agent as idle after completing a task
    pub fn complete_task(&mut self, success: bool) {
        self.status = AgentStatus::Idle;
        self.current_task_id = None;
        if success {
            self.tasks_completed += 1;
        } else {
            self.tasks_failed += 1;
        }
        self.heartbeat();
    }
}

/// Analysis result from the Analyzer agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisResult {
    /// Files that would be affected by this contribution
    pub affected_files: Vec<String>,
    /// Risk score (0.0 - 1.0)
    pub risk_score: f64,
    /// Whether this requires human review
    pub requires_review: bool,
    /// Reason for the risk assessment
    pub risk_reason: Option<String>,
    /// Suggested implementation approach
    pub suggested_approach: Option<String>,
    /// Estimated complexity (1-10)
    pub complexity: u8,
    /// Dependencies identified
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Related existing code patterns found
    #[serde(default)]
    pub related_patterns: Vec<String>,
}

impl AnalysisResult {
    /// Create an analysis result indicating the contribution is safe to proceed
    pub fn safe(affected_files: Vec<String>) -> Self {
        Self {
            affected_files,
            risk_score: 0.2,
            requires_review: false,
            risk_reason: None,
            suggested_approach: None,
            complexity: 3,
            dependencies: Vec::new(),
            related_patterns: Vec::new(),
        }
    }

    /// Create an analysis result indicating high risk
    pub fn high_risk(affected_files: Vec<String>, reason: String) -> Self {
        Self {
            affected_files,
            risk_score: 0.8,
            requires_review: true,
            risk_reason: Some(reason),
            suggested_approach: None,
            complexity: 7,
            dependencies: Vec::new(),
            related_patterns: Vec::new(),
        }
    }
}

/// Code generation result from the Coder agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeGenerationResult {
    /// Generated files with their content
    pub files: Vec<GeneratedFile>,
    /// Summary of changes
    pub summary: String,
    /// Test coverage estimate
    pub test_coverage_estimate: Option<f64>,
}

/// A generated file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedFile {
    /// File path relative to project root
    pub path: String,
    /// File content
    pub content: String,
    /// Whether this is a new file or modification
    pub is_new: bool,
    /// Original content (for modifications)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_content: Option<String>,
}

/// Validation result from the validation pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Overall pass/fail status
    pub passed: bool,
    /// Individual stage results
    pub stages: Vec<ValidationStageResult>,
    /// Total errors found
    pub error_count: usize,
    /// Total warnings found
    pub warning_count: usize,
}

impl ValidationResult {
    /// Create a new empty validation result
    pub fn new() -> Self {
        Self {
            passed: true,
            stages: Vec::new(),
            error_count: 0,
            warning_count: 0,
        }
    }

    /// Add a stage result
    pub fn add_stage(&mut self, stage: ValidationStageResult) {
        if !stage.passed {
            self.passed = false;
        }
        self.error_count += stage.errors.len();
        self.warning_count += stage.warnings.len();
        self.stages.push(stage);
    }
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Result from a single validation stage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationStageResult {
    /// Stage name
    pub stage: ValidationStage,
    /// Whether this stage passed
    pub passed: bool,
    /// Errors found
    #[serde(default)]
    pub errors: Vec<ValidationMessage>,
    /// Warnings found
    #[serde(default)]
    pub warnings: Vec<ValidationMessage>,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

/// Validation pipeline stages
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStage {
    /// Check syntax (rustfmt)
    Syntax,
    /// Check linting (clippy)
    Linting,
    /// Type checking (cargo check)
    TypeCheck,
    /// Run unit tests
    UnitTests,
    /// Schema validation
    Schema,
    /// Security checks
    Security,
}

impl std::fmt::Display for ValidationStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationStage::Syntax => write!(f, "syntax"),
            ValidationStage::Linting => write!(f, "linting"),
            ValidationStage::TypeCheck => write!(f, "type_check"),
            ValidationStage::UnitTests => write!(f, "unit_tests"),
            ValidationStage::Schema => write!(f, "schema"),
            ValidationStage::Security => write!(f, "security"),
        }
    }
}

/// A validation message (error or warning)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationMessage {
    /// File path where the issue was found
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
    /// Column number
    pub column: Option<u32>,
    /// Message text
    pub message: String,
    /// Error/warning code
    pub code: Option<String>,
}

/// Response for listing agents
#[derive(Debug, Serialize)]
pub struct ListAgentsResponse {
    pub agents: Vec<Agent>,
    pub total: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_creation() {
        let agent = Agent::new(
            "test-analyzer".to_string(),
            AgentType::Analyzer,
            vec!["rust".to_string(), "typescript".to_string()],
        );

        assert_eq!(agent.name, "test-analyzer");
        assert_eq!(agent.agent_type, AgentType::Analyzer);
        assert_eq!(agent.status, AgentStatus::Idle);
        assert_eq!(agent.capabilities.len(), 2);
    }

    #[test]
    fn test_agent_task_lifecycle() {
        let mut agent = Agent::new(
            "test-coder".to_string(),
            AgentType::Coder,
            vec![],
        );

        assert_eq!(agent.status, AgentStatus::Idle);
        assert!(agent.current_task_id.is_none());

        agent.start_task("task-123".to_string());
        assert_eq!(agent.status, AgentStatus::Busy);
        assert_eq!(agent.current_task_id, Some("task-123".to_string()));

        agent.complete_task(true);
        assert_eq!(agent.status, AgentStatus::Idle);
        assert!(agent.current_task_id.is_none());
        assert_eq!(agent.tasks_completed, 1);
        assert_eq!(agent.tasks_failed, 0);

        agent.start_task("task-456".to_string());
        agent.complete_task(false);
        assert_eq!(agent.tasks_completed, 1);
        assert_eq!(agent.tasks_failed, 1);
    }

    #[test]
    fn test_analysis_result() {
        let safe = AnalysisResult::safe(vec!["src/utils.rs".to_string()]);
        assert!(!safe.requires_review);
        assert!(safe.risk_score < 0.5);

        let risky = AnalysisResult::high_risk(
            vec!["src/storage/engine.rs".to_string()],
            "Modifies core storage engine".to_string(),
        );
        assert!(risky.requires_review);
        assert!(risky.risk_score > 0.7);
    }

    #[test]
    fn test_validation_result() {
        let mut result = ValidationResult::new();
        assert!(result.passed);

        result.add_stage(ValidationStageResult {
            stage: ValidationStage::Syntax,
            passed: true,
            errors: vec![],
            warnings: vec![],
            duration_ms: 100,
        });
        assert!(result.passed);

        result.add_stage(ValidationStageResult {
            stage: ValidationStage::Linting,
            passed: false,
            errors: vec![ValidationMessage {
                file: Some("src/main.rs".to_string()),
                line: Some(10),
                column: Some(5),
                message: "unused variable".to_string(),
                code: Some("W001".to_string()),
            }],
            warnings: vec![],
            duration_ms: 200,
        });
        assert!(!result.passed);
        assert_eq!(result.error_count, 1);
    }
}
