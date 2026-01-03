//! HTTP handlers for AI contribution endpoints
//!
//! Provides endpoints for submitting, listing, and managing AI contributions.

use axum::{
    extract::{Path, Query, State},
    response::Json,
    Extension,
};
use serde::Deserialize;

use super::handlers::AppState;
use super::auth::Claims;
use crate::ai::{
    AITask, AITaskStatus, Agent, AgentStatus, AgentType, Contribution, ContributionStatus,
    ListAgentsResponse, ListAITasksResponse, ListContributionsResponse, Priority,
    ReviewContributionRequest, SubmitContributionRequest, SubmitContributionResponse,
};
use crate::error::DbError;

/// Query parameters for listing contributions
#[derive(Debug, Deserialize)]
pub struct ListContributionsQuery {
    /// Filter by status
    pub status: Option<String>,
    /// Filter by requester
    pub requester: Option<String>,
    /// Limit results
    pub limit: Option<usize>,
    /// Offset for pagination
    pub offset: Option<usize>,
}

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

/// POST /_api/ai/contributions - Submit a new contribution
///
/// Requires Write permission (any authenticated user with write access)
pub async fn submit_contribution_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Extension(claims): Extension<Claims>,
    Json(request): Json<SubmitContributionRequest>,
) -> Result<Json<SubmitContributionResponse>, DbError> {
    let username = claims.sub;
    let db = state.storage.get_database(&db_name)?;

    // Ensure _ai_contributions collection exists
    if db.get_collection("_ai_contributions").is_err() {
        db.create_collection("_ai_contributions".to_string(), None)?;
    }

    // Ensure _ai_tasks collection exists
    if db.get_collection("_ai_tasks").is_err() {
        db.create_collection("_ai_tasks".to_string(), None)?;
    }

    // Determine priority from context
    let priority = request
        .context
        .as_ref()
        .map(|c| match c.priority {
            Priority::Critical => 100,
            Priority::High => 75,
            Priority::Medium => 50,
            Priority::Low => 25,
        })
        .unwrap_or(50);

    // Create the contribution
    let contribution = Contribution::new(
        request.contribution_type,
        request.description,
        username,
        request.context,
    );

    let contribution_id = contribution.id.clone();

    // Store contribution in collection
    let coll = db.get_collection("_ai_contributions")?;
    let doc_value = serde_json::to_value(&contribution)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.insert(doc_value)?;

    // Create initial AI analysis task
    let task = AITask::analyze(contribution_id.clone(), priority);
    let tasks_coll = db.get_collection("_ai_tasks")?;
    let task_value = serde_json::to_value(&task)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    tasks_coll.insert(task_value)?;

    Ok(Json(SubmitContributionResponse {
        status: "success".to_string(),
        id: contribution_id.clone(),
        message: format!(
            "Contribution {} submitted successfully. AI analysis task queued.",
            contribution_id
        ),
    }))
}

/// GET /_api/ai/contributions - List all contributions
///
/// Requires Read permission
pub async fn list_contributions_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(query): Query<ListContributionsQuery>,
) -> Result<Json<ListContributionsResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Return empty if collection doesn't exist
    if db.get_collection("_ai_contributions").is_err() {
        return Ok(Json(ListContributionsResponse {
            contributions: Vec::new(),
            total: 0,
        }));
    }

    let coll = db.get_collection("_ai_contributions")?;
    let mut contributions = Vec::new();

    for doc in coll.scan(None) {
        let contribution: Contribution = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted contribution data".to_string()))?;

        // Apply filters
        if let Some(ref status_filter) = query.status {
            let status_str = contribution.status.to_string();
            if status_str != *status_filter {
                continue;
            }
        }

        if let Some(ref requester_filter) = query.requester {
            if contribution.requester != *requester_filter {
                continue;
            }
        }

        contributions.push(contribution);
    }

    // Sort by created_at descending (newest first)
    contributions.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let total = contributions.len();

    // Apply pagination
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(100);
    let contributions: Vec<Contribution> = contributions.into_iter().skip(offset).take(limit).collect();

    Ok(Json(ListContributionsResponse {
        contributions,
        total,
    }))
}

