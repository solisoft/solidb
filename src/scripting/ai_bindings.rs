//! Lua bindings for AI contribution system
//!
//! Provides `solidb.ai.*` functions for interacting with the AI pipeline from Lua scripts.
//! Includes bindings for:
//! - Core contribution system (`solidb.ai.*`)
//! - Agent Marketplace (`solidb.ai.marketplace.*`)
//! - Learning Loops (`solidb.ai.learning.*`)
//! - Autonomous Recovery (`solidb.ai.recovery.*`)

use mlua::{Lua, Result as LuaResult, Table, Value as LuaValue};
use serde_json::Value as JsonValue;
use std::sync::Arc;

use crate::ai::{
    AITask,
    AITaskStatus,
    AITaskType,
    Agent,
    // Marketplace
    AgentDiscoveryQuery,
    AgentMarketplace,
    AgentType,
    Contribution,
    ContributionType,
    // Learning
    FeedbackOutcome,
    FeedbackQuery,
    FeedbackType,
    LearningSystem,
    PatternQuery,
    PatternType,
    Priority,
    // Recovery
    RecoveryConfig,
    RecoveryWorker,
    TaskOrchestrator,
};
use crate::storage::StorageEngine;

/// Create the solidb.ai table with all AI-related functions
pub fn create_ai_table(lua: &Lua, storage: Arc<StorageEngine>, db_name: &str) -> LuaResult<Table> {
    let ai_table = lua.create_table()?;

    // solidb.ai.submit_contribution(type, description, context?) -> contribution_id
    let storage_submit = storage.clone();
    let db_submit = db_name.to_string();
    let submit_fn = lua.create_function(move |_lua, args: (String, String, Option<Table>)| {
        let (contrib_type_str, description, context_table) = args;

        let contrib_type = match contrib_type_str.to_lowercase().as_str() {
            "feature" => ContributionType::Feature,
            "bugfix" => ContributionType::Bugfix,
            "enhancement" => ContributionType::Enhancement,
            "documentation" => ContributionType::Documentation,
            _ => {
                return Err(mlua::Error::RuntimeError(format!(
                "Invalid contribution type: {}. Use: feature, bugfix, enhancement, documentation",
                contrib_type_str
            )))
            }
        };

        // Parse context if provided
        let context = if let Some(ctx) = context_table {
            let mut related: Vec<String> = Vec::new();
            if let Ok(rel) = ctx.get::<Table>("related_collections") {
                for pair in rel.pairs::<i64, String>() {
                    if let Ok((_, v)) = pair {
                        related.push(v);
                    }
                }
            }

            let priority = ctx
                .get::<String>("priority")
                .map(|p| match p.to_lowercase().as_str() {
                    "critical" => Priority::Critical,
                    "high" => Priority::High,
                    "low" => Priority::Low,
                    _ => Priority::Medium,
                })
                .unwrap_or(Priority::Medium);

            Some(crate::ai::ContributionContext {
                related_collections: related,
                priority,
                metadata: None,
            })
        } else {
            None
        };

        // Get database
        let db = storage_submit
            .get_database(&db_submit)
            .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

        // Ensure collections exist
        if db.get_collection("_ai_contributions").is_err() {
            db.create_collection("_ai_contributions".to_string(), None)
                .map_err(|e| {
                    mlua::Error::RuntimeError(format!("Failed to create collection: {}", e))
                })?;
        }
        if db.get_collection("_ai_tasks").is_err() {
            db.create_collection("_ai_tasks".to_string(), None)
                .map_err(|e| {
                    mlua::Error::RuntimeError(format!("Failed to create collection: {}", e))
                })?;
        }

        // Create contribution
        let contribution = Contribution::new(
            contrib_type,
            description,
            "_lua_script".to_string(),
            context.clone(),
        );
        let contribution_id = contribution.id.clone();

        // Store contribution
        let coll = db
            .get_collection("_ai_contributions")
            .map_err(|e| mlua::Error::RuntimeError(format!("Collection error: {}", e)))?;
        let doc_value = serde_json::to_value(&contribution)
            .map_err(|e| mlua::Error::RuntimeError(format!("Serialization error: {}", e)))?;
        coll.insert(doc_value)
            .map_err(|e| mlua::Error::RuntimeError(format!("Insert error: {}", e)))?;

        // Create initial analysis task
        let priority = context
            .as_ref()
            .map(|c| match c.priority {
                Priority::Critical => 100,
                Priority::High => 75,
                Priority::Medium => 50,
                Priority::Low => 25,
            })
            .unwrap_or(50);

        let task = AITask::analyze(contribution_id.clone(), priority);
        let tasks_coll = db
            .get_collection("_ai_tasks")
            .map_err(|e| mlua::Error::RuntimeError(format!("Collection error: {}", e)))?;
        let task_value = serde_json::to_value(&task)
            .map_err(|e| mlua::Error::RuntimeError(format!("Serialization error: {}", e)))?;
        tasks_coll
            .insert(task_value)
            .map_err(|e| mlua::Error::RuntimeError(format!("Insert error: {}", e)))?;

        Ok(contribution_id)
    })?;
    ai_table.set("submit_contribution", submit_fn)?;

    // solidb.ai.get_contribution(id) -> contribution table or nil
    let storage_get = storage.clone();
    let db_get = db_name.to_string();
    let get_contribution_fn = lua.create_function(move |lua, id: String| {
        let db = storage_get
            .get_database(&db_get)
            .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

        let coll = match db.get_collection("_ai_contributions") {
            Ok(c) => c,
            Err(_) => return Ok(LuaValue::Nil),
        };

        match coll.get(&id) {
            Ok(doc) => {
                let json_str = serde_json::to_string(&doc.to_value())
                    .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
                let lua_val: LuaValue = lua
                    .load(&format!("return {}", json_to_lua(&json_str)))
                    .eval()
                    .unwrap_or(LuaValue::Nil);
                Ok(lua_val)
            }
            Err(_) => Ok(LuaValue::Nil),
        }
    })?;
    ai_table.set("get_contribution", get_contribution_fn)?;

    // solidb.ai.list_contributions(options?) -> array of contributions
    let storage_list = storage.clone();
    let db_list = db_name.to_string();
    let list_contributions_fn = lua.create_function(move |lua, options: Option<Table>| {
        let db = storage_list
            .get_database(&db_list)
            .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

        let coll = match db.get_collection("_ai_contributions") {
            Ok(c) => c,
            Err(_) => {
                let result = lua.create_table()?;
                return Ok(result);
            }
        };

        let status_filter = options
            .as_ref()
            .and_then(|o| o.get::<String>("status").ok());

        let limit = options
            .as_ref()
            .and_then(|o| o.get::<usize>("limit").ok())
            .unwrap_or(100);

        let result = lua.create_table()?;
        let mut idx = 1;

        for doc in coll.scan(None) {
            if idx > limit {
                break;
            }

            let contribution: Contribution = match serde_json::from_value(doc.to_value()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Apply status filter
            if let Some(ref filter) = status_filter {
                if contribution.status.to_string() != *filter {
                    continue;
                }
            }

            let json_str = serde_json::to_string(&contribution)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let lua_val: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&json_str)))
                .eval()
                .unwrap_or(LuaValue::Nil);

            result.set(idx, lua_val)?;
            idx += 1;
        }

        Ok(result)
    })?;
    ai_table.set("list_contributions", list_contributions_fn)?;

    // solidb.ai.claim_task(task_id, agent_id) -> task table
    let storage_claim = storage.clone();
    let db_claim = db_name.to_string();
    let claim_task_fn =
        lua.create_function(move |lua, (task_id, agent_id): (String, String)| {
            let db = storage_claim
                .get_database(&db_claim)
                .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

            let coll = db
                .get_collection("_ai_tasks")
                .map_err(|e| mlua::Error::RuntimeError(format!("Collection error: {}", e)))?;

            let doc = coll
                .get(&task_id)
                .map_err(|e| mlua::Error::RuntimeError(format!("Task not found: {}", e)))?;

            let mut task: AITask = serde_json::from_value(doc.to_value())
                .map_err(|e| mlua::Error::RuntimeError(format!("Parse error: {}", e)))?;

            if task.status != AITaskStatus::Pending {
                return Err(mlua::Error::RuntimeError(format!(
                    "Cannot claim task in {} status",
                    task.status
                )));
            }

            task.start(agent_id);

            let doc_value = serde_json::to_value(&task)
                .map_err(|e| mlua::Error::RuntimeError(format!("Serialization error: {}", e)))?;
            coll.update(&task_id, doc_value)
                .map_err(|e| mlua::Error::RuntimeError(format!("Update error: {}", e)))?;

            let json_str = serde_json::to_string(&task)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let lua_val: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&json_str)))
                .eval()
                .unwrap_or(LuaValue::Nil);

            Ok(lua_val)
        })?;
    ai_table.set("claim_task", claim_task_fn)?;

    // solidb.ai.complete_task(task_id, output?) -> task table
    let storage_complete = storage.clone();
    let db_complete = db_name.to_string();
    let complete_task_fn =
        lua.create_function(move |lua, (task_id, output): (String, Option<Table>)| {
            let db = storage_complete
                .get_database(&db_complete)
                .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

            let tasks_coll = db
                .get_collection("_ai_tasks")
                .map_err(|e| mlua::Error::RuntimeError(format!("Collection error: {}", e)))?;

            let doc = tasks_coll
                .get(&task_id)
                .map_err(|e| mlua::Error::RuntimeError(format!("Task not found: {}", e)))?;

            let mut task: AITask = serde_json::from_value(doc.to_value())
                .map_err(|e| mlua::Error::RuntimeError(format!("Parse error: {}", e)))?;

            if task.status != AITaskStatus::Running {
                return Err(mlua::Error::RuntimeError(format!(
                    "Cannot complete task in {} status",
                    task.status
                )));
            }

            // Convert Lua table to JSON if provided
            let output_json: Option<JsonValue> =
                output.map(|t| lua_table_to_json(lua, t)).transpose()?;

            task.complete(output_json.clone());

            // Get contribution for orchestration
            let contrib_coll = db
                .get_collection("_ai_contributions")
                .map_err(|e| mlua::Error::RuntimeError(format!("Collection error: {}", e)))?;
            let contrib_doc = contrib_coll
                .get(&task.contribution_id)
                .map_err(|e| mlua::Error::RuntimeError(format!("Contribution not found: {}", e)))?;
            let mut contribution: Contribution = serde_json::from_value(contrib_doc.to_value())
                .map_err(|e| mlua::Error::RuntimeError(format!("Parse error: {}", e)))?;

            // Run orchestration
            let orchestration =
                TaskOrchestrator::on_task_complete(&task, &contribution, output_json.as_ref());

            // Update contribution status if specified
            if let Some(new_status) = orchestration.contribution_status {
                contribution.set_status(new_status);
                let contrib_value = serde_json::to_value(&contribution).map_err(|e| {
                    mlua::Error::RuntimeError(format!("Serialization error: {}", e))
                })?;
                contrib_coll
                    .update(&task.contribution_id, contrib_value)
                    .map_err(|e| mlua::Error::RuntimeError(format!("Update error: {}", e)))?;
            }

            // Create follow-up tasks
            for next_task in &orchestration.next_tasks {
                let task_value = serde_json::to_value(next_task).map_err(|e| {
                    mlua::Error::RuntimeError(format!("Serialization error: {}", e))
                })?;
                tasks_coll
                    .insert(task_value)
                    .map_err(|e| mlua::Error::RuntimeError(format!("Insert error: {}", e)))?;
            }

            // Update the completed task
            let doc_value = serde_json::to_value(&task)
                .map_err(|e| mlua::Error::RuntimeError(format!("Serialization error: {}", e)))?;
            tasks_coll
                .update(&task_id, doc_value)
                .map_err(|e| mlua::Error::RuntimeError(format!("Update error: {}", e)))?;

            // Return result with orchestration info
            let result = lua.create_table()?;
            let task_json = serde_json::to_string(&task)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let task_lua: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&task_json)))
                .eval()
                .unwrap_or(LuaValue::Nil);
            result.set("task", task_lua)?;
            result.set("message", orchestration.message)?;
            result.set("next_tasks_created", orchestration.next_tasks.len())?;

            Ok(result)
        })?;
    ai_table.set("complete_task", complete_task_fn)?;

    // solidb.ai.get_pending_tasks(options?) -> array of tasks
    let storage_pending = storage.clone();
    let db_pending = db_name.to_string();
    let get_pending_tasks_fn = lua.create_function(move |lua, options: Option<Table>| {
        let db = storage_pending
            .get_database(&db_pending)
            .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

        let coll = match db.get_collection("_ai_tasks") {
            Ok(c) => c,
            Err(_) => {
                let result = lua.create_table()?;
                return Ok(result);
            }
        };

        let task_type_filter = options
            .as_ref()
            .and_then(|o| o.get::<String>("task_type").ok());

        let limit = options
            .as_ref()
            .and_then(|o| o.get::<usize>("limit").ok())
            .unwrap_or(10);

        let result = lua.create_table()?;
        let mut idx = 1;
        let mut tasks: Vec<AITask> = Vec::new();

        for doc in coll.scan(None) {
            let task: AITask = match serde_json::from_value(doc.to_value()) {
                Ok(t) => t,
                Err(_) => continue,
            };

            if task.status != AITaskStatus::Pending {
                continue;
            }

            // Apply task type filter
            if let Some(ref filter) = task_type_filter {
                if task.task_type.to_string() != *filter {
                    continue;
                }
            }

            tasks.push(task);
        }

        // Sort by priority (descending) then created_at (ascending)
        tasks.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });

        for task in tasks.into_iter().take(limit) {
            let json_str = serde_json::to_string(&task)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let lua_val: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&json_str)))
                .eval()
                .unwrap_or(LuaValue::Nil);

            result.set(idx, lua_val)?;
            idx += 1;
        }

        Ok(result)
    })?;
    ai_table.set("get_pending_tasks", get_pending_tasks_fn)?;

    // solidb.ai.register_agent(name, agent_type, capabilities?) -> agent table
    let storage_reg = storage.clone();
    let db_reg = db_name.to_string();
    let register_agent_fn = lua.create_function(
        move |lua, (name, agent_type_str, capabilities): (String, String, Option<Table>)| {
            let agent_type = match agent_type_str.to_lowercase().as_str() {
                "analyzer" => AgentType::Analyzer,
                "coder" => AgentType::Coder,
                "tester" => AgentType::Tester,
                "reviewer" => AgentType::Reviewer,
                "integrator" => AgentType::Integrator,
                _ => {
                    return Err(mlua::Error::RuntimeError(format!(
                "Invalid agent type: {}. Use: analyzer, coder, tester, reviewer, integrator",
                agent_type_str
            )))
                }
            };

            let caps: Vec<String> = capabilities
                .map(|t| {
                    let mut v = Vec::new();
                    for pair in t.pairs::<i64, String>() {
                        if let Ok((_, cap)) = pair {
                            v.push(cap);
                        }
                    }
                    v
                })
                .unwrap_or_default();

            let db = storage_reg
                .get_database(&db_reg)
                .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

            if db.get_collection("_ai_agents").is_err() {
                db.create_collection("_ai_agents".to_string(), None)
                    .map_err(|e| {
                        mlua::Error::RuntimeError(format!("Failed to create collection: {}", e))
                    })?;
            }

            let agent = Agent::new(name, agent_type, caps);

            let coll = db
                .get_collection("_ai_agents")
                .map_err(|e| mlua::Error::RuntimeError(format!("Collection error: {}", e)))?;
            let doc_value = serde_json::to_value(&agent)
                .map_err(|e| mlua::Error::RuntimeError(format!("Serialization error: {}", e)))?;
            coll.insert(doc_value)
                .map_err(|e| mlua::Error::RuntimeError(format!("Insert error: {}", e)))?;

            let json_str = serde_json::to_string(&agent)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let lua_val: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&json_str)))
                .eval()
                .unwrap_or(LuaValue::Nil);

            Ok(lua_val)
        },
    )?;
    ai_table.set("register_agent", register_agent_fn)?;

    // ============================================
    // MARKETPLACE SUB-TABLE: solidb.ai.marketplace
    // ============================================
    let marketplace_table = create_marketplace_table(lua, storage.clone(), db_name)?;
    ai_table.set("marketplace", marketplace_table)?;

    // ============================================
    // LEARNING SUB-TABLE: solidb.ai.learning
    // ============================================
    let learning_table = create_learning_table(lua, storage.clone(), db_name)?;
    ai_table.set("learning", learning_table)?;

    // ============================================
    // RECOVERY SUB-TABLE: solidb.ai.recovery
    // ============================================
    let recovery_table = create_recovery_table(lua, storage.clone(), db_name)?;
    ai_table.set("recovery", recovery_table)?;

    Ok(ai_table)
}

