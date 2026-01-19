//! AI Task handlers
//!
//! Provides endpoints for managing AI tasks.

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;

use crate::ai::{orchestrator::TaskOrchestrator, AITask, AITaskStatus, ListAITasksResponse};
use crate::error::DbError;
use crate::server::handlers::AppState;

/// Query parameters for listing AI tasks
#[derive(Debug, Deserialize)]
pub struct ListAITasksQuery {
    /// Filter by contribution ID
    pub contribution_id: Option<String>,
    /// Filter by status
    pub status: Option<String>,
    /// Limit results
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
}

/// GET /_api/ai/tasks - List AI tasks
///
/// Requires Read permission
pub async fn list_ai_tasks_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(query): Query<ListAITasksQuery>,
) -> Result<Json<ListAITasksResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Return empty if collection doesn't exist
    if db.get_collection("_ai_tasks").is_err() {
        return Ok(Json(ListAITasksResponse {
            tasks: Vec::new(),
            total: 0,
        }));
    }

    let coll = db.get_collection("_ai_tasks")?;
    let mut tasks = Vec::new();

    for doc in coll.scan(None) {
        let task: AITask = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted task data".to_string()))?;

        // Apply filters
        if let Some(ref contribution_id) = query.contribution_id {
            if task.contribution_id != *contribution_id {
                continue;
            }
        }

        if let Some(ref status_filter) = query.status {
            let status_str = task.status.to_string();
            if status_str != *status_filter {
                continue;
            }
        }

        tasks.push(task);
    }

    // Sort by priority descending, then by created_at ascending
    tasks.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.created_at.cmp(&b.created_at))
    });

    let total = tasks.len();

    // Apply pagination
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(100);
    let tasks: Vec<AITask> = tasks.into_iter().skip(offset).take(limit).collect();

    Ok(Json(ListAITasksResponse { tasks, total }))
}

/// GET /_api/ai/tasks/:id - Get a specific AI task
///
/// Requires Read permission
pub async fn get_ai_task_handler(
    State(state): State<AppState>,
    Path((db_name, task_id)): Path<(String, String)>,
) -> Result<Json<AITask>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_tasks")?;

    let doc = coll.get(&task_id)?;

    let task: AITask = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted task data".to_string()))?;

    Ok(Json(task))
}

/// Request body for claiming a task
#[derive(Debug, Deserialize)]
pub struct ClaimTaskRequest {
    pub agent_id: String,
}

/// POST /_api/ai/tasks/:id/claim - Claim a task for processing
///
/// Used by AI agents to claim pending tasks
pub async fn claim_task_handler(
    State(state): State<AppState>,
    Path((db_name, task_id)): Path<(String, String)>,
    Json(request): Json<ClaimTaskRequest>,
) -> Result<Json<AITask>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_tasks")?;

    let doc = coll.get(&task_id)?;
    let mut task: AITask = serde_json::from_value(doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted task data: {}", e)))?;

    if task.status != AITaskStatus::Pending {
        return Err(DbError::BadRequest(format!(
            "Task {} is not pending (current status: {})",
            task_id, task.status
        )));
    }

    task.status = AITaskStatus::Running;
    task.agent_id = Some(request.agent_id);
    task.started_at = Some(chrono::Utc::now());

    let doc_value = serde_json::to_value(&task)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&task_id, doc_value)?;

    Ok(Json(task))
}

/// Request body for completing a task
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum CompleteTaskRequest {
    /// Nested output format (for backward compatibility)
    Nested { output: serde_json::Value },
    /// Flat format with individual fields
    Flat {
        /// Summary of changes
        summary: Option<String>,
        /// Risk score from analysis
        risk_score: Option<f64>,
        /// Whether review is required
        requires_review: Option<bool>,
        /// Affected files (for analysis task)
        affected_files: Option<Vec<String>>,
        /// Validation passed
        passed: Option<bool>,
        /// Validation stages
        stages: Option<Vec<String>>,
        /// Validation errors
        errors: Option<Vec<serde_json::Value>>,
        /// Tests run count
        tests_run: Option<u32>,
        /// Tests passed count
        tests_passed: Option<u32>,
        /// Test failures
        test_failures: Option<serde_json::Value>,
        /// Files to generate (for code generation)
        files: Option<Vec<serde_json::Value>>,
    },
}

/// Response for task completion with orchestration info
#[derive(Debug, serde::Serialize)]
pub struct CompleteTaskResponse {
    pub task: AITask,
    pub next_stage: Option<String>,
    pub message: String,
}