/// GET /_api/ai/contributions/:id - Get a specific contribution
///
/// Requires Read permission
pub async fn get_contribution_handler(
    State(state): State<AppState>,
    Path((db_name, contribution_id)): Path<(String, String)>,
) -> Result<Json<Contribution>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_contributions")?;

    let doc = coll.get(&contribution_id)?;

    let contribution: Contribution = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted contribution data".to_string()))?;

    Ok(Json(contribution))
}

/// POST /_api/ai/contributions/:id/approve - Approve a contribution
///
/// Requires Admin permission. Automatically creates a merge task.
pub async fn approve_contribution_handler(
    State(state): State<AppState>,
    Path((db_name, contribution_id)): Path<(String, String)>,
    Json(request): Json<ReviewContributionRequest>,
) -> Result<Json<Contribution>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_contributions")?;

    let doc = coll.get(&contribution_id)?;

    let mut contribution: Contribution = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted contribution data".to_string()))?;

    // Only contributions in Review status can be approved
    if contribution.status != ContributionStatus::Review {
        return Err(DbError::BadRequest(format!(
            "Cannot approve contribution in {} status. Must be in 'review' status.",
            contribution.status
        )));
    }

    contribution.set_status(ContributionStatus::Approved);
    if let Some(feedback) = request.feedback {
        contribution.feedback = Some(feedback);
    }

    // Update contribution in collection
    let doc_value = serde_json::to_value(&contribution)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&contribution_id, doc_value)?;

    // Use orchestrator to create merge task
    let orchestration = crate::ai::TaskOrchestrator::on_approval(&contribution, 50);

    // Ensure _ai_tasks collection exists
    if db.get_collection("_ai_tasks").is_err() {
        db.create_collection("_ai_tasks".to_string(), None)?;
    }

    // Create the merge task
    let tasks_coll = db.get_collection("_ai_tasks")?;
    for next_task in &orchestration.next_tasks {
        let task_value = serde_json::to_value(next_task)
            .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
        tasks_coll.insert(task_value)?;
    }

    Ok(Json(contribution))
}

/// POST /_api/ai/contributions/:id/reject - Reject a contribution
///
/// Requires Admin permission
pub async fn reject_contribution_handler(
    State(state): State<AppState>,
    Path((db_name, contribution_id)): Path<(String, String)>,
    Json(request): Json<ReviewContributionRequest>,
) -> Result<Json<Contribution>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_contributions")?;

    let doc = coll.get(&contribution_id)?;

    let mut contribution: Contribution = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted contribution data".to_string()))?;

    // Can reject from most statuses (not already rejected/cancelled/merged)
    match contribution.status {
        ContributionStatus::Rejected | ContributionStatus::Cancelled | ContributionStatus::Merged => {
            return Err(DbError::BadRequest(format!(
                "Cannot reject contribution in {} status",
                contribution.status
            )));
        }
        _ => {}
    }

    contribution.set_status(ContributionStatus::Rejected);
    contribution.feedback = request.feedback;

    // Update in collection
    let doc_value = serde_json::to_value(&contribution)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&contribution_id, doc_value)?;

    Ok(Json(contribution))
}

/// POST /_api/ai/contributions/:id/cancel - Cancel a contribution (by requester)
///
/// Requires Write permission (only the requester can cancel)
pub async fn cancel_contribution_handler(
    State(state): State<AppState>,
    Path((db_name, contribution_id)): Path<(String, String)>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<Contribution>, DbError> {
    let username = &claims.sub;
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_contributions")?;

    let doc = coll.get(&contribution_id)?;

    let mut contribution: Contribution = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted contribution data".to_string()))?;

    // Only the requester can cancel their own contribution
    if contribution.requester != *username {
        return Err(DbError::Forbidden(
            "Only the requester can cancel a contribution".to_string(),
        ));
    }

    // Cannot cancel already completed contributions
    match contribution.status {
        ContributionStatus::Merged | ContributionStatus::Rejected | ContributionStatus::Cancelled => {
            return Err(DbError::BadRequest(format!(
                "Cannot cancel contribution in {} status",
                contribution.status
            )));
        }
        _ => {}
    }

    contribution.set_status(ContributionStatus::Cancelled);

    // Update in collection
    let doc_value = serde_json::to_value(&contribution)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&contribution_id, doc_value)?;

    Ok(Json(contribution))
}