/// Create the solidb.ai.marketplace table
fn create_marketplace_table(
    lua: &Lua,
    storage: Arc<StorageEngine>,
    db_name: &str,
) -> LuaResult<Table> {
    let marketplace_table = lua.create_table()?;

    // solidb.ai.marketplace.discover(options?) -> array of agents
    let storage_discover = storage.clone();
    let db_discover = db_name.to_string();
    let discover_fn = lua.create_function(move |lua, options: Option<Table>| {
        let db = storage_discover
            .get_database(&db_discover)
            .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

        let query = if let Some(opts) = options {
            AgentDiscoveryQuery {
                required_capabilities: opts.get::<Table>("required_capabilities").ok().map(|t| {
                    let mut caps = Vec::new();
                    for pair in t.pairs::<i64, String>() {
                        if let Ok((_, v)) = pair {
                            caps.push(v);
                        }
                    }
                    caps
                }),
                agent_type: opts.get::<String>("agent_type").ok().and_then(|s| {
                    match s.to_lowercase().as_str() {
                        "analyzer" => Some(AgentType::Analyzer),
                        "coder" => Some(AgentType::Coder),
                        "tester" => Some(AgentType::Tester),
                        "reviewer" => Some(AgentType::Reviewer),
                        "integrator" => Some(AgentType::Integrator),
                        _ => None,
                    }
                }),
                min_trust_score: opts.get::<f64>("min_trust_score").ok(),
                limit: opts.get::<usize>("limit").ok(),
                task_type: opts.get::<String>("task_type").ok(),
                idle_only: opts.get::<bool>("idle_only").ok(),
            }
        } else {
            AgentDiscoveryQuery::default()
        };

        let agents = AgentMarketplace::discover_agents(&db, &query)
            .map_err(|e| mlua::Error::RuntimeError(format!("Discovery error: {}", e)))?;

        let result = lua.create_table()?;
        for (idx, agent) in agents.iter().enumerate() {
            let json_str = serde_json::to_string(agent)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let lua_val: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&json_str)))
                .eval()
                .unwrap_or(LuaValue::Nil);
            result.set(idx + 1, lua_val)?;
        }

        Ok(result)
    })?;
    marketplace_table.set("discover", discover_fn)?;

    // solidb.ai.marketplace.get_reputation(agent_id) -> reputation table or nil
    let storage_rep = storage.clone();
    let db_rep = db_name.to_string();
    let get_reputation_fn = lua.create_function(move |lua, agent_id: String| {
        let db = storage_rep
            .get_database(&db_rep)
            .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

        match AgentMarketplace::get_reputation(&db, &agent_id) {
            Ok(reputation) => {
                let json_str = serde_json::to_string(&reputation)
                    .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
                let lua_val: LuaValue = lua
                    .load(&format!("return {}", json_to_lua(&json_str)))
                    .eval()
                    .unwrap_or(LuaValue::Nil);
                Ok(lua_val)
            }
            Err(_) => Ok(LuaValue::Nil),
        }
    })?;
    marketplace_table.set("get_reputation", get_reputation_fn)?;

    // solidb.ai.marketplace.select_for_task(task_id) -> selected agent or nil
    let storage_select = storage.clone();
    let db_select = db_name.to_string();
    let select_fn = lua.create_function(move |lua, task_id: String| {
        let db = storage_select
            .get_database(&db_select)
            .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

        // Get the task to determine what agent type is needed
        let tasks_coll = match db.get_collection("_ai_tasks") {
            Ok(c) => c,
            Err(_) => return Ok(LuaValue::Nil),
        };

        let doc = match tasks_coll.get(&task_id) {
            Ok(d) => d,
            Err(_) => return Ok(LuaValue::Nil),
        };

        let task: AITask = serde_json::from_value(doc.to_value())
            .map_err(|e| mlua::Error::RuntimeError(format!("Parse error: {}", e)))?;

        match AgentMarketplace::select_agent_for_task(&db, &task) {
            Ok(Some(agent)) => {
                let json_str = serde_json::to_string(&agent)
                    .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
                let lua_val: LuaValue = lua
                    .load(&format!("return {}", json_to_lua(&json_str)))
                    .eval()
                    .unwrap_or(LuaValue::Nil);
                Ok(lua_val)
            }
            Ok(None) | Err(_) => Ok(LuaValue::Nil),
        }
    })?;
    marketplace_table.set("select_for_task", select_fn)?;

    // solidb.ai.marketplace.get_rankings(limit?) -> array of agent rankings
    let storage_rank = storage.clone();
    let db_rank = db_name.to_string();
    let rankings_fn = lua.create_function(move |lua, limit: Option<usize>| {
        let db = storage_rank
            .get_database(&db_rank)
            .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

        let rankings = AgentMarketplace::get_rankings(&db, limit.or(Some(10)))
            .map_err(|e| mlua::Error::RuntimeError(format!("Rankings error: {}", e)))?;

        let result = lua.create_table()?;
        for (idx, ranking) in rankings.iter().enumerate() {
            let json_str = serde_json::to_string(ranking)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let lua_val: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&json_str)))
                .eval()
                .unwrap_or(LuaValue::Nil);
            result.set(idx + 1, lua_val)?;
        }

        Ok(result)
    })?;
    marketplace_table.set("get_rankings", rankings_fn)?;

    Ok(marketplace_table)
}

