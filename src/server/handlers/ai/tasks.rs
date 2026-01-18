//! AI Task handlers
//!
//! Provides endpoints for managing AI tasks.

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;

use crate::server::handlers::AppState;
use crate::ai::{AITask, AITaskStatus, GeneratedFile, ListAITasksResponse};
use crate::error::DbError;

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
            task_id,
            task.status
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
pub struct CompleteTaskRequest {
    /// Generated files
    pub files: Vec<GeneratedFile>,
    /// Summary of changes
    pub summary: String,
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
    let coll = db.get_collection("_ai_tasks")?;

    let doc = coll.get(&task_id)?;
    let mut task: AITask = serde_json::from_value(doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted task data: {}", e)))?;

    if task.status != AITaskStatus::Running {
        return Err(DbError::BadRequest(format!(
            "Task {} is not in progress (current status: {})",
            task_id,
            task.status
        )));
    }

    task.status = AITaskStatus::Completed;
    task.completed_at = Some(chrono::Utc::now());
    task.output = Some(serde_json::json!({
        "summary": request.summary,
        "files": request.files,
    }));

    let doc_value = serde_json::to_value(&task)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&task_id, doc_value)?;

    Ok(Json(CompleteTaskResponse {
        task,
        next_stage: None,
        message: "Task completed successfully".to_string(),
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
            task_id,
            task.status
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