// ============================================================================
// AI Task Handlers
// ============================================================================

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
pub async fn claim_ai_task_handler(
    State(state): State<AppState>,
    Path((db_name, task_id)): Path<(String, String)>,
    Json(request): Json<ClaimTaskRequest>,
) -> Result<Json<AITask>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_tasks")?;

    let doc = coll.get(&task_id)?;

    let mut task: AITask = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted task data".to_string()))?;

    // Can only claim pending tasks
    if task.status != AITaskStatus::Pending {
        return Err(DbError::BadRequest(format!(
            "Cannot claim task in {} status. Must be 'pending'.",
            task.status
        )));
    }

    task.start(request.agent_id);

    // Update contribution status to Analyzing
    if let Ok(contrib_coll) = db.get_collection("_ai_contributions") {
        if let Ok(contrib_doc) = contrib_coll.get(&task.contribution_id) {
            if let Ok(mut contribution) =
                serde_json::from_value::<Contribution>(contrib_doc.to_value())
            {
                if contribution.status == ContributionStatus::Submitted {
                    contribution.set_status(ContributionStatus::Analyzing);
                    if let Ok(contrib_value) = serde_json::to_value(&contribution) {
                        let _ = contrib_coll.update(&task.contribution_id, contrib_value);
                    }
                }
            }
        }
    }

    // Update in collection
    let doc_value = serde_json::to_value(&task)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&task_id, doc_value)?;

    Ok(Json(task))
}

/// Request body for completing a task
#[derive(Debug, Deserialize)]
pub struct CompleteTaskRequest {
    #[serde(default)]
    pub output: Option<serde_json::Value>,
}

use crate::ai::TaskOrchestrator;

/// Response for task completion with orchestration info
#[derive(Debug, serde::Serialize)]
pub struct CompleteTaskResponse {
    pub task: AITask,
    pub orchestration: crate::ai::OrchestrationResult,
}

