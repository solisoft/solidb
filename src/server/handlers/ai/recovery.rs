//! Recovery System handlers
//!
//! Provides endpoints for monitoring and managing the AI recovery system.

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;

use crate::server::handlers::AppState;
use crate::ai::{ListRecoveryEventsResponse, RecoveryConfig, RecoverySystemStatus, RecoveryWorker};
use crate::error::DbError;

/// GET /_api/database/{db}/ai/recovery/status - Get recovery system status
///
/// Returns current status of the recovery system
pub async fn get_recovery_status_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
) -> Result<Json<RecoverySystemStatus>, DbError> {
    let config = RecoveryConfig::default();
    let worker = RecoveryWorker::new(state.storage.clone(), db_name, config);
    let status = worker.get_status()?;
    Ok(Json(status))
}

/// Request body for retrying a task
#[derive(Debug, Deserialize)]
pub struct RetryTaskRequest {
    pub task_id: String,
}

/// POST /_api/database/{db}/ai/recovery/task/{id}/retry - Force retry a task
///
/// Manually triggers retry for a failed or stalled task
pub async fn retry_task_handler(
    State(state): State<AppState>,
    Path((db_name, task_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, DbError> {
    let config = RecoveryConfig::default();
    let worker = RecoveryWorker::new(state.storage.clone(), db_name, config);
    worker.force_retry_task(&task_id)?;
    Ok(Json(serde_json::json!({
        "status": "success",
        "message": format!("Task {} queued for retry", task_id)
    })))
}

/// POST /_api/database/{db}/ai/recovery/agent/{id}/reset - Reset circuit breaker
///
/// Manually resets the circuit breaker for an agent
pub async fn reset_circuit_breaker_handler(
    State(state): State<AppState>,
    Path((db_name, agent_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, DbError> {
    let config = RecoveryConfig::default();
    let worker = RecoveryWorker::new(state.storage.clone(), db_name, config);
    worker.reset_circuit_breaker(&agent_id)?;
    Ok(Json(serde_json::json!({
        "status": "success",
        "message": format!("Circuit breaker reset for agent {}", agent_id)
    })))
}

/// Query parameters for listing recovery events
#[derive(Debug, Deserialize)]
pub struct ListRecoveryEventsQuery {
    pub limit: Option<usize>,
}

/// GET /_api/database/{db}/ai/recovery/events - List recovery events
///
/// Returns recent recovery events
pub async fn list_recovery_events_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(params): Query<ListRecoveryEventsQuery>,
) -> Result<Json<ListRecoveryEventsResponse>, DbError> {
    let config = RecoveryConfig::default();
    let worker = RecoveryWorker::new(state.storage.clone(), db_name, config);
    let events = worker.list_events(params.limit)?;
    let total = events.len();
    Ok(Json(ListRecoveryEventsResponse { events, total }))
}
