//! Feedback types and capture operations for the AI learning system.
//!
//! This module provides types and functions for capturing and managing
//! feedback from human reviews, validation failures, test results, etc.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::DbError;
use crate::storage::StorageEngine;

use super::agent::ValidationResult;
use super::task::AITaskType;

/// System collection for storing feedback events
pub const FEEDBACK_COLLECTION: &str = "_ai_feedback";

// ============================================================================
// Feedback Types
// ============================================================================

/// Type of feedback event captured
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedbackType {
    /// Feedback from human review (approve/reject)
    HumanReview,
    /// Validation pipeline failure
    ValidationFailure,
    /// Test execution failure
    TestFailure,
    /// Task was escalated or required intervention
    TaskEscalation,
    /// Successful contribution merged
    Success,
}

impl std::fmt::Display for FeedbackType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedbackType::HumanReview => write!(f, "human_review"),
            FeedbackType::ValidationFailure => write!(f, "validation_failure"),
            FeedbackType::TestFailure => write!(f, "test_failure"),
            FeedbackType::TaskEscalation => write!(f, "task_escalation"),
            FeedbackType::Success => write!(f, "success"),
        }
    }
}

/// Outcome of the feedback event
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackOutcome {
    /// Positive outcome (approved, passed, succeeded)
    Positive,
    /// Negative outcome (rejected, failed)
    Negative,
    /// Neutral outcome (informational)
    Neutral,
}

/// A feedback event captured from the pipeline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackEvent {
    /// Unique identifier
    #[serde(rename = "_key")]
    pub id: String,
    /// Type of feedback
    pub feedback_type: FeedbackType,
    /// Associated contribution ID
    pub contribution_id: String,
    /// Associated task ID (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Agent that was involved (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Outcome of the event
    pub outcome: FeedbackOutcome,
    /// Context and details
    pub context: FeedbackContext,
    /// When the feedback was captured
    pub captured_at: DateTime<Utc>,
    /// Whether this feedback has been processed for patterns
    #[serde(default)]
    pub processed: bool,
}

/// Context for a feedback event
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FeedbackContext {
    /// Human-provided comments (from reviews)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<String>,
    /// Validation result details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation_result: Option<ValidationResult>,
    /// Test output details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_output: Option<TestOutput>,
    /// Error messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_messages: Option<Vec<String>>,
    /// Task type that was being executed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_type: Option<AITaskType>,
    /// Contribution type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contribution_type: Option<String>,
    /// Files involved
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files_affected: Option<Vec<String>>,
    /// Additional metadata
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, Value>,
}

/// Test execution output captured for feedback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestOutput {
    /// Total tests run
    pub tests_run: u32,
    /// Tests passed
    pub tests_passed: u32,
    /// Tests failed
    pub tests_failed: u32,
    /// Test names that failed
    #[serde(default)]
    pub failed_tests: Vec<String>,
    /// Raw output (truncated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<String>,
}

/// Query parameters for listing feedback
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FeedbackQuery {
    pub feedback_type: Option<FeedbackType>,
    pub outcome: Option<FeedbackOutcome>,
    pub contribution_id: Option<String>,
    pub agent_id: Option<String>,
    pub processed: Option<bool>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub limit: Option<usize>,
}

/// Response for listing feedback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListFeedbackResponse {
    pub feedback: Vec<FeedbackEvent>,
    pub total: usize,
    pub page: usize,
    pub page_size: usize,
}

// ============================================================================
// Feedback Capture Functions
// ============================================================================

/// FeedbackSystem provides functions for capturing and managing feedback
pub struct FeedbackSystem;