/// Create the solidb.ai.learning table
fn create_learning_table(
    lua: &Lua,
    storage: Arc<StorageEngine>,
    db_name: &str,
) -> LuaResult<Table> {
    let learning_table = lua.create_table()?;

    // solidb.ai.learning.list_feedback(options?) -> array of feedback events
    let storage_fb = storage.clone();
    let db_fb = db_name.to_string();
    let list_feedback_fn = lua.create_function(move |lua, options: Option<Table>| {
        let query = if let Some(opts) = options {
            FeedbackQuery {
                feedback_type: opts.get::<String>("feedback_type").ok().and_then(|s| {
                    match s.to_lowercase().as_str() {
                        "humanreview" | "human_review" => Some(FeedbackType::HumanReview),
                        "validationfailure" | "validation_failure" => {
                            Some(FeedbackType::ValidationFailure)
                        }
                        "testfailure" | "test_failure" => Some(FeedbackType::TestFailure),
                        "taskescalation" | "task_escalation" => Some(FeedbackType::TaskEscalation),
                        "success" => Some(FeedbackType::Success),
                        _ => None,
                    }
                }),
                outcome: opts.get::<String>("outcome").ok().and_then(|s| {
                    match s.to_lowercase().as_str() {
                        "positive" => Some(FeedbackOutcome::Positive),
                        "negative" => Some(FeedbackOutcome::Negative),
                        "neutral" => Some(FeedbackOutcome::Neutral),
                        _ => None,
                    }
                }),
                contribution_id: opts.get::<String>("contribution_id").ok(),
                agent_id: opts.get::<String>("agent_id").ok(),
                processed: opts.get::<bool>("processed").ok(),
                limit: opts.get::<usize>("limit").ok(),
            }
        } else {
            FeedbackQuery::default()
        };

        let response = LearningSystem::list_feedback(&storage_fb, &db_fb, &query)
            .map_err(|e| mlua::Error::RuntimeError(format!("List error: {}", e)))?;
        let feedback = response.feedback;

        let result = lua.create_table()?;
        for (idx, event) in feedback.iter().enumerate() {
            let json_str = serde_json::to_string(event)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let lua_val: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&json_str)))
                .eval()
                .unwrap_or(LuaValue::Nil);
            result.set(idx + 1, lua_val)?;
        }

        Ok(result)
    })?;
    learning_table.set("list_feedback", list_feedback_fn)?;

    // solidb.ai.learning.list_patterns(options?) -> array of patterns
    let storage_pat = storage.clone();
    let db_pat = db_name.to_string();
    let list_patterns_fn = lua.create_function(move |lua, options: Option<Table>| {
        let query = if let Some(opts) = options {
            PatternQuery {
                pattern_type: opts.get::<String>("pattern_type").ok().and_then(|s| {
                    match s.to_lowercase().as_str() {
                        "successpattern" | "success_pattern" | "success" => {
                            Some(PatternType::SuccessPattern)
                        }
                        "antipattern" | "anti_pattern" | "anti" => Some(PatternType::AntiPattern),
                        "errorpattern" | "error_pattern" | "error" => {
                            Some(PatternType::ErrorPattern)
                        }
                        "escalationpattern" | "escalation_pattern" | "escalation" => {
                            Some(PatternType::EscalationPattern)
                        }
                        _ => None,
                    }
                }),
                task_type: opts.get::<String>("task_type").ok().and_then(|s| {
                    match s.to_lowercase().as_str() {
                        "analyzecontribution" | "analyze" => Some(AITaskType::AnalyzeContribution),
                        "generatecode" | "generate" => Some(AITaskType::GenerateCode),
                        "validatecode" | "validate" => Some(AITaskType::ValidateCode),
                        "runtests" | "tests" => Some(AITaskType::RunTests),
                        "preparereview" | "review" => Some(AITaskType::PrepareReview),
                        "mergechanges" | "merge" => Some(AITaskType::MergeChanges),
                        _ => None,
                    }
                }),
                min_confidence: opts.get::<f64>("min_confidence").ok(),
                limit: opts.get::<usize>("limit").ok(),
            }
        } else {
            PatternQuery::default()
        };

        let response = LearningSystem::list_patterns(&storage_pat, &db_pat, &query)
            .map_err(|e| mlua::Error::RuntimeError(format!("List error: {}", e)))?;
        let patterns = response.patterns;

        let result = lua.create_table()?;
        for (idx, pattern) in patterns.iter().enumerate() {
            let json_str = serde_json::to_string(pattern)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let lua_val: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&json_str)))
                .eval()
                .unwrap_or(LuaValue::Nil);
            result.set(idx + 1, lua_val)?;
        }

        Ok(result)
    })?;
    learning_table.set("list_patterns", list_patterns_fn)?;

    // solidb.ai.learning.get_recommendations(task_id) -> array of recommendations
    let storage_rec = storage.clone();
    let db_rec = db_name.to_string();
    let recommendations_fn = lua.create_function(move |lua, task_id: String| {
        let db = storage_rec
            .get_database(&db_rec)
            .map_err(|e| mlua::Error::RuntimeError(format!("Database error: {}", e)))?;

        // Get task
        let tasks_coll = match db.get_collection("_ai_tasks") {
            Ok(c) => c,
            Err(_) => {
                let result = lua.create_table()?;
                return Ok(result);
            }
        };

        let doc = match tasks_coll.get(&task_id) {
            Ok(d) => d,
            Err(_) => {
                let result = lua.create_table()?;
                return Ok(result);
            }
        };

        let task: AITask = serde_json::from_value(doc.to_value())
            .map_err(|e| mlua::Error::RuntimeError(format!("Parse error: {}", e)))?;

        // Get contribution status
        let contrib_status = match db.get_collection("_ai_contributions") {
            Ok(coll) => match coll.get(&task.contribution_id) {
                Ok(doc) => serde_json::from_value::<Contribution>(doc.to_value())
                    .ok()
                    .map(|c| c.status),
                Err(_) => None,
            },
            Err(_) => None,
        };

        let recommendations = LearningSystem::get_recommendations(
            &storage_rec,
            &db_rec,
            &task,
            contrib_status.as_ref(),
        )
        .map_err(|e| mlua::Error::RuntimeError(format!("Recommendations error: {}", e)))?;

        let result = lua.create_table()?;
        for (idx, rec) in recommendations.iter().enumerate() {
            let json_str = serde_json::to_string(rec)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let lua_val: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&json_str)))
                .eval()
                .unwrap_or(LuaValue::Nil);
            result.set(idx + 1, lua_val)?;
        }

        Ok(result)
    })?;
    learning_table.set("get_recommendations", recommendations_fn)?;

    // solidb.ai.learning.process_batch(limit?) -> processing result
    let storage_proc = storage.clone();
    let db_proc = db_name.to_string();
    let process_fn = lua.create_function(move |lua, limit: Option<usize>| {
        let result =
            LearningSystem::process_feedback_batch(&storage_proc, &db_proc, limit.unwrap_or(100))
                .map_err(|e| mlua::Error::RuntimeError(format!("Processing error: {}", e)))?;

        let json_str = serde_json::to_string(&result)
            .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
        let lua_val: LuaValue = lua
            .load(&format!("return {}", json_to_lua(&json_str)))
            .eval()
            .unwrap_or(LuaValue::Nil);

        Ok(lua_val)
    })?;
    learning_table.set("process_batch", process_fn)?;

    Ok(learning_table)
}