/// POST /_api/ai/tasks/:id/complete - Mark a task as completed
///
/// Used by AI agents to report successful completion.
/// Automatically creates follow-up tasks based on the pipeline orchestration.
pub async fn complete_ai_task_handler(
    State(state): State<AppState>,
    Path((db_name, task_id)): Path<(String, String)>,
    Json(request): Json<CompleteTaskRequest>,
) -> Result<Json<CompleteTaskResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let tasks_coll = db.get_collection("_ai_tasks")?;

    let doc = tasks_coll.get(&task_id)?;

    let mut task: AITask = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted task data".to_string()))?;

    // Can only complete running tasks
    if task.status != AITaskStatus::Running {
        return Err(DbError::BadRequest(format!(
            "Cannot complete task in {} status. Must be 'running'.",
            task.status
        )));
    }

    // Get the contribution for orchestration
    let contrib_coll = db.get_collection("_ai_contributions")?;
    let contrib_doc = contrib_coll.get(&task.contribution_id)?;
    let mut contribution: Contribution = serde_json::from_value(contrib_doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted contribution data".to_string()))?;

    // Mark task as complete
    task.complete(request.output.clone());

    // Run orchestration to determine next steps
    let orchestration = TaskOrchestrator::on_task_complete(
        &task,
        &contribution,
        request.output.as_ref(),
    );

    // Update contribution status if specified
    if let Some(new_status) = orchestration.contribution_status {
        contribution.set_status(new_status);
        let contrib_value = serde_json::to_value(&contribution)
            .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
        contrib_coll.update(&task.contribution_id, contrib_value)?;
    }

    // Create follow-up tasks
    for next_task in &orchestration.next_tasks {
        let task_value = serde_json::to_value(next_task)
            .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
        tasks_coll.insert(task_value)?;
    }

    // Update the completed task
    let doc_value = serde_json::to_value(&task)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    tasks_coll.update(&task_id, doc_value)?;

    Ok(Json(CompleteTaskResponse {
        task,
        orchestration,
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
pub async fn fail_ai_task_handler(
    State(state): State<AppState>,
    Path((db_name, task_id)): Path<(String, String)>,
    Json(request): Json<FailTaskRequest>,
) -> Result<Json<AITask>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_tasks")?;

    let doc = coll.get(&task_id)?;

    let mut task: AITask = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted task data".to_string()))?;

    // Can only fail running tasks
    if task.status != AITaskStatus::Running {
        return Err(DbError::BadRequest(format!(
            "Cannot fail task in {} status. Must be 'running'.",
            task.status
        )));
    }

    task.fail(request.error);

    // Update in collection
    let doc_value = serde_json::to_value(&task)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&task_id, doc_value)?;

    Ok(Json(task))
}

// ============================================================================
// AI Agent Handlers
// ============================================================================

/// Query parameters for listing agents
#[derive(Debug, Deserialize)]
pub struct ListAgentsQuery {
    /// Filter by agent type
    pub agent_type: Option<String>,
    /// Filter by status
    pub status: Option<String>,
}

/// Request body for registering an agent
#[derive(Debug, Deserialize)]
pub struct RegisterAgentRequest {
    pub name: String,
    pub agent_type: AgentType,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
}

/// GET /_api/ai/agents - List registered agents
///
/// Requires Read permission
pub async fn list_agents_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(query): Query<ListAgentsQuery>,
) -> Result<Json<ListAgentsResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Return empty if collection doesn't exist
    if db.get_collection("_ai_agents").is_err() {
        return Ok(Json(ListAgentsResponse {
            agents: Vec::new(),
            total: 0,
        }));
    }

    let coll = db.get_collection("_ai_agents")?;
    let mut agents = Vec::new();

    for doc in coll.scan(None) {
        let agent: Agent = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted agent data".to_string()))?;

        // Apply filters
        if let Some(ref type_filter) = query.agent_type {
            let type_str = agent.agent_type.to_string();
            if type_str != *type_filter {
                continue;
            }
        }

        if let Some(ref status_filter) = query.status {
            let status_str = match agent.status {
                AgentStatus::Idle => "idle",
                AgentStatus::Busy => "busy",
                AgentStatus::Offline => "offline",
                AgentStatus::Error => "error",
            };
            if status_str != *status_filter {
                continue;
            }
        }

        agents.push(agent);
    }

    // Sort by name
    agents.sort_by(|a, b| a.name.cmp(&b.name));

    let total = agents.len();

    Ok(Json(ListAgentsResponse { agents, total }))
}

/// POST /_api/ai/agents - Register a new agent
///
/// Requires Admin permission
pub async fn register_agent_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(request): Json<RegisterAgentRequest>,
) -> Result<Json<Agent>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Ensure _ai_agents collection exists
    if db.get_collection("_ai_agents").is_err() {
        db.create_collection("_ai_agents".to_string(), None)?;
    }

    let mut agent = Agent::new(request.name, request.agent_type, request.capabilities);
    agent.config = request.config;

    let coll = db.get_collection("_ai_agents")?;
    let doc_value = serde_json::to_value(&agent)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.insert(doc_value)?;

    Ok(Json(agent))
}

/// GET /_api/ai/agents/:id - Get agent details
///
/// Requires Read permission
pub async fn get_agent_handler(
    State(state): State<AppState>,
    Path((db_name, agent_id)): Path<(String, String)>,
) -> Result<Json<Agent>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_agents")?;

    let doc = coll.get(&agent_id)?;

    let agent: Agent = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted agent data".to_string()))?;

    Ok(Json(agent))
}

/// POST /_api/ai/agents/:id/heartbeat - Update agent heartbeat
///
/// Used by agents to report they are still alive
pub async fn agent_heartbeat_handler(
    State(state): State<AppState>,
    Path((db_name, agent_id)): Path<(String, String)>,
) -> Result<Json<Agent>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_agents")?;

    let doc = coll.get(&agent_id)?;

    let mut agent: Agent = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted agent data".to_string()))?;

    agent.heartbeat();

    let doc_value = serde_json::to_value(&agent)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&agent_id, doc_value)?;

    Ok(Json(agent))
}

