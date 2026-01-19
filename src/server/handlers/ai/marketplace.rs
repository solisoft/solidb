//! Marketplace handlers for AI agent marketplace
//!
//! Provides endpoints for discovering and ranking AI agents.

use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;

use crate::ai::{
    AgentDiscoveryQuery, AgentDiscoveryResponse, AgentMarketplace, AgentRankingsResponse,
    AgentReputation,
};
use crate::error::DbError;
use crate::server::handlers::AppState;

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
}

/// GET /_api/ai/marketplace/discover - Discover agents for a task
///
/// Returns agents sorted by relevance for the given task
pub async fn discover_agents_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(params): Query<MarketplaceDiscoverQuery>,
) -> Result<Json<AgentDiscoveryResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    let query = AgentDiscoveryQuery {
        required_capabilities: params
            .capabilities
            .map(|c| c.split(',').map(|s| s.trim().to_string()).collect()),
        agent_type: params
            .agent_type
            .and_then(|s| match s.to_lowercase().as_str() {
                "analyzer" => Some(crate::ai::AgentType::Analyzer),
                "coder" => Some(crate::ai::AgentType::Coder),
                "tester" => Some(crate::ai::AgentType::Tester),
                "reviewer" => Some(crate::ai::AgentType::Reviewer),
                "integrator" => Some(crate::ai::AgentType::Integrator),
                _ => None,
            }),
        min_trust_score: params.min_trust_score,
        task_type: params.task_type,
        limit: None,
        idle_only: None,
    };

    let agents = AgentMarketplace::discover_agents(&db, &query)?;
    let total = agents.len();

    Ok(Json(AgentDiscoveryResponse { agents, total }))
}

/// GET /_api/ai/marketplace/agents/:id - Get agent details
///
/// Returns full agent profile including reputation
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
    pub task_requirements: serde_json::Value,
}

/// POST /_api/ai/marketplace/select - Select best agent for a task
///
/// Uses marketplace intelligence to select the optimal agent
pub async fn select_agent_handler(
    State(_state): State<AppState>,
    Path(_db_name): Path<String>,
    Json(_request): Json<SelectAgentRequest>,
) -> Result<Json<serde_json::Value>, DbError> {
    // For now, return a placeholder response
    // In a full implementation, this would use marketplace ranking
    Ok(Json(serde_json::json!({
        "status": "success",
        "message": "Agent selection submitted for processing"
    })))
}

/// Request body for verifying agent capability
#[derive(Debug, Deserialize)]
pub struct VerifyCapabilityRequest {
    pub capability: String,
    pub evidence: Option<serde_json::Value>,
}

/// POST /_api/ai/marketplace/agents/:id/verify - Verify agent capability
///
/// Submits evidence for capability verification (for future use)
pub async fn verify_capability_handler(
    State(state): State<AppState>,
    Path((db_name, agent_id)): Path<(String, String)>,
    Json(_request): Json<VerifyCapabilityRequest>,
) -> Result<Json<serde_json::Value>, DbError> {
    let _db = state.storage.get_database(&db_name)?;

    // For now, just acknowledge the request
    // In a full implementation, this would trigger verification
    Ok(Json(serde_json::json!({
        "status": "pending",
        "message": "Capability verification submitted for review",
        "agent_id": agent_id
    })))
}

/// Query parameters for rankings
#[derive(Debug, Deserialize)]
pub struct RankingsQuery {
    pub limit: Option<usize>,
}

/// GET /_api/ai/marketplace/rankings - Get agent rankings
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
