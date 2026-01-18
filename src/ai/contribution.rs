//! AI Contribution types for the AI-augmented database system
//!
//! This module defines the data structures for managing AI contributions
//! where users can describe needs in natural language and AI agents
//! help implement them.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Type of contribution being submitted
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContributionType {
    /// New feature request
    Feature,
    /// Bug fix request
    Bugfix,
    /// Enhancement to existing functionality
    Enhancement,
    /// Documentation improvement
    Documentation,
}

impl std::fmt::Display for ContributionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContributionType::Feature => write!(f, "feature"),
            ContributionType::Bugfix => write!(f, "bugfix"),
            ContributionType::Enhancement => write!(f, "enhancement"),
            ContributionType::Documentation => write!(f, "documentation"),
        }
    }
}

/// Status of a contribution through the pipeline
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ContributionStatus {
    /// Just submitted, waiting to be processed
    #[default]
    Submitted,
    /// AI is analyzing the request
    Analyzing,
    /// AI is generating code/changes
    Generating,
    /// Changes are being validated
    Validating,
    /// Waiting for human review
    Review,
    /// Approved by human reviewer
    Approved,
    /// Rejected by human reviewer
    Rejected,
    /// Successfully merged
    Merged,
    /// Cancelled by requester
    Cancelled,
}

impl std::fmt::Display for ContributionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContributionStatus::Submitted => write!(f, "submitted"),
            ContributionStatus::Analyzing => write!(f, "analyzing"),
            ContributionStatus::Generating => write!(f, "generating"),
            ContributionStatus::Validating => write!(f, "validating"),
            ContributionStatus::Review => write!(f, "review"),
            ContributionStatus::Approved => write!(f, "approved"),
            ContributionStatus::Rejected => write!(f, "rejected"),
            ContributionStatus::Merged => write!(f, "merged"),
            ContributionStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// Priority level for contributions
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Default)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    #[default]
    Medium,
    High,
    Critical,
}

/// Context information for a contribution
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ContributionContext {
    /// Related collections that might be affected
    #[serde(default)]
    pub related_collections: Vec<String>,
    /// Priority level
    #[serde(default)]
    pub priority: Priority,
    /// Additional metadata
    #[serde(default)]
    pub metadata: Option<Value>,
}

/// A contribution request from a user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contribution {
    /// Unique identifier (UUID)
    #[serde(rename = "_key")]
    pub id: String,
    /// Type of contribution
    pub contribution_type: ContributionType,
    /// Natural language description of the need
    pub description: String,
    /// Email or identifier of the requester
    pub requester: String,
    /// Current status in the pipeline
    pub status: ContributionStatus,
    /// When the contribution was created
    pub created_at: DateTime<Utc>,
    /// When the contribution was last updated
    pub updated_at: DateTime<Utc>,
    /// Optional context information
    #[serde(default)]
    pub context: ContributionContext,
    /// Feedback from reviewers (if rejected or revised)
    #[serde(default)]
    pub feedback: Option<String>,
    /// Risk score (0.0 - 1.0) calculated during analysis
    #[serde(default)]
    pub risk_score: Option<f64>,
    /// Affected files identified during analysis
    #[serde(default)]
    pub affected_files: Vec<String>,
}

impl Contribution {
    /// Create a new contribution
    pub fn new(
        contribution_type: ContributionType,
        description: String,
        requester: String,
        context: Option<ContributionContext>,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            contribution_type,
            description,
            requester,
            status: ContributionStatus::Submitted,
            created_at: now,
            updated_at: now,
            context: context.unwrap_or_default(),
            feedback: None,
            risk_score: None,
            affected_files: Vec::new(),
        }
    }

    /// Update the status of the contribution
    pub fn set_status(&mut self, status: ContributionStatus) {
        self.status = status;
        self.updated_at = Utc::now();
    }

    /// Check if the contribution requires human review based on risk
    pub fn requires_human_review(&self) -> bool {
        // Require review if risk score > 0.7 or if affecting core modules
        if let Some(risk) = self.risk_score {
            if risk > 0.7 {
                return true;
            }
        }

        // Check for core module modifications
        const CORE_MODULES: &[&str] = &[
            "storage/engine",
            "transaction/manager",
            "cluster/manager",
            "server/auth",
            "sync/worker",
        ];

        for file in &self.affected_files {
            for core in CORE_MODULES {
                if file.contains(core) {
                    return true;
                }
            }
        }

        false
    }
}

/// Request body for submitting a new contribution
#[derive(Debug, Clone, Deserialize)]
pub struct SubmitContributionRequest {
    #[serde(rename = "type")]
    pub contribution_type: ContributionType,
    pub description: String,
    #[serde(default)]
    pub context: Option<ContributionContext>,
}

/// Response after submitting a contribution
#[derive(Debug, Serialize)]
pub struct SubmitContributionResponse {
    pub status: String,
    pub id: String,
    pub message: String,
}

/// Request body for approving/rejecting a contribution
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewContributionRequest {
    #[serde(default)]
    pub feedback: Option<String>,
}

/// Response for listing contributions
#[derive(Debug, Serialize)]
pub struct ListContributionsResponse {
    pub contributions: Vec<Contribution>,
    pub total: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_contribution_creation() {
        let contrib = Contribution::new(
            ContributionType::Feature,
            "Add dark mode support".to_string(),
            "user@example.com".to_string(),
            None,
        );

        assert_eq!(contrib.contribution_type, ContributionType::Feature);
        assert_eq!(contrib.status, ContributionStatus::Submitted);
        assert!(!contrib.id.is_empty());
    }

    #[test]
    fn test_contribution_status_update() {
        let mut contrib = Contribution::new(
            ContributionType::Bugfix,
            "Fix login issue".to_string(),
            "dev@example.com".to_string(),
            None,
        );

        let original_updated = contrib.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(10));

        contrib.set_status(ContributionStatus::Analyzing);

        assert_eq!(contrib.status, ContributionStatus::Analyzing);
        assert!(contrib.updated_at > original_updated);
    }

    #[test]
    fn test_requires_human_review_high_risk() {
        let mut contrib = Contribution::new(
            ContributionType::Feature,
            "Add new auth method".to_string(),
            "user@example.com".to_string(),
            None,
        );

        contrib.risk_score = Some(0.8);
        assert!(contrib.requires_human_review());

        contrib.risk_score = Some(0.5);
        assert!(!contrib.requires_human_review());
    }

    #[test]
    fn test_requires_human_review_core_module() {
        let mut contrib = Contribution::new(
            ContributionType::Enhancement,
            "Optimize storage".to_string(),
            "user@example.com".to_string(),
            None,
        );

        contrib.affected_files = vec!["src/storage/engine.rs".to_string()];
        assert!(contrib.requires_human_review());

        contrib.affected_files = vec!["src/utils/helpers.rs".to_string()];
        assert!(!contrib.requires_human_review());
    }

    #[test]
    fn test_contribution_type_serialization() {
        let json = serde_json::to_string(&ContributionType::Feature).unwrap();
        assert_eq!(json, "\"feature\"");

        let parsed: ContributionType = serde_json::from_str("\"bugfix\"").unwrap();
        assert_eq!(parsed, ContributionType::Bugfix);
    }
}
