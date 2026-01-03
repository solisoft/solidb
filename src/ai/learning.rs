//! Learning Loops - Feedback capture and pattern extraction
//!
//! This module implements the learning system that captures feedback from
//! human reviews, validation failures, and test results to identify patterns
//! that can improve agent performance.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::DbError;
use crate::storage::StorageEngine;

use super::agent::ValidationResult;
use super::contribution::ContributionStatus;
use super::task::{AITask, AITaskType};

/// System collection for storing feedback events
pub const FEEDBACK_COLLECTION: &str = "_ai_feedback";

/// System collection for storing learned patterns
pub const PATTERNS_COLLECTION: &str = "_ai_patterns";

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

// ============================================================================
// Pattern Types
// ============================================================================

/// Type of learned pattern
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PatternType {
    /// Pattern that leads to success
    SuccessPattern,
    /// Pattern that should be avoided
    AntiPattern,
    /// Common error pattern
    ErrorPattern,
    /// Escalation pattern requiring human review
    EscalationPattern,
}

impl std::fmt::Display for PatternType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatternType::SuccessPattern => write!(f, "success_pattern"),
            PatternType::AntiPattern => write!(f, "anti_pattern"),
            PatternType::ErrorPattern => write!(f, "error_pattern"),
            PatternType::EscalationPattern => write!(f, "escalation_pattern"),
        }
    }
}

/// A learned pattern extracted from feedback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    /// Unique identifier
    #[serde(rename = "_key")]
    pub id: String,
    /// Type of pattern
    pub pattern_type: PatternType,
    /// Pattern signature for matching
    pub signature: PatternSignature,
    /// Confidence level (0.0 - 1.0)
    pub confidence: f64,
    /// Number of times this pattern was observed
    pub occurrence_count: u64,
    /// Suggested actions when pattern is matched
    pub suggested_actions: Vec<SuggestedAction>,
    /// When the pattern was first identified
    pub created_at: DateTime<Utc>,
    /// When the pattern was last updated
    pub updated_at: DateTime<Utc>,
    /// Feedback event IDs that contributed to this pattern
    #[serde(default)]
    pub source_feedback_ids: Vec<String>,
}

/// Signature for matching patterns
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PatternSignature {
    /// Keywords that indicate this pattern
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Regex patterns to match (stored as strings)
    #[serde(default)]
    pub regex_patterns: Vec<String>,
    /// Task types this pattern applies to
    #[serde(default)]
    pub applicable_task_types: Vec<AITaskType>,
    /// Contribution types this pattern applies to
    #[serde(default)]
    pub applicable_contribution_types: Vec<String>,
    /// Error codes or categories
    #[serde(default)]
    pub error_categories: Vec<String>,
}

/// Suggested action when a pattern is matched
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestedAction {
    /// Type of action
    pub action_type: ActionType,
    /// Description of the action
    pub description: String,
    /// Priority (higher = more important)
    pub priority: i32,
}

/// Types of suggested actions
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    /// Retry the task
    Retry,
    /// Escalate to human review
    Escalate,
    /// Skip this step
    Skip,
    /// Use a different agent
    ReassignAgent,
    /// Add additional validation
    AddValidation,
    /// Apply a specific fix
    ApplyFix,
    /// Log and monitor
    Monitor,
}

// ============================================================================
// Recommendation Types
// ============================================================================

/// A pattern match with context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternMatch {
    /// The matched pattern
    pub pattern: Pattern,
    /// Match score (0.0 - 1.0)
    pub match_score: f64,
    /// Which parts of the signature matched
    pub matched_components: Vec<String>,
}

/// A recommendation based on pattern matching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    /// The pattern that triggered this recommendation
    pub pattern_id: String,
    /// Suggested action
    pub action: SuggestedAction,
    /// Confidence in this recommendation
    pub confidence: f64,
    /// Reason for the recommendation
    pub reason: String,
}

// ============================================================================
// Learning System
// ============================================================================

/// Result of processing a batch of feedback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingResult {
    /// Number of feedback events processed
    pub processed_count: u64,
    /// Number of new patterns created
    pub patterns_created: u64,
    /// Number of existing patterns updated
    pub patterns_updated: u64,
    /// Any errors encountered
    #[serde(default)]
    pub errors: Vec<String>,
}

