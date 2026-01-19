//! AI Agent handlers
//!
//! Provides endpoints for registering and managing AI agents.

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;

use crate::ai::{Agent, AgentStatus, AgentType, ListAgentsResponse};
use crate::error::DbError;
use crate::server::handlers::AppState;

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
    /// Agent type - defaults to Analyzer if not specified
    #[serde(default = "default_agent_type")]
    pub agent_type: AgentType,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub config: Option<serde_json::Value>,
    /// Webhook URL for task notifications
    pub url: Option<String>,
    /// AI model (for www2 compatibility)
    pub model: Option<String>,
    /// System prompt (for www2 compatibility)
    pub system_prompt: Option<String>,
    /// Managed agent fields
    #[serde(default)]
    pub managed: bool,
    pub llm_url: Option<String>,
    pub llm_key: Option<String>,
}

fn default_agent_type() -> AgentType {
    AgentType::Analyzer
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
/// Agents register themselves using this endpoint
pub async fn register_agent_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(request): Json<RegisterAgentRequest>,
) -> Result<Json<Agent>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Ensure collection exists
    if db.get_collection("_ai_agents").is_err() {
        db.create_collection("_ai_agents".to_string(), None)?;
    }

    let coll = db.get_collection("_ai_agents")?;

    let id = uuid::Uuid::new_v4().to_string();

    // Build config from request fields
    let mut config = serde_json::json!({});
    if let Some(cfg) = request.config {
        if let Some(cfg_obj) = cfg.as_object() {
            for (k, v) in cfg_obj {
                config[k] = v.clone();
            }
        }
    }
    // Add model to config if provided
    if let Some(model) = request.model {
        config["model"] = serde_json::Value::String(model);
    }
    if let Some(system_prompt) = request.system_prompt {
        config["system_prompt"] = serde_json::Value::String(system_prompt);
    }

    let agent = Agent {
        id: id.clone(),
        name: request.name,
        agent_type: request.agent_type,
        status: AgentStatus::Idle,
        url: request.url,
        capabilities: request.capabilities,
        config: if config.as_object().map(|o| o.is_empty()).unwrap_or(true) {
            None
        } else {
            Some(config)
        },
        registered_at: chrono::Utc::now(),
        last_heartbeat: None,
        current_task_id: None,
        tasks_completed: 0,
        tasks_failed: 0,
    };

    let doc_value = serde_json::to_value(&agent)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;

    coll.insert(doc_value)?;

    Ok(Json(agent))
}

/// GET /_api/ai/agents/:id - Get a specific agent
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

/// POST /_api/ai/agents/:id/heartbeat - Heartbeat from agent
///
/// Agents send periodic heartbeats to indicate they're still alive
pub async fn agent_heartbeat_handler(
    State(state): State<AppState>,
    Path((db_name, agent_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_agents")?;

    let doc = coll.get(&agent_id)?;
    let mut agent: Agent = serde_json::from_value(doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted agent data: {}", e)))?;

    agent.status = AgentStatus::Idle;
    agent.last_heartbeat = Some(chrono::Utc::now());

    let doc_value = serde_json::to_value(&agent)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&agent_id, doc_value)?;

    Ok(Json(serde_json::json!({
        "status": "ok",
        "message": "Heartbeat received"
    })))
}

/// DELETE /_api/ai/agents/:id - Unregister an agent
///
/// Agents unregister themselves using this endpoint
pub async fn unregister_agent_handler(
    State(state): State<AppState>,
    Path((db_name, agent_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_agents")?;

    coll.delete(&agent_id)?;

    Ok(Json(serde_json::json!({
        "status": "success",
        "message": format!("Agent {} unregistered", agent_id)
    })))
}

/// Request body for updating an agent
#[derive(Debug, Deserialize)]
pub struct UpdateAgentRequest {
    pub name: Option<String>,
    pub agent_type: Option<AgentType>,
    pub capabilities: Option<Vec<String>>,
    pub config: Option<serde_json::Value>,
    pub url: Option<String>,
    pub model: Option<String>,
    pub system_prompt: Option<String>,
}

/// PUT /_api/ai/agents/:id - Update an existing agent
///
/// Requires Admin permission
pub async fn update_agent_handler(
    State(state): State<AppState>,
    Path((db_name, agent_id)): Path<(String, String)>,
    Json(request): Json<UpdateAgentRequest>,
) -> Result<Json<Agent>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let coll = db.get_collection("_ai_agents")?;

    let doc = coll.get(&agent_id)?;
    let mut agent: Agent = serde_json::from_value(doc.to_value())
        .map_err(|e| DbError::InternalError(format!("Corrupted agent data: {}", e)))?;

    if let Some(name) = request.name {
        agent.name = name;
    }
    if let Some(agent_type) = request.agent_type {
        agent.agent_type = agent_type;
    }
    if let Some(capabilities) = request.capabilities {
        agent.capabilities = capabilities;
    }
    if let Some(url) = request.url {
        agent.url = Some(url);
    }

    // Update config if needed
    let mut config = agent.config.unwrap_or_else(|| serde_json::json!({}));

    // Merge provided config
    if let Some(new_config) = request.config {
        if let Some(obj) = config.as_object_mut() {
            if let Some(new_obj) = new_config.as_object() {
                for (k, v) in new_obj {
                    obj.insert(k.clone(), v.clone());
                }
            }
        } else {
            config = new_config;
        }
    }

    // Update model/system_prompt in config
    if let Some(model) = request.model {
        if let Some(obj) = config.as_object_mut() {
            obj.insert("model".to_string(), serde_json::Value::String(model));
        }
    }
    if let Some(system_prompt) = request.system_prompt {
        if let Some(obj) = config.as_object_mut() {
            obj.insert(
                "system_prompt".to_string(),
                serde_json::Value::String(system_prompt),
            );
        }
    }

    agent.config = if config.as_object().map(|o| o.is_empty()).unwrap_or(true) {
        None
    } else {
        Some(config)
    };

    // Save update
    let doc_value = serde_json::to_value(&agent)
        .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
    coll.update(&agent_id, doc_value)?;

    Ok(Json(agent))
}