/// DELETE /_api/ai/agents/:id - Unregister an agent
///
/// Requires Admin permission
pub async fn unregister_agent_handler(
    State(state): State<AppState>,
    Path((db_name, agent_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_agents")?;

    // Verify agent exists
    let doc = coll.get(&agent_id)?;
    let agent: Agent = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted agent data".to_string()))?;

    // Don't allow unregistering busy agents
    if agent.status == AgentStatus::Busy {
        return Err(DbError::BadRequest(
            "Cannot unregister a busy agent. Wait for current task to complete.".to_string(),
        ));
    }

    coll.delete(&agent_id)?;

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": format!("Agent {} unregistered", agent_id)
    })))
}

// ============================================================================
// Validation Pipeline Handlers
// ============================================================================

use crate::ai::{ValidationConfig, ValidationPipeline, ValidationResult};

/// Request body for running validation
#[derive(Debug, Deserialize)]
pub struct RunValidationRequest {
    /// Project root path (defaults to current directory)
    #[serde(default)]
    pub project_root: Option<String>,
    /// Run tests (defaults to true)
    #[serde(default = "default_true")]
    pub run_tests: bool,
    /// Run clippy (defaults to true)
    #[serde(default = "default_true")]
    pub run_clippy: bool,
    /// Run rustfmt check (defaults to true)
    #[serde(default = "default_true")]
    pub run_rustfmt: bool,
    /// Quick mode - skip tests (defaults to false)
    #[serde(default)]
    pub quick: bool,
}

fn default_true() -> bool {
    true
}

/// POST /_api/ai/validate - Run validation pipeline
///
/// Runs cargo check, clippy, and tests on the project
pub async fn run_validation_handler(
    Json(request): Json<RunValidationRequest>,
) -> Result<Json<ValidationResult>, DbError> {
    let project_root = request
        .project_root
        .unwrap_or_else(|| ".".to_string());

    // Verify the path exists
    if !crate::ai::validation::path_exists(&project_root) {
        return Err(DbError::BadRequest(format!(
            "Project root does not exist: {}",
            project_root
        )));
    }

    let config = ValidationConfig {
        project_root,
        run_tests: request.run_tests && !request.quick,
        run_clippy: request.run_clippy,
        run_rustfmt: request.run_rustfmt,
        test_timeout_secs: 300,
        test_filter: None,
    };

    let pipeline = ValidationPipeline::new(config);

    let result = if request.quick {
        pipeline.run_quick()
    } else {
        pipeline.run()
    };

    Ok(Json(result))
}

/// GET /_api/ai/validate/quick - Run quick validation (no tests)
///
/// Runs only cargo check and rustfmt
pub async fn run_quick_validation_handler() -> Result<Json<ValidationResult>, DbError> {
    let pipeline = ValidationPipeline::for_project(".");
    let result = pipeline.run_quick();
    Ok(Json(result))
}

// ============================================================================
// Marketplace Handlers
// ============================================================================

use crate::ai::{
    AgentDiscoveryQuery, AgentDiscoveryResponse, AgentMarketplace, AgentRankingsResponse,
    AgentReputation, RankedAgent,
};

/// Query parameters for marketplace discovery
#[derive(Debug, Deserialize)]
pub struct MarketplaceDiscoverQuery {
    /// Required capabilities (comma-separated)
    pub capabilities: Option<String>,
    /// Agent type filter
    pub agent_type: Option<String>,
    /// Minimum trust score (0.0-1.0)
    pub min_trust_score: Option<f64>,
    /// Task type for affinity scoring
    pub task_type: Option<String>,
    /// Only return idle agents
    pub idle_only: Option<bool>,
    /// Maximum results
    pub limit: Option<usize>,
}

