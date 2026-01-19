//! Learning System handlers
//!
//! Provides endpoints for feedback and pattern management.

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;

use crate::ai::{
    FeedbackEvent, FeedbackOutcome, FeedbackQuery, FeedbackSystem, FeedbackType, LearningSystem,
    ListFeedbackResponse, ListPatternsResponse, Pattern, PatternQuery, ProcessingResult,
    Recommendation,
};
use crate::error::DbError;
use crate::server::handlers::AppState;

/// Query parameters for listing feedback
#[derive(Debug, Deserialize)]
pub struct ListFeedbackQueryParams {
    /// Filter by feedback type
    pub feedback_type: Option<String>,
    /// Filter by outcome
    pub outcome: Option<String>,
    /// Filter by contribution ID
    pub contribution_id: Option<String>,
    /// Filter by agent ID
    pub agent_id: Option<String>,
    /// Filter by processed status
    pub processed: Option<bool>,
    /// Limit results
    pub limit: Option<usize>,
}

impl ListFeedbackQueryParams {
    fn to_feedback_query(&self) -> FeedbackQuery {
        FeedbackQuery {
            feedback_type: self.feedback_type.as_ref().and_then(|t| {
                match t.to_lowercase().as_str() {
                    "human_review" => Some(FeedbackType::HumanReview),
                    "validation_failure" => Some(FeedbackType::ValidationFailure),
                    "test_failure" => Some(FeedbackType::TestFailure),
                    "task_escalation" => Some(FeedbackType::TaskEscalation),
                    "success" => Some(FeedbackType::Success),
                    _ => None,
                }
            }),
            outcome: self
                .outcome
                .as_ref()
                .and_then(|o| match o.to_lowercase().as_str() {
                    "positive" => Some(FeedbackOutcome::Positive),
                    "negative" => Some(FeedbackOutcome::Negative),
                    "neutral" => Some(FeedbackOutcome::Neutral),
                    _ => None,
                }),
            contribution_id: self.contribution_id.clone(),
            agent_id: self.agent_id.clone(),
            processed: self.processed,
            start_date: None,
            end_date: None,
            limit: self.limit,
        }
    }
}

/// GET /_api/database/{db}/ai/learning/feedback - List feedback events
///
/// Returns feedback events with optional filtering
pub async fn list_feedback_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(params): Query<ListFeedbackQueryParams>,
) -> Result<Json<ListFeedbackResponse>, DbError> {
    let query = params.to_feedback_query();
    let response = FeedbackSystem::list_feedback(&state.storage, &db_name, query)?;
    Ok(Json(response))
}

/// GET /_api/database/{db}/ai/learning/feedback/{id} - Get a specific feedback event
pub async fn get_feedback_handler(
    State(state): State<AppState>,
    Path((db_name, feedback_id)): Path<(String, String)>,
) -> Result<Json<FeedbackEvent>, DbError> {
    let event = FeedbackSystem::get_feedback(&state.storage, &db_name, &feedback_id)?;
    match event {
        Some(e) => Ok(Json(e)),
        None => Err(DbError::DocumentNotFound(format!(
            "Feedback {}",
            feedback_id
        ))),
    }
}

/// Query parameters for listing patterns
#[derive(Debug, Deserialize)]
pub struct ListPatternsQueryParams {
    /// Filter by pattern type
    pub pattern_type: Option<String>,
    /// Filter by applicable task type
    pub task_type: Option<String>,
    /// Minimum confidence threshold
    pub min_confidence: Option<f64>,
    /// Limit results
    pub limit: Option<usize>,
}