/// POST /_api/ai/tasks/:id/complete - Mark a task as completed
///
/// Used by AI agents to report successful completion.
/// Automatically creates follow-up tasks based on the pipeline orchestration.
pub async fn complete_task_handler(
    State(state): State<AppState>,
    Path((db_name, task_id)): Path<(String, String)>,
    Json(request): Json<CompleteTaskRequest>,
) -> Result<Json<CompleteTaskResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let tasks_coll = db.get_collection("_ai_tasks")?;
    let contribs_coll = db.get_collection("_ai_contributions")?;

    let doc = tasks_coll.get(&task_id)?;
    let mut task: AITask = serde_json::from_value(doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted task data: {}", e)))?;

    if task.status != AITaskStatus::Running {
        return Err(DbError::BadRequest(format!(
            "Task {} is not in progress (current status: {})",
            task_id, task.status
        )));
    }

    // Build output JSON based on task type - handle both formats
    let output = match &request {
        // Nested output format (for backward compatibility with tests)
        CompleteTaskRequest::Nested { output } => output.clone(),
        // Flat format with individual fields
        CompleteTaskRequest::Flat {
            summary,
            risk_score,
            requires_review,
            affected_files,
            passed,
            stages,
            errors,
            tests_run,
            tests_passed,
            test_failures,
            files,
        } => match task.task_type {
            crate::ai::AITaskType::AnalyzeContribution => {
                serde_json::json!({
                    "risk_score": risk_score.unwrap_or(0.5),
                    "requires_review": requires_review.unwrap_or(false),
                    "affected_files": affected_files.clone().unwrap_or_default()
                })
            }
            crate::ai::AITaskType::GenerateCode => {
                serde_json::json!({
                    "summary": summary.clone().unwrap_or_default(),
                    "files": files.clone().unwrap_or_default()
                })
            }
            crate::ai::AITaskType::ValidateCode => {
                serde_json::json!({
                    "passed": passed.unwrap_or(false),
                    "stages": stages.clone().unwrap_or_default(),
                    "errors": errors.clone().unwrap_or_default()
                })
            }
            crate::ai::AITaskType::RunTests => {
                serde_json::json!({
                    "passed": passed.unwrap_or(false),
                    "tests_run": tests_run.unwrap_or(0),
                    "tests_passed": tests_passed.unwrap_or(0),
                    "failures": test_failures.clone()
                })
            }
            crate::ai::AITaskType::PrepareReview | crate::ai::AITaskType::MergeChanges => {
                serde_json::json!({})
            }
        },
    };

    task.status = AITaskStatus::Completed;
    task.completed_at = Some(chrono::Utc::now());
    task.output = Some(output.clone());

    // Update task
    let doc_value = serde_json::to_value(&task)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    tasks_coll.update(&task_id, doc_value)?;

    // Get the contribution
    let contrib_doc = contribs_coll.get(&task.contribution_id)?;
    let mut contribution: crate::ai::Contribution = serde_json::from_value(contrib_doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted contribution data: {}", e)))?;

    // For analysis tasks, sync risk score and affected files to contribution
    if matches!(task.task_type, crate::ai::AITaskType::AnalyzeContribution) {
        if let Some(risk_score) = output.get("risk_score").and_then(|v| v.as_f64()) {
            contribution.risk_score = Some(risk_score);
        }
        if let Some(affected) = output.get("affected_files").and_then(|v| v.as_array()) {
            contribution.affected_files = affected
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        }
    }

    // Run orchestration to determine next steps
    let orchestration_result =
        TaskOrchestrator::on_task_complete(&task, &contribution, Some(&output));

    // Create next tasks
    let mut next_stage = None;
    for next_task in &orchestration_result.next_tasks {
        let task_value = serde_json::to_value(next_task)
            .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
        tasks_coll.insert(task_value)?;
        next_stage = Some(next_task.task_type.to_string());
    }

    // Update contribution status if needed
    if let Some(new_status) = orchestration_result.contribution_status {
        contribution.status = new_status;
        contribution.updated_at = chrono::Utc::now();
        let contrib_value = serde_json::to_value(&contribution)
            .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
        contribs_coll.update(&contribution.id, contrib_value)?;
    }

    Ok(Json(CompleteTaskResponse {
        task,
        next_stage,
        message: orchestration_result.message,
    }))
}

/// Request body for failing a task
#[derive(Debug, Deserialize)]
pub struct FailTaskRequest {
    pub error: String,
}

/// POST /_api/ai/tasks/:id/fail - Mark a task as failed
///
/// Used by AI agents to report task failure
pub async fn fail_task_handler(
    State(state): State<AppState>,
    Path((db_name, task_id)): Path<(String, String)>,
    Json(request): Json<FailTaskRequest>,
) -> Result<Json<AITask>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_tasks")?;

    let doc = coll.get(&task_id)?;
    let mut task: AITask = serde_json::from_value(doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted task data: {}", e)))?;

    if task.status != AITaskStatus::Running {
        return Err(DbError::BadRequest(format!(
            "Task {} is not in progress (current status: {})",
            task_id, task.status
        )));
    }

    task.status = AITaskStatus::Failed;
    task.completed_at = Some(chrono::Utc::now());
    task.fail(request.error);

    // Update in collection
    let doc_value = serde_json::to_value(&task)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&task_id, doc_value)?;

    Ok(Json(task))
}