/// GET /_api/database/{db}/ai/marketplace/discover - Discover agents
///
/// Query agents based on capabilities, type, trust score, etc.
pub async fn discover_agents_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(params): Query<MarketplaceDiscoverQuery>,
) -> Result<Json<AgentDiscoveryResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Parse agent type if provided
    let agent_type = params.agent_type.as_ref().and_then(|t| {
        match t.to_lowercase().as_str() {
            "analyzer" => Some(crate::ai::AgentType::Analyzer),
            "coder" => Some(crate::ai::AgentType::Coder),
            "tester" => Some(crate::ai::AgentType::Tester),
            "reviewer" => Some(crate::ai::AgentType::Reviewer),
            "integrator" => Some(crate::ai::AgentType::Integrator),
            _ => None,
        }
    });

    // Parse capabilities
    let required_capabilities = params.capabilities.map(|c| {
        c.split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    });

    let query = AgentDiscoveryQuery {
        required_capabilities,
        agent_type,
        min_trust_score: params.min_trust_score,
        task_type: params.task_type,
        idle_only: params.idle_only,
        limit: params.limit,
    };

    let agents = AgentMarketplace::discover_agents(&db, &query)?;
    let total = agents.len();

    Ok(Json(AgentDiscoveryResponse { agents, total }))
}

/// GET /_api/database/{db}/ai/marketplace/agent/{id}/reputation - Get agent reputation
///
/// Returns detailed reputation metrics for an agent
pub async fn get_agent_reputation_handler(
    State(state): State<AppState>,
    Path((db_name, agent_id)): Path<(String, String)>,
) -> Result<Json<AgentReputation>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let reputation = AgentMarketplace::get_reputation(&db, &agent_id)?;
    Ok(Json(reputation))
}

/// Request body for selecting an agent
#[derive(Debug, Deserialize)]
pub struct SelectAgentRequest {
    /// Task ID to select agent for
    pub task_id: String,
}

/// POST /_api/database/{db}/ai/marketplace/select - Select best agent for task
///
/// Automatically selects the best available agent for a given task
pub async fn select_agent_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(request): Json<SelectAgentRequest>,
) -> Result<Json<Option<RankedAgent>>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Get the task
    let tasks_coll = db.get_collection("_ai_tasks")?;
    let task_doc = tasks_coll.get(&request.task_id)?;
    let task: crate::ai::AITask = serde_json::from_value(task_doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted task data".to_string()))?;

    let agent = AgentMarketplace::select_agent_for_task(&db, &task)?;
    Ok(Json(agent))
}

/// Query parameters for rankings
#[derive(Debug, Deserialize)]
pub struct RankingsQuery {
    pub limit: Option<usize>,
}

/// GET /_api/database/{db}/ai/marketplace/rankings - Get agent rankings
///
/// Returns agents sorted by trust score
pub async fn get_agent_rankings_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(params): Query<RankingsQuery>,
) -> Result<Json<AgentRankingsResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let rankings = AgentMarketplace::get_rankings(&db, params.limit)?;
    let total = rankings.len();

    Ok(Json(AgentRankingsResponse { rankings, total }))
}

// ============================================================================
// Learning System Handlers
// ============================================================================

use crate::ai::{
    FeedbackEvent, FeedbackOutcome, FeedbackQuery, FeedbackType, LearningSystem,
    ListFeedbackResponse, ListPatternsResponse, Pattern, PatternQuery, PatternType,
    ProcessingResult, Recommendation,
};

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
            outcome: self.outcome.as_ref().and_then(|o| {
                match o.to_lowercase().as_str() {
                    "positive" => Some(FeedbackOutcome::Positive),
                    "negative" => Some(FeedbackOutcome::Negative),
                    "neutral" => Some(FeedbackOutcome::Neutral),
                    _ => None,
                }
            }),
            contribution_id: self.contribution_id.clone(),
            agent_id: self.agent_id.clone(),
            processed: self.processed,
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
    let response = LearningSystem::list_feedback(&state.storage, &db_name, &query)?;
    Ok(Json(response))
}