/// Create the solidb.ai.recovery table
fn create_recovery_table(
    lua: &Lua,
    storage: Arc<StorageEngine>,
    db_name: &str,
) -> LuaResult<Table> {
    let recovery_table = lua.create_table()?;

    // solidb.ai.recovery.get_status() -> recovery system status
    let storage_status = storage.clone();
    let db_status = db_name.to_string();
    let status_fn = lua.create_function(move |lua, ()| {
        let worker = RecoveryWorker::new(
            storage_status.clone(),
            db_status.clone(),
            RecoveryConfig::default(),
        );

        let status = worker
            .get_status()
            .map_err(|e| mlua::Error::RuntimeError(format!("Status error: {}", e)))?;

        let json_str = serde_json::to_string(&status)
            .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
        let lua_val: LuaValue = lua
            .load(&format!("return {}", json_to_lua(&json_str)))
            .eval()
            .unwrap_or(LuaValue::Nil);

        Ok(lua_val)
    })?;
    recovery_table.set("get_status", status_fn)?;

    // solidb.ai.recovery.retry_task(task_id) -> boolean
    let storage_retry = storage.clone();
    let db_retry = db_name.to_string();
    let retry_fn = lua.create_function(move |_lua, task_id: String| {
        let worker = RecoveryWorker::new(
            storage_retry.clone(),
            db_retry.clone(),
            RecoveryConfig::default(),
        );

        let success = worker
            .force_retry_task(&task_id)
            .map_err(|e| mlua::Error::RuntimeError(format!("Retry error: {}", e)))?;

        Ok(success)
    })?;
    recovery_table.set("retry_task", retry_fn)?;

    // solidb.ai.recovery.reset_circuit(agent_id) -> boolean
    let storage_reset = storage.clone();
    let db_reset = db_name.to_string();
    let reset_fn = lua.create_function(move |_lua, agent_id: String| {
        let worker = RecoveryWorker::new(
            storage_reset.clone(),
            db_reset.clone(),
            RecoveryConfig::default(),
        );

        worker
            .reset_circuit_breaker(&agent_id)
            .map_err(|e| mlua::Error::RuntimeError(format!("Reset error: {}", e)))?;

        Ok(true)
    })?;
    recovery_table.set("reset_circuit", reset_fn)?;

    // solidb.ai.recovery.list_events(limit?) -> array of recovery events
    let storage_events = storage.clone();
    let db_events = db_name.to_string();
    let events_fn = lua.create_function(move |lua, limit: Option<usize>| {
        let worker = RecoveryWorker::new(
            storage_events.clone(),
            db_events.clone(),
            RecoveryConfig::default(),
        );

        let events = worker
            .list_events(limit)
            .map_err(|e| mlua::Error::RuntimeError(format!("List error: {}", e)))?;

        let result = lua.create_table()?;
        for (idx, event) in events.iter().enumerate() {
            let json_str = serde_json::to_string(event)
                .map_err(|e| mlua::Error::RuntimeError(format!("JSON error: {}", e)))?;
            let lua_val: LuaValue = lua
                .load(&format!("return {}", json_to_lua(&json_str)))
                .eval()
                .unwrap_or(LuaValue::Nil);
            result.set(idx + 1, lua_val)?;
        }

        Ok(result)
    })?;
    recovery_table.set("list_events", events_fn)?;

    Ok(recovery_table)
}