impl ListPatternsQueryParams {
    fn to_pattern_query(&self) -> PatternQuery {
        PatternQuery {
            pattern_type: self.pattern_type.as_ref().and_then(|t| {
                match t.to_lowercase().as_str() {
                    "error_pattern" => Some(crate::ai::PatternType::ErrorPattern),
                    "success_pattern" => Some(crate::ai::PatternType::SuccessPattern),
                    "anti_pattern" => Some(crate::ai::PatternType::AntiPattern),
                    "escalation_pattern" => Some(crate::ai::PatternType::EscalationPattern),
                    _ => None,
                }
            }),
            task_type: self
                .task_type
                .as_ref()
                .and_then(|t| match t.to_lowercase().as_str() {
                    "analyze_contribution" => Some(crate::ai::AITaskType::AnalyzeContribution),
                    "generate_code" => Some(crate::ai::AITaskType::GenerateCode),
                    "validate_code" => Some(crate::ai::AITaskType::ValidateCode),
                    "run_tests" => Some(crate::ai::AITaskType::RunTests),
                    "prepare_review" => Some(crate::ai::AITaskType::PrepareReview),
                    "merge_changes" => Some(crate::ai::AITaskType::MergeChanges),
                    _ => None,
                }),
            min_confidence: self.min_confidence,
            limit: self.limit,
        }
    }
}

/// GET /_api/database/{db}/ai/learning/patterns - List learned patterns
///
/// Returns patterns extracted from feedback
pub async fn list_patterns_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(params): Query<ListPatternsQueryParams>,
) -> Result<Json<ListPatternsResponse>, DbError> {
    let query = params.to_pattern_query();
    let response = LearningSystem::list_patterns(&state.storage, &db_name, &query)?;
    Ok(Json(response))
}

/// GET /_api/database/{db}/ai/learning/patterns/{id} - Get a specific pattern
pub async fn get_pattern_handler(
    State(state): State<AppState>,
    Path((db_name, pattern_id)): Path<(String, String)>,
) -> Result<Json<Pattern>, DbError> {
    let pattern = LearningSystem::get_pattern(&state.storage, &db_name, &pattern_id)?;
    Ok(Json(pattern))
}

/// POST /_api/database/{db}/ai/learning/process - Process unprocessed feedback
///
/// Triggers pattern extraction from new feedback
pub async fn process_feedback_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
) -> Result<Json<ProcessingResult>, DbError> {
    let result = LearningSystem::process_feedback_batch(&state.storage, &db_name, 100)?;
    Ok(Json(result))
}

/// Request body for getting recommendations
#[derive(Debug, Deserialize)]
pub struct GetRecommendationsRequest {
    /// Task ID to get recommendations for
    pub task_id: String,
    /// Optional contribution status
    pub contribution_status: Option<String>,
}

/// POST /_api/database/{db}/ai/learning/recommendations - Get recommendations
///
/// Returns recommendations based on learned patterns
pub async fn get_recommendations_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(request): Json<GetRecommendationsRequest>,
) -> Result<Json<Vec<Recommendation>>, DbError> {
    use crate::ai::AITask;

    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_tasks")?;

    let doc = coll.get(&request.task_id)?;
    let task: AITask = serde_json::from_value(doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted task data: {}", e)))?;

    let contribution_status = if let Some(status_str) = request.contribution_status {
        use crate::ai::ContributionStatus;
        Some(match status_str.to_lowercase().as_str() {
            "submitted" => ContributionStatus::Submitted,
            "analyzing" => ContributionStatus::Analyzing,
            "generating" => ContributionStatus::Generating,
            "validating" => ContributionStatus::Validating,
            "review" => ContributionStatus::Review,
            "approved" => ContributionStatus::Approved,
            "rejected" => ContributionStatus::Rejected,
            "merged" => ContributionStatus::Merged,
            "cancelled" => ContributionStatus::Cancelled,
            _ => {
                return Err(DbError::BadRequest(format!(
                    "Invalid contribution status: {}",
                    status_str
                )));
            }
        })
    } else {
        None
    };

    let recommendations = LearningSystem::get_recommendations(
        &state.storage,
        &db_name,
        &task,
        contribution_status.as_ref(),
    )?;

    Ok(Json(recommendations))
}
