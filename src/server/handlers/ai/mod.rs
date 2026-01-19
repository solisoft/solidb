//! AI handler modules
//!
//! Provides handlers for AI contributions, tasks, agents, validation,
//! marketplace, learning, and recovery.

pub mod agents;
pub mod contributions;
pub mod learning;
pub mod marketplace;
pub mod recovery;
pub mod tasks;
pub mod validation;

// Re-export handlers for convenient access
pub use agents::{
    agent_heartbeat_handler, get_agent_handler, list_agents_handler, register_agent_handler,
    unregister_agent_handler, update_agent_handler,
};
pub use contributions::{
    approve_contribution_handler, cancel_contribution_handler, get_contribution_handler,
    list_contributions_handler, reject_contribution_handler, submit_contribution_handler,
};
pub use learning::{
    get_feedback_handler, get_recommendations_handler, list_feedback_handler,
    list_patterns_handler, process_feedback_handler,
};
pub use marketplace::{
    discover_agents_handler, get_agent_rankings_handler, get_agent_reputation_handler,
    select_agent_handler, verify_capability_handler,
};
pub use recovery::{
    get_recovery_status_handler, list_recovery_events_handler, reset_circuit_breaker_handler,
    retry_task_handler,
};
pub use tasks::{
    claim_task_handler, complete_task_handler, fail_task_handler, get_ai_task_handler,
    list_ai_tasks_handler,
};
pub use validation::{run_quick_validation_handler, run_validation_handler};

// Backwards compatibility aliases for old handler names (with ai_ prefix)
pub use agents::{
    agent_heartbeat_handler as ai_agent_heartbeat_handler,
    get_agent_handler as ai_get_agent_handler, list_agents_handler as ai_list_agents_handler,
    register_agent_handler as ai_register_agent_handler,
    unregister_agent_handler as ai_unregister_agent_handler,
    update_agent_handler as ai_update_agent_handler,
};
pub use contributions::{
    approve_contribution_handler as ai_approve_contribution_handler,
    cancel_contribution_handler as ai_cancel_contribution_handler,
    get_contribution_handler as ai_get_contribution_handler,
    list_contributions_handler as ai_list_contributions_handler,
    reject_contribution_handler as ai_reject_contribution_handler,
    submit_contribution_handler as ai_submit_contribution_handler,
};
pub use learning::{
    get_feedback_handler as ai_get_feedback_handler, get_pattern_handler as ai_get_pattern_handler,
    get_recommendations_handler as ai_get_recommendations_handler,
    list_feedback_handler as ai_list_feedback_handler,
    list_patterns_handler as ai_list_patterns_handler,
    process_feedback_handler as ai_process_feedback_handler,
};
pub use marketplace::{
    discover_agents_handler as ai_discover_agents_handler,
    get_agent_rankings_handler as ai_get_agent_rankings_handler,
    get_agent_reputation_handler as ai_get_agent_reputation_handler,
    select_agent_handler as ai_select_agent_handler,
    verify_capability_handler as ai_verify_capability_handler,
};
pub use recovery::{
    get_recovery_status_handler as ai_get_recovery_status_handler,
    list_recovery_events_handler as ai_list_recovery_events_handler,
    reset_circuit_breaker_handler as ai_reset_circuit_breaker_handler,
    retry_task_handler as ai_retry_task_handler,
};
pub use tasks::{
    claim_task_handler as claim_ai_task_handler, complete_task_handler as complete_ai_task_handler,
    fail_task_handler as fail_ai_task_handler, get_ai_task_handler as ai_get_ai_task_handler,
    list_ai_tasks_handler as ai_list_ai_tasks_handler,
};

use axum::{extract::Path, extract::State, response::Json, Extension};
use serde::Deserialize;

use crate::error::DbError;
use crate::server::auth::Claims;
use crate::server::handlers::AppState;

/// Request for generic content generation
#[derive(Debug, Deserialize)]
pub struct GenerateContentRequest {
    /// The user prompt
    pub prompt: String,
    /// Optional system prompt
    pub system: Option<String>,
    /// LLM provider to use
    pub provider: Option<String>,
}

/// Response for content generation
#[derive(Debug, serde::Serialize)]
pub struct GenerateContentResponse {
    pub content: String,
    pub provider: String,
}

/// POST /_api/database/{db}/ai/generate - Generate content using LLM
///
/// Simple endpoint for generic content generation without any domain-specific prompts.
/// Uses the same _env credentials as NL queries.
pub async fn generate_content_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Extension(_claims): Extension<Claims>,
    Json(request): Json<GenerateContentRequest>,
) -> Result<Json<GenerateContentResponse>, DbError> {
    use crate::server::llm_client::{LLMClient, Message};

    let client = LLMClient::from_storage(&state.storage, &db_name, request.provider.as_deref())?;
    let provider_name = format!("{:?}", client.provider()).to_lowercase();

    let mut messages = Vec::new();

    // Add system message if provided
    if let Some(system) = &request.system {
        messages.push(Message::system(system));
    }

    // Add user prompt
    messages.push(Message::user(&request.prompt));

    let content = client.chat(messages).await?;

    Ok(Json(GenerateContentResponse {
        content,
        provider: provider_name,
    }))
}

#[cfg(test)]
mod tests {
    use crate::ai::{ContributionType, Priority, SubmitContributionRequest};

    #[test]
    fn test_submit_contribution_request_parse() {
        let json = r#"{
            "contribution_type": "feature",
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