/// Convert JSON string to Lua table literal
fn json_to_lua(json: &str) -> String {
    // Parse and convert to Lua syntax
    if let Ok(value) = serde_json::from_str::<JsonValue>(json) {
        json_value_to_lua(&value)
    } else {
        "nil".to_string()
    }
}

fn json_value_to_lua(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "nil".to_string(),
        JsonValue::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        JsonValue::Number(n) => n.to_string(),
        JsonValue::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        JsonValue::Array(arr) => {
            let items: Vec<String> = arr.iter().map(json_value_to_lua).collect();
            format!("{{{}}}", items.join(", "))
        }
        JsonValue::Object(obj) => {
            let items: Vec<String> = obj
                .iter()
                .map(|(k, v)| format!("[\"{}\"] = {}", k, json_value_to_lua(v)))
                .collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

/// Convert Lua table to JSON value
fn lua_table_to_json(lua: &Lua, table: Table) -> LuaResult<JsonValue> {
    let mut map = serde_json::Map::new();

    for pair in table.pairs::<LuaValue, LuaValue>() {
        let (key, value) = pair?;

        let key_str = match key {
            LuaValue::String(s) => s.to_str()?.to_string(),
            LuaValue::Integer(i) => i.to_string(),
            _ => continue,
        };

        let json_val = lua_value_to_json(lua, value)?;
        map.insert(key_str, json_val);
    }

    Ok(JsonValue::Object(map))
}

fn lua_value_to_json(lua: &Lua, value: LuaValue) -> LuaResult<JsonValue> {
    match value {
        LuaValue::Nil => Ok(JsonValue::Null),
        LuaValue::Boolean(b) => Ok(JsonValue::Bool(b)),
        LuaValue::Integer(i) => Ok(JsonValue::Number(i.into())),
        LuaValue::Number(n) => Ok(serde_json::Number::from_f64(n)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null)),
        LuaValue::String(s) => Ok(JsonValue::String(s.to_str()?.to_string())),
        LuaValue::Table(t) => lua_table_to_json(lua, t),
        _ => Ok(JsonValue::Null),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_to_lua_simple() {
        assert_eq!(json_to_lua("null"), "nil");
        assert_eq!(json_to_lua("true"), "true");
        assert_eq!(json_to_lua("false"), "false");
        assert_eq!(json_to_lua("42"), "42");
        assert_eq!(json_to_lua("\"hello\""), "\"hello\"");
    }

    #[test]
    fn test_json_to_lua_array() {
        let result = json_to_lua("[1, 2, 3]");
        assert_eq!(result, "{1, 2, 3}");
    }

    #[test]
    fn test_json_to_lua_object() {
        let result = json_to_lua(r#"{"a": 1, "b": "test"}"#);
        // Order may vary, just check it's valid
        assert!(result.contains("[\"a\"] = 1"));
        assert!(result.contains("[\"b\"] = \"test\""));
    }
}