/// GET /_api/database/{db}/ai/learning/feedback/{id} - Get a specific feedback event
pub async fn get_feedback_handler(
    State(state): State<AppState>,
    Path((db_name, feedback_id)): Path<(String, String)>,
) -> Result<Json<FeedbackEvent>, DbError> {
    let event = LearningSystem::get_feedback(&state.storage, &db_name, &feedback_id)?;
    Ok(Json(event))
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
                    "success_pattern" => Some(PatternType::SuccessPattern),
                    "anti_pattern" => Some(PatternType::AntiPattern),
                    "error_pattern" => Some(PatternType::ErrorPattern),
                    "escalation_pattern" => Some(PatternType::EscalationPattern),
                    _ => None,
                }
            }),
            task_type: self.task_type.as_ref().and_then(|t| {
                match t.to_lowercase().as_str() {
                    "analyze_contribution" => Some(crate::ai::AITaskType::AnalyzeContribution),
                    "generate_code" => Some(crate::ai::AITaskType::GenerateCode),
                    "validate_code" => Some(crate::ai::AITaskType::ValidateCode),
                    "run_tests" => Some(crate::ai::AITaskType::RunTests),
                    "prepare_review" => Some(crate::ai::AITaskType::PrepareReview),
                    "merge_changes" => Some(crate::ai::AITaskType::MergeChanges),
                    _ => None,
                }
            }),
            min_confidence: self.min_confidence,
            limit: self.limit,
        }
    }
}

/// GET /_api/database/{db}/ai/learning/patterns - List learned patterns
///
/// Returns patterns with optional filtering
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

/// Request body for processing feedback batch
#[derive(Debug, Deserialize)]
pub struct ProcessFeedbackRequest {
    /// Maximum number of feedback events to process
    #[serde(default = "default_batch_limit")]
    pub limit: usize,
}

fn default_batch_limit() -> usize {
    50
}

/// POST /_api/database/{db}/ai/learning/process - Process feedback batch
///
/// Triggers batch processing of unprocessed feedback to extract patterns
pub async fn process_feedback_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(request): Json<ProcessFeedbackRequest>,
) -> Result<Json<ProcessingResult>, DbError> {
    let result = LearningSystem::process_feedback_batch(&state.storage, &db_name, request.limit)?;
    Ok(Json(result))
}

/// Request body for getting recommendations
#[derive(Debug, Deserialize)]
pub struct GetRecommendationsRequest {
    /// Task ID to get recommendations for
    pub task_id: String,
}

/// GET /_api/database/{db}/ai/learning/recommendations - Get recommendations for a task
///
/// Returns pattern-based recommendations for a specific task
pub async fn get_recommendations_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(request): Query<GetRecommendationsRequest>,
) -> Result<Json<Vec<Recommendation>>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Get the task
    let tasks_coll = db.get_collection("_ai_tasks")?;
    let task_doc = tasks_coll.get(&request.task_id)?;
    let task: crate::ai::AITask = serde_json::from_value(task_doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted task data".to_string()))?;

    // Get the contribution status
    let contribution_status = if let Ok(contrib_coll) = db.get_collection("_ai_contributions") {
        if let Ok(contrib_doc) = contrib_coll.get(&task.contribution_id) {
            serde_json::from_value::<Contribution>(contrib_doc.to_value())
                .ok()
                .map(|c| c.status)
        } else {
            None
        }
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

// ============================================================================
// Recovery System Handlers
// ============================================================================

use crate::ai::{
    ListRecoveryEventsResponse, RecoveryConfig, RecoverySystemStatus, RecoveryWorker,
};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::{ContributionType, Priority};

    #[test]
    fn test_submit_request_deserialization() {
        let json = r#"{
            "type": "feature",
            "description": "Add dark mode support",
            "context": {
                "related_collections": ["users", "settings"],
                "priority": "high"
            }
        }"#;

        let request: SubmitContributionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(request.contribution_type, ContributionType::Feature);
        assert_eq!(request.description, "Add dark mode support");

        let context = request.context.unwrap();
        assert_eq!(context.related_collections, vec!["users", "settings"]);
        assert_eq!(context.priority, Priority::High);
    }
}