impl FeedbackSystem {
    /// Save feedback event to storage
    pub fn save_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        event: &FeedbackEvent,
    ) -> Result<(), DbError> {
        let db = storage.get_database(db_name)?;

        // Ensure collection exists
        if db.get_collection(FEEDBACK_COLLECTION).is_err() {
            db.create_collection(FEEDBACK_COLLECTION.to_string(), None)?;
        }

        let coll = db.get_collection(FEEDBACK_COLLECTION)?;
        let json = serde_json::to_value(event)
            .map_err(|e| DbError::InternalError(format!("Failed to serialize feedback: {}", e)))?;

        // Try to update first, then insert if not found
        if coll.get(&event.id).is_ok() {
            coll.update(&event.id, json)?;
        } else {
            coll.insert(json)?;
        }
        Ok(())
    }

    /// Get a feedback event by ID
    pub fn get_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        feedback_id: &str,
    ) -> Result<Option<FeedbackEvent>, DbError> {
        let db = storage.get_database(db_name)?;
        let coll = db.get_collection(FEEDBACK_COLLECTION)?;

        match coll.get(feedback_id) {
            Ok(doc) => Ok(Some(serde_json::from_value(doc.to_value())?)),
            Err(DbError::DocumentNotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// List feedback events with optional filters
    pub fn list_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        query: FeedbackQuery,
    ) -> Result<ListFeedbackResponse, DbError> {
        let db = storage.get_database(db_name)?;
        let coll = db.get_collection(FEEDBACK_COLLECTION)?;

        // Get all feedback and filter in memory
        let all_docs = coll.all();

        let mut filtered: Vec<FeedbackEvent> = all_docs
            .iter()
            .filter_map(|doc| {
                let value = doc.to_value();
                serde_json::from_value(value).ok()
            })
            .filter(|event: &FeedbackEvent| {
                // Apply filters
                if let Some(ref ft) = query.feedback_type {
                    if event.feedback_type != *ft {
                        return false;
                    }
                }
                if let Some(ref outcome) = query.outcome {
                    if event.outcome != *outcome {
                        return false;
                    }
                }
                if let Some(ref contrib_id) = query.contribution_id {
                    if event.contribution_id != *contrib_id {
                        return false;
                    }
                }
                if let Some(ref agent_id) = query.agent_id {
                    if event.agent_id.as_ref() != Some(agent_id) {
                        return false;
                    }
                }
                if let Some(processed) = query.processed {
                    if event.processed != processed {
                        return false;
                    }
                }
                if let Some(ref start) = query.start_date {
                    if event.captured_at < *start {
                        return false;
                    }
                }
                if let Some(ref end) = query.end_date {
                    if event.captured_at > *end {
                        return false;
                    }
                }
                true
            })
            .collect();

        // Sort by captured_at descending (most recent first)
        filtered.sort_by(|a, b| b.captured_at.cmp(&a.captured_at));

        let total = filtered.len();
        let page = 0;
        let page_size = query.limit.unwrap_or(50).min(100);

        let feedback: Vec<FeedbackEvent> = filtered.into_iter().take(page_size).collect();

        Ok(ListFeedbackResponse {
            feedback,
            total,
            page,
            page_size,
        })
    }

    /// Capture feedback from a human review (approve/reject)
    pub fn capture_review_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        contribution_id: &str,
        approved: bool,
        comments: Option<String>,
        agent_id: Option<String>,
    ) -> Result<FeedbackEvent, DbError> {
        let event = FeedbackEvent {
            id: uuid::Uuid::new_v4().to_string(),
            feedback_type: FeedbackType::HumanReview,
            contribution_id: contribution_id.to_string(),
            task_id: None,
            agent_id,
            outcome: if approved {
                FeedbackOutcome::Positive
            } else {
                FeedbackOutcome::Negative
            },
            context: FeedbackContext {
                comments,
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        Self::save_feedback(storage, db_name, &event)?;
        Ok(event)
    }

    /// Capture feedback from a validation result
    pub fn capture_validation_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        task: &super::task::AITask,
        validation_result: &ValidationResult,
    ) -> Result<FeedbackEvent, DbError> {
        let outcome = if validation_result.passed {
            FeedbackOutcome::Positive
        } else {
            FeedbackOutcome::Negative
        };

        // Extract error messages from failed stages
        let error_messages: Vec<String> = validation_result
            .stages
            .iter()
            .filter(|s| !s.passed)
            .flat_map(|s| s.errors.iter())
            .map(|m| m.message.clone())
            .collect();

        let event = FeedbackEvent {
            id: uuid::Uuid::new_v4().to_string(),
            feedback_type: FeedbackType::ValidationFailure,
            contribution_id: task.contribution_id.clone(),
            task_id: Some(task.id.clone()),
            agent_id: task.agent_id.clone(),
            outcome,
            context: FeedbackContext {
                validation_result: Some(validation_result.clone()),
                error_messages: if error_messages.is_empty() {
                    None
                } else {
                    Some(error_messages)
                },
                task_type: Some(task.task_type.clone()),
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        Self::save_feedback(storage, db_name, &event)?;
        Ok(event)
    }

    /// Capture feedback from test execution
    pub fn capture_test_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        task: &super::task::AITask,
        test_output: TestOutput,
    ) -> Result<FeedbackEvent, DbError> {
        let outcome = if test_output.tests_failed == 0 {
            FeedbackOutcome::Positive
        } else {
            FeedbackOutcome::Negative
        };

        let event = FeedbackEvent {
            id: uuid::Uuid::new_v4().to_string(),
            feedback_type: FeedbackType::TestFailure,
            contribution_id: task.contribution_id.clone(),
            task_id: Some(task.id.clone()),
            agent_id: task.agent_id.clone(),
            outcome,
            context: FeedbackContext {
                test_output: Some(test_output),
                task_type: Some(task.task_type.clone()),
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        Self::save_feedback(storage, db_name, &event)?;
        Ok(event)
    }

    /// Capture feedback when a contribution is successfully merged
    pub fn capture_success_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        contribution_id: &str,
        contribution_type: Option<String>,
        files_affected: Option<Vec<String>>,
        agent_id: Option<String>,
    ) -> Result<FeedbackEvent, DbError> {
        let event = FeedbackEvent {
            id: uuid::Uuid::new_v4().to_string(),
            feedback_type: FeedbackType::Success,
            contribution_id: contribution_id.to_string(),
            task_id: None,
            agent_id,
            outcome: FeedbackOutcome::Positive,
            context: FeedbackContext {
                contribution_type,
                files_affected,
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        Self::save_feedback(storage, db_name, &event)?;
        Ok(event)
    }

    /// Capture feedback when a task requires escalation
    pub fn capture_escalation_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        task: &super::task::AITask,
        reason: String,
    ) -> Result<FeedbackEvent, DbError> {
        let event = FeedbackEvent {
            id: uuid::Uuid::new_v4().to_string(),
            feedback_type: FeedbackType::TaskEscalation,
            contribution_id: task.contribution_id.clone(),
            task_id: Some(task.id.clone()),
            agent_id: task.agent_id.clone(),
            outcome: FeedbackOutcome::Neutral,
            context: FeedbackContext {
                error_messages: Some(vec![reason]),
                task_type: Some(task.task_type.clone()),
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        Self::save_feedback(storage, db_name, &event)?;
        Ok(event)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feedback_type_display() {
        assert_eq!(FeedbackType::HumanReview.to_string(), "human_review");
        assert_eq!(
            FeedbackType::ValidationFailure.to_string(),
            "validation_failure"
        );
        assert_eq!(FeedbackType::TestFailure.to_string(), "test_failure");
        assert_eq!(FeedbackType::TaskEscalation.to_string(), "task_escalation");
        assert_eq!(FeedbackType::Success.to_string(), "success");
    }

    #[test]
    fn test_feedback_event_creation() {
        let event = FeedbackEvent {
            id: "test-123".to_string(),
            feedback_type: FeedbackType::HumanReview,
            contribution_id: "contrib-456".to_string(),
            task_id: None,
            agent_id: Some("agent-789".to_string()),
            outcome: FeedbackOutcome::Positive,
            context: FeedbackContext {
                comments: Some("Looks good!".to_string()),
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        assert_eq!(event.feedback_type, FeedbackType::HumanReview);
        assert_eq!(event.outcome, FeedbackOutcome::Positive);
        assert!(!event.processed);
    }
}