/// Response for listing feedback events
#[derive(Debug, Serialize)]
pub struct ListFeedbackResponse {
    pub feedback: Vec<FeedbackEvent>,
    pub total: usize,
}

/// Response for listing patterns
#[derive(Debug, Serialize)]
pub struct ListPatternsResponse {
    pub patterns: Vec<Pattern>,
    pub total: usize,
}

/// Query for filtering feedback events
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FeedbackQuery {
    pub feedback_type: Option<FeedbackType>,
    pub outcome: Option<FeedbackOutcome>,
    pub contribution_id: Option<String>,
    pub agent_id: Option<String>,
    pub processed: Option<bool>,
    pub limit: Option<usize>,
}

/// Query for filtering patterns
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PatternQuery {
    pub pattern_type: Option<PatternType>,
    pub task_type: Option<AITaskType>,
    pub min_confidence: Option<f64>,
    pub limit: Option<usize>,
}

/// The Learning System for capturing feedback and extracting patterns
pub struct LearningSystem;

impl LearningSystem {
    // ========================================================================
    // Feedback Capture
    // ========================================================================

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
        task: &AITask,
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
        task: &AITask,
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

    /// Capture feedback when a task is escalated
    pub fn capture_escalation_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        task: &AITask,
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
                comments: Some(reason),
                task_type: Some(task.task_type.clone()),
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        Self::save_feedback(storage, db_name, &event)?;
        Ok(event)
    }

    // ========================================================================
    // Pattern Learning
    // ========================================================================

    /// Process a batch of unprocessed feedback to extract patterns
    pub fn process_feedback_batch(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        limit: usize,
    ) -> Result<ProcessingResult, DbError> {
        let mut result = ProcessingResult {
            processed_count: 0,
            patterns_created: 0,
            patterns_updated: 0,
            errors: Vec::new(),
        };

        // Get unprocessed feedback
        let query = FeedbackQuery {
            processed: Some(false),
            limit: Some(limit),
            ..Default::default()
        };
        let feedback_list = Self::list_feedback(storage, db_name, &query)?;

        for mut event in feedback_list.feedback {
            // Extract patterns from this feedback
            match Self::extract_patterns_from_event(&event) {
                Ok(candidates) => {
                    for candidate in candidates {
                        match Self::upsert_pattern(storage, db_name, candidate, &event.id) {
                            Ok(true) => result.patterns_created += 1,
                            Ok(false) => result.patterns_updated += 1,
                            Err(e) => result.errors.push(format!(
                                "Failed to upsert pattern for event {}: {}",
                                event.id, e
                            )),
                        }
                    }

                    // Mark feedback as processed
                    event.processed = true;
                    if let Err(e) = Self::save_feedback(storage, db_name, &event) {
                        result
                            .errors
                            .push(format!("Failed to mark event {} as processed: {}", event.id, e));
                    }
                    result.processed_count += 1;
                }
                Err(e) => {
                    result.errors.push(format!(
                        "Failed to extract patterns from event {}: {}",
                        event.id, e
                    ));
                }
            }
        }

        Ok(result)
    }

    /// Extract pattern candidates from a feedback event
    fn extract_patterns_from_event(event: &FeedbackEvent) -> Result<Vec<Pattern>, DbError> {
        let mut patterns = Vec::new();

        match event.feedback_type {
            FeedbackType::ValidationFailure | FeedbackType::TestFailure => {
                if event.outcome == FeedbackOutcome::Negative {
                    // Extract error pattern
                    if let Some(error_pattern) = Self::extract_error_pattern(event) {
                        patterns.push(error_pattern);
                    }
                }
            }
            FeedbackType::HumanReview => {
                if event.outcome == FeedbackOutcome::Negative {
                    // Extract anti-pattern from rejection
                    if let Some(anti_pattern) = Self::extract_anti_pattern(event) {
                        patterns.push(anti_pattern);
                    }
                } else if event.outcome == FeedbackOutcome::Positive {
                    // Extract success pattern from approval
                    if let Some(success_pattern) = Self::extract_success_pattern(event) {
                        patterns.push(success_pattern);
                    }
                }
            }
            FeedbackType::TaskEscalation => {
                // Extract escalation pattern
                if let Some(escalation_pattern) = Self::extract_escalation_pattern(event) {
                    patterns.push(escalation_pattern);
                }
            }
            FeedbackType::Success => {
                // Extract success pattern
                if let Some(success_pattern) = Self::extract_success_pattern(event) {
                    patterns.push(success_pattern);
                }
            }
        }

        Ok(patterns)
    }

    /// Extract an error pattern from failed validation/test
    fn extract_error_pattern(event: &FeedbackEvent) -> Option<Pattern> {
        let mut keywords = Vec::new();
        let mut error_categories = Vec::new();

        // Extract keywords from error messages
        if let Some(errors) = &event.context.error_messages {
            for error in errors {
                // Extract common error keywords
                let error_lower = error.to_lowercase();
                if error_lower.contains("type") || error_lower.contains("mismatch") {
                    keywords.push("type_error".to_string());
                    error_categories.push("type_safety".to_string());
                }
                if error_lower.contains("borrow") || error_lower.contains("lifetime") {
                    keywords.push("borrow_error".to_string());
                    error_categories.push("memory_safety".to_string());
                }
                if error_lower.contains("unused") {
                    keywords.push("unused_code".to_string());
                    error_categories.push("lint".to_string());
                }
                if error_lower.contains("test") || error_lower.contains("assert") {
                    keywords.push("test_failure".to_string());
                    error_categories.push("testing".to_string());
                }
            }
        }

        if keywords.is_empty() {
            return None;
        }

        // Deduplicate
        keywords.sort();
        keywords.dedup();
        error_categories.sort();
        error_categories.dedup();

        Some(Pattern {
            id: format!("error_{}", uuid::Uuid::new_v4()),
            pattern_type: PatternType::ErrorPattern,
            signature: PatternSignature {
                keywords,
                error_categories,
                applicable_task_types: event.context.task_type.iter().cloned().collect(),
                ..Default::default()
            },
            confidence: 0.5, // Initial confidence
            occurrence_count: 1,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::Retry,
                    description: "Retry with error context provided to agent".to_string(),
                    priority: 1,
                },
                SuggestedAction {
                    action_type: ActionType::Escalate,
                    description: "Escalate if retry fails".to_string(),
                    priority: 2,
                },
            ],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source_feedback_ids: vec![],
        })
    }

    /// Extract an anti-pattern from a rejected review
    fn extract_anti_pattern(event: &FeedbackEvent) -> Option<Pattern> {
        let comments = event.context.comments.as_ref()?;
        let comments_lower = comments.to_lowercase();

        let mut keywords = Vec::new();

        // Extract keywords from rejection comments
        if comments_lower.contains("security") || comments_lower.contains("unsafe") {
            keywords.push("security_concern".to_string());
        }
        if comments_lower.contains("performance") || comments_lower.contains("slow") {
            keywords.push("performance_issue".to_string());
        }
        if comments_lower.contains("style") || comments_lower.contains("format") {
            keywords.push("style_violation".to_string());
        }
        if comments_lower.contains("logic") || comments_lower.contains("bug") {
            keywords.push("logic_error".to_string());
        }
        if comments_lower.contains("incomplete") || comments_lower.contains("missing") {
            keywords.push("incomplete_implementation".to_string());
        }

        if keywords.is_empty() {
            keywords.push("general_rejection".to_string());
        }

        Some(Pattern {
            id: format!("anti_{}", uuid::Uuid::new_v4()),
            pattern_type: PatternType::AntiPattern,
            signature: PatternSignature {
                keywords,
                ..Default::default()
            },
            confidence: 0.6, // Higher initial confidence for human feedback
            occurrence_count: 1,
            suggested_actions: vec![
                SuggestedAction {
                    action_type: ActionType::ReassignAgent,
                    description: "Consider using a different agent for similar tasks".to_string(),
                    priority: 1,
                },
                SuggestedAction {
                    action_type: ActionType::AddValidation,
                    description: "Add additional validation for this pattern".to_string(),
                    priority: 2,
                },
            ],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source_feedback_ids: vec![],
        })
    }

    /// Extract a success pattern from approval or successful merge
    fn extract_success_pattern(event: &FeedbackEvent) -> Option<Pattern> {
        let mut keywords = Vec::new();

        // Extract context from contribution type
        if let Some(contrib_type) = &event.context.contribution_type {
            keywords.push(format!("contrib_{}", contrib_type.to_lowercase()));
        }

        // Extract from files affected
        if let Some(files) = &event.context.files_affected {
            for file in files {
                if file.contains("test") {
                    keywords.push("includes_tests".to_string());
                }
                if file.ends_with(".rs") {
                    keywords.push("rust_code".to_string());
                }
            }
        }

        if keywords.is_empty() {
            keywords.push("successful_contribution".to_string());
        }

        // Deduplicate
        keywords.sort();
        keywords.dedup();

        Some(Pattern {
            id: format!("success_{}", uuid::Uuid::new_v4()),
            pattern_type: PatternType::SuccessPattern,
            signature: PatternSignature {
                keywords,
                applicable_contribution_types: event
                    .context
                    .contribution_type
                    .iter()
                    .cloned()
                    .collect(),
                ..Default::default()
            },
            confidence: 0.7, // High confidence for success
            occurrence_count: 1,
            suggested_actions: vec![SuggestedAction {
                action_type: ActionType::Monitor,
                description: "Continue using this approach for similar tasks".to_string(),
                priority: 1,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source_feedback_ids: vec![],
        })
    }

    /// Extract an escalation pattern
    fn extract_escalation_pattern(event: &FeedbackEvent) -> Option<Pattern> {
        let reason = event.context.comments.as_ref()?;
        let reason_lower = reason.to_lowercase();

        let mut keywords = Vec::new();

        if reason_lower.contains("timeout") || reason_lower.contains("stuck") {
            keywords.push("task_timeout".to_string());
        }
        if reason_lower.contains("complex") || reason_lower.contains("difficult") {
            keywords.push("high_complexity".to_string());
        }
        if reason_lower.contains("unclear") || reason_lower.contains("ambiguous") {
            keywords.push("unclear_requirements".to_string());
        }

        if keywords.is_empty() {
            keywords.push("general_escalation".to_string());
        }

        Some(Pattern {
            id: format!("escalation_{}", uuid::Uuid::new_v4()),
            pattern_type: PatternType::EscalationPattern,
            signature: PatternSignature {
                keywords,
                applicable_task_types: event.context.task_type.iter().cloned().collect(),
                ..Default::default()
            },
            confidence: 0.5,
            occurrence_count: 1,
            suggested_actions: vec![SuggestedAction {
                action_type: ActionType::Escalate,
                description: "Early escalation recommended for similar patterns".to_string(),
                priority: 1,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source_feedback_ids: vec![],
        })
    }

    /// Upsert a pattern (create new or update existing)
    /// Returns true if created, false if updated
    fn upsert_pattern(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        mut pattern: Pattern,
        feedback_id: &str,
    ) -> Result<bool, DbError> {
        // Try to find existing pattern with similar signature
        let existing = Self::find_matching_pattern(storage, db_name, &pattern.signature)?;

        if let Some(mut existing_pattern) = existing {
            // Update existing pattern
            existing_pattern.occurrence_count += 1;
            existing_pattern.updated_at = Utc::now();
            existing_pattern.source_feedback_ids.push(feedback_id.to_string());

            // Increase confidence with more occurrences (up to 0.95)
            existing_pattern.confidence =
                (existing_pattern.confidence + 0.05).min(0.95);

            Self::save_pattern(storage, db_name, &existing_pattern)?;
            Ok(false)
        } else {
            // Create new pattern
            pattern.source_feedback_ids.push(feedback_id.to_string());
            Self::save_pattern(storage, db_name, &pattern)?;
            Ok(true)
        }
    }

    /// Find a pattern with matching signature
    fn find_matching_pattern(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        signature: &PatternSignature,
    ) -> Result<Option<Pattern>, DbError> {
        // Get all patterns and find one with overlapping keywords
        let patterns = Self::list_patterns(
            storage,
            db_name,
            &PatternQuery {
                limit: Some(100),
                ..Default::default()
            },
        )?;

        for pattern in patterns.patterns {
            // Check for keyword overlap
            let overlap: usize = pattern
                .signature
                .keywords
                .iter()
                .filter(|k| signature.keywords.contains(k))
                .count();

            // If more than half the keywords match, consider it the same pattern
            if !signature.keywords.is_empty()
                && overlap > signature.keywords.len() / 2
            {
                return Ok(Some(pattern));
            }
        }

        Ok(None)
    }

    // ========================================================================
    // Pattern Application
    // ========================================================================

    /// Match patterns against a context
    pub fn match_patterns(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        context: &FeedbackContext,
    ) -> Result<Vec<PatternMatch>, DbError> {
        let patterns = Self::list_patterns(
            storage,
            db_name,
            &PatternQuery {
                limit: Some(100),
                ..Default::default()
            },
        )?;

        let mut matches = Vec::new();

        for pattern in patterns.patterns {
            if let Some(pattern_match) = Self::score_pattern_match(&pattern, context) {
                matches.push(pattern_match);
            }
        }

        // Sort by match score descending
        matches.sort_by(|a, b| b.match_score.partial_cmp(&a.match_score).unwrap());

        Ok(matches)
    }

    /// Score how well a pattern matches a context
    fn score_pattern_match(pattern: &Pattern, context: &FeedbackContext) -> Option<PatternMatch> {
        let mut score = 0.0;
        let mut matched_components = Vec::new();

        // Check task type match
        if let Some(task_type) = &context.task_type {
            if pattern.signature.applicable_task_types.contains(task_type) {
                score += 0.3;
                matched_components.push(format!("task_type:{}", task_type));
            }
        }

        // Check contribution type match
        if let Some(contrib_type) = &context.contribution_type {
            if pattern
                .signature
                .applicable_contribution_types
                .contains(contrib_type)
            {
                score += 0.2;
                matched_components.push(format!("contribution_type:{}", contrib_type));
            }
        }

        // Check error category match
        if let Some(errors) = &context.error_messages {
            let error_text = errors.join(" ").to_lowercase();
            for category in &pattern.signature.error_categories {
                if error_text.contains(&category.to_lowercase()) {
                    score += 0.2;
                    matched_components.push(format!("error_category:{}", category));
                }
            }
        }

        // Check keyword match in comments
        if let Some(comments) = &context.comments {
            let comments_lower = comments.to_lowercase();
            for keyword in &pattern.signature.keywords {
                if comments_lower.contains(&keyword.to_lowercase()) {
                    score += 0.1;
                    matched_components.push(format!("keyword:{}", keyword));
                }
            }
        }

        // Apply pattern confidence
        score *= pattern.confidence;

        if score > 0.1 {
            Some(PatternMatch {
                pattern: pattern.clone(),
                match_score: score,
                matched_components,
            })
        } else {
            None
        }
    }

    /// Get recommendations for a task based on pattern matching
    pub fn get_recommendations(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        task: &AITask,
        contribution_status: Option<&ContributionStatus>,
    ) -> Result<Vec<Recommendation>, DbError> {
        let context = FeedbackContext {
            task_type: Some(task.task_type.clone()),
            ..Default::default()
        };

        let pattern_matches = Self::match_patterns(storage, db_name, &context)?;

        let mut recommendations = Vec::new();

        for pattern_match in pattern_matches.iter().take(5) {
            for action in &pattern_match.pattern.suggested_actions {
                recommendations.push(Recommendation {
                    pattern_id: pattern_match.pattern.id.clone(),
                    action: action.clone(),
                    confidence: pattern_match.match_score * pattern_match.pattern.confidence,
                    reason: format!(
                        "Matched pattern '{}' with score {:.2} based on: {}",
                        pattern_match.pattern.pattern_type,
                        pattern_match.match_score,
                        pattern_match.matched_components.join(", ")
                    ),
                });
            }
        }

        // Sort by confidence
        recommendations.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());

        // Suppress if contribution is already merged
        if let Some(ContributionStatus::Merged) = contribution_status {
            recommendations.retain(|r| r.action.action_type == ActionType::Monitor);
        }

        Ok(recommendations)
    }

    // ========================================================================
    // Storage Operations
    // ========================================================================

    /// Save a feedback event to storage
    fn save_feedback(
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

    /// List feedback events with optional filtering
    pub fn list_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        query: &FeedbackQuery,
    ) -> Result<ListFeedbackResponse, DbError> {
        let db = storage.get_database(db_name)?;

        if db.get_collection(FEEDBACK_COLLECTION).is_err() {
            return Ok(ListFeedbackResponse {
                feedback: Vec::new(),
                total: 0,
            });
        }

        let coll = db.get_collection(FEEDBACK_COLLECTION)?;
        let limit = query.limit.unwrap_or(100);

        let feedback: Vec<FeedbackEvent> = coll
            .scan(None)
            .into_iter()
            .filter_map(|doc| serde_json::from_value::<FeedbackEvent>(doc.to_value()).ok())
            .filter(|e: &FeedbackEvent| {
                // Apply filters
                if let Some(ft) = &query.feedback_type {
                    if &e.feedback_type != ft {
                        return false;
                    }
                }
                if let Some(outcome) = &query.outcome {
                    if &e.outcome != outcome {
                        return false;
                    }
                }
                if let Some(cid) = &query.contribution_id {
                    if &e.contribution_id != cid {
                        return false;
                    }
                }
                if let Some(aid) = &query.agent_id {
                    if e.agent_id.as_ref() != Some(aid) {
                        return false;
                    }
                }
                if let Some(processed) = query.processed {
                    if e.processed != processed {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .collect();

        let total = feedback.len();
        Ok(ListFeedbackResponse { feedback, total })
    }

    /// Get a specific feedback event
    pub fn get_feedback(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        feedback_id: &str,
    ) -> Result<FeedbackEvent, DbError> {
        let db = storage.get_database(db_name)?;

        if db.get_collection(FEEDBACK_COLLECTION).is_err() {
            return Err(DbError::DocumentNotFound(format!(
                "Feedback {} not found",
                feedback_id
            )));
        }

        let coll = db.get_collection(FEEDBACK_COLLECTION)?;
        let doc = coll.get(feedback_id)?;
        serde_json::from_value(doc.to_value())
            .map_err(|e| DbError::InternalError(format!("Failed to deserialize feedback: {}", e)))
    }

    /// Save a pattern to storage
    fn save_pattern(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        pattern: &Pattern,
    ) -> Result<(), DbError> {
        let db = storage.get_database(db_name)?;

        // Ensure collection exists
        if db.get_collection(PATTERNS_COLLECTION).is_err() {
            db.create_collection(PATTERNS_COLLECTION.to_string(), None)?;
        }

        let coll = db.get_collection(PATTERNS_COLLECTION)?;
        let json = serde_json::to_value(pattern)
            .map_err(|e| DbError::InternalError(format!("Failed to serialize pattern: {}", e)))?;

        // Try to update first, then insert if not found
        if coll.get(&pattern.id).is_ok() {
            coll.update(&pattern.id, json)?;
        } else {
            coll.insert(json)?;
        }
        Ok(())
    }

    /// List patterns with optional filtering
    pub fn list_patterns(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        query: &PatternQuery,
    ) -> Result<ListPatternsResponse, DbError> {
        let db = storage.get_database(db_name)?;

        if db.get_collection(PATTERNS_COLLECTION).is_err() {
            return Ok(ListPatternsResponse {
                patterns: Vec::new(),
                total: 0,
            });
        }

        let coll = db.get_collection(PATTERNS_COLLECTION)?;
        let limit = query.limit.unwrap_or(100);

        let patterns: Vec<Pattern> = coll
            .scan(None)
            .into_iter()
            .filter_map(|doc| serde_json::from_value::<Pattern>(doc.to_value()).ok())
            .filter(|p: &Pattern| {
                // Apply filters
                if let Some(pt) = &query.pattern_type {
                    if &p.pattern_type != pt {
                        return false;
                    }
                }
                if let Some(tt) = &query.task_type {
                    if !p.signature.applicable_task_types.contains(tt) {
                        return false;
                    }
                }
                if let Some(min_conf) = query.min_confidence {
                    if p.confidence < min_conf {
                        return false;
                    }
                }
                true
            })
            .take(limit)
            .collect();

        let total = patterns.len();
        Ok(ListPatternsResponse { patterns, total })
    }

    /// Get a specific pattern
    pub fn get_pattern(
        storage: &Arc<StorageEngine>,
        db_name: &str,
        pattern_id: &str,
    ) -> Result<Pattern, DbError> {
        let db = storage.get_database(db_name)?;

        if db.get_collection(PATTERNS_COLLECTION).is_err() {
            return Err(DbError::DocumentNotFound(format!(
                "Pattern {} not found",
                pattern_id
            )));
        }

        let coll = db.get_collection(PATTERNS_COLLECTION)?;
        let doc = coll.get(pattern_id)?;
        serde_json::from_value(doc.to_value())
            .map_err(|e| DbError::InternalError(format!("Failed to deserialize pattern: {}", e)))
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
    fn test_pattern_type_display() {
        assert_eq!(PatternType::SuccessPattern.to_string(), "success_pattern");
        assert_eq!(PatternType::AntiPattern.to_string(), "anti_pattern");
        assert_eq!(PatternType::ErrorPattern.to_string(), "error_pattern");
        assert_eq!(
            PatternType::EscalationPattern.to_string(),
            "escalation_pattern"
        );
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

    #[test]
    fn test_pattern_creation() {
        let pattern = Pattern {
            id: "pattern-123".to_string(),
            pattern_type: PatternType::ErrorPattern,
            signature: PatternSignature {
                keywords: vec!["type_error".to_string(), "mismatch".to_string()],
                error_categories: vec!["type_safety".to_string()],
                ..Default::default()
            },
            confidence: 0.75,
            occurrence_count: 5,
            suggested_actions: vec![SuggestedAction {
                action_type: ActionType::Retry,
                description: "Retry with context".to_string(),
                priority: 1,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source_feedback_ids: vec!["fb-1".to_string(), "fb-2".to_string()],
        };

        assert_eq!(pattern.pattern_type, PatternType::ErrorPattern);
        assert_eq!(pattern.confidence, 0.75);
        assert_eq!(pattern.occurrence_count, 5);
    }

    #[test]
    fn test_extract_error_pattern_type_error() {
        let event = FeedbackEvent {
            id: "test-123".to_string(),
            feedback_type: FeedbackType::ValidationFailure,
            contribution_id: "contrib-456".to_string(),
            task_id: Some("task-789".to_string()),
            agent_id: None,
            outcome: FeedbackOutcome::Negative,
            context: FeedbackContext {
                error_messages: Some(vec![
                    "error[E0308]: mismatched types".to_string(),
                    "expected `String`, found `&str`".to_string(),
                ]),
                task_type: Some(AITaskType::ValidateCode),
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        let pattern = LearningSystem::extract_error_pattern(&event).unwrap();
        assert_eq!(pattern.pattern_type, PatternType::ErrorPattern);
        assert!(pattern.signature.keywords.contains(&"type_error".to_string()));
        assert!(pattern
            .signature
            .error_categories
            .contains(&"type_safety".to_string()));
    }

    #[test]
    fn test_extract_error_pattern_borrow() {
        let event = FeedbackEvent {
            id: "test-124".to_string(),
            feedback_type: FeedbackType::ValidationFailure,
            contribution_id: "contrib-456".to_string(),
            task_id: None,
            agent_id: None,
            outcome: FeedbackOutcome::Negative,
            context: FeedbackContext {
                error_messages: Some(vec![
                    "error[E0502]: cannot borrow `x` as mutable".to_string(),
                ]),
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        let pattern = LearningSystem::extract_error_pattern(&event).unwrap();
        assert!(pattern
            .signature
            .keywords
            .contains(&"borrow_error".to_string()));
        assert!(pattern
            .signature
            .error_categories
            .contains(&"memory_safety".to_string()));
    }

    #[test]
    fn test_extract_anti_pattern() {
        let event = FeedbackEvent {
            id: "test-125".to_string(),
            feedback_type: FeedbackType::HumanReview,
            contribution_id: "contrib-456".to_string(),
            task_id: None,
            agent_id: None,
            outcome: FeedbackOutcome::Negative,
            context: FeedbackContext {
                comments: Some(
                    "Security concern: input not validated properly. Also performance issues."
                        .to_string(),
                ),
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        let pattern = LearningSystem::extract_anti_pattern(&event).unwrap();
        assert_eq!(pattern.pattern_type, PatternType::AntiPattern);
        assert!(pattern
            .signature
            .keywords
            .contains(&"security_concern".to_string()));
        assert!(pattern
            .signature
            .keywords
            .contains(&"performance_issue".to_string()));
    }

    #[test]
    fn test_extract_success_pattern() {
        let event = FeedbackEvent {
            id: "test-126".to_string(),
            feedback_type: FeedbackType::Success,
            contribution_id: "contrib-456".to_string(),
            task_id: None,
            agent_id: None,
            outcome: FeedbackOutcome::Positive,
            context: FeedbackContext {
                contribution_type: Some("feature".to_string()),
                files_affected: Some(vec![
                    "src/main.rs".to_string(),
                    "tests/integration_test.rs".to_string(),
                ]),
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        let pattern = LearningSystem::extract_success_pattern(&event).unwrap();
        assert_eq!(pattern.pattern_type, PatternType::SuccessPattern);
        assert!(pattern
            .signature
            .keywords
            .contains(&"contrib_feature".to_string()));
        assert!(pattern
            .signature
            .keywords
            .contains(&"includes_tests".to_string()));
        assert!(pattern.signature.keywords.contains(&"rust_code".to_string()));
    }

    #[test]
    fn test_extract_escalation_pattern() {
        let event = FeedbackEvent {
            id: "test-127".to_string(),
            feedback_type: FeedbackType::TaskEscalation,
            contribution_id: "contrib-456".to_string(),
            task_id: Some("task-789".to_string()),
            agent_id: Some("agent-001".to_string()),
            outcome: FeedbackOutcome::Neutral,
            context: FeedbackContext {
                comments: Some("Task timeout after 10 minutes, stuck on complex parsing".to_string()),
                task_type: Some(AITaskType::GenerateCode),
                ..Default::default()
            },
            captured_at: Utc::now(),
            processed: false,
        };

        let pattern = LearningSystem::extract_escalation_pattern(&event).unwrap();
        assert_eq!(pattern.pattern_type, PatternType::EscalationPattern);
        assert!(pattern
            .signature
            .keywords
            .contains(&"task_timeout".to_string()));
        assert!(pattern
            .signature
            .keywords
            .contains(&"high_complexity".to_string()));
    }

    #[test]
    fn test_pattern_serialization() {
        let pattern = Pattern {
            id: "pattern-123".to_string(),
            pattern_type: PatternType::ErrorPattern,
            signature: PatternSignature {
                keywords: vec!["test".to_string()],
                ..Default::default()
            },
            confidence: 0.8,
            occurrence_count: 3,
            suggested_actions: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source_feedback_ids: vec![],
        };

        let json = serde_json::to_string(&pattern).unwrap();
        let parsed: Pattern = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, pattern.id);
        assert_eq!(parsed.pattern_type, pattern.pattern_type);
        assert_eq!(parsed.confidence, pattern.confidence);
    }

    #[test]
    fn test_test_output() {
        let output = TestOutput {
            tests_run: 100,
            tests_passed: 95,
            tests_failed: 5,
            failed_tests: vec!["test_one".to_string(), "test_two".to_string()],
            raw_output: Some("test output...".to_string()),
        };

        assert_eq!(output.tests_run, 100);
        assert_eq!(output.tests_passed, 95);
        assert_eq!(output.tests_failed, 5);
        assert_eq!(output.failed_tests.len(), 2);
    }
}
