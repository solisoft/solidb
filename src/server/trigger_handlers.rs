use axum::{
    extract::{Path, State},
    response::Json,
};
use serde::{Deserialize, Serialize};

use super::handlers::AppState;
use crate::error::DbError;
use crate::triggers::{Trigger, TriggerEvent};

#[derive(Debug, Serialize)]
pub struct ListTriggersResponse {
    pub triggers: Vec<Trigger>,
    pub total: usize,
}

#[derive(Debug, Deserialize)]
pub struct CreateTriggerRequest {
    pub name: String,
    pub collection: String,
    pub events: Vec<String>, // "insert", "update", "delete"
    pub script_path: String,
    #[serde(default)]
    pub queue: Option<String>,
    #[serde(default)]
    pub priority: Option<i32>,
    #[serde(default)]
    pub max_retries: Option<i32>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub filter: Option<String>,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub struct UpdateTriggerRequest {
    pub name: Option<String>,
    pub collection: Option<String>,
    pub events: Option<Vec<String>>,
    pub script_path: Option<String>,
    pub queue: Option<String>,
    pub priority: Option<i32>,
    pub max_retries: Option<i32>,
    pub enabled: Option<bool>,
    pub filter: Option<String>,
}

/// Parse event strings into TriggerEvent enums
fn parse_events(events: &[String]) -> Result<Vec<TriggerEvent>, DbError> {
    let mut result = Vec::new();
    for event in events {
        match event.to_lowercase().as_str() {
            "insert" => result.push(TriggerEvent::Insert),
            "update" => result.push(TriggerEvent::Update),
            "delete" => result.push(TriggerEvent::Delete),
            _ => {
                return Err(DbError::BadRequest(format!(
                    "Invalid event type: '{}'. Must be 'insert', 'update', or 'delete'",
                    event
                )));
            }
        }
    }
    if result.is_empty() {
        return Err(DbError::BadRequest(
            "At least one event type is required".to_string(),
        ));
    }
    Ok(result)
}

/// List all triggers in a database
pub async fn list_triggers_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
) -> Result<Json<ListTriggersResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // If _triggers collection doesn't exist, return empty list
    if db.get_collection("_triggers").is_err() {
        return Ok(Json(ListTriggersResponse {
            triggers: Vec::new(),
            total: 0,
        }));
    }

    let triggers_coll = db.get_collection("_triggers")?;
    let mut triggers = Vec::new();

    for doc in triggers_coll.scan(None) {
        let trigger: Trigger = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted trigger data".to_string()))?;
        triggers.push(trigger);
    }

    // Sort by created_at desc
    triggers.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let total = triggers.len();
    Ok(Json(ListTriggersResponse { triggers, total }))
}

/// Get a single trigger by ID
pub async fn get_trigger_handler(
    State(state): State<AppState>,
    Path((db_name, trigger_id)): Path<(String, String)>,
) -> Result<Json<Trigger>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let triggers_coll = db.get_collection("_triggers")?;

    let doc = triggers_coll.get(&trigger_id)?;
    let trigger: Trigger = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted trigger data".to_string()))?;

    Ok(Json(trigger))
}

/// Create a new trigger
pub async fn create_trigger_handler(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<CreateTriggerRequest>,
) -> Result<Json<Trigger>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // Ensure _triggers collection exists
    if db.get_collection("_triggers").is_err() {
        db.create_collection("_triggers".to_string(), None)?;
    }

    let triggers_coll = db.get_collection("_triggers")?;

    // Validate that the target collection exists
    if db.get_collection(&req.collection).is_err() {
        return Err(DbError::BadRequest(format!(
            "Collection '{}' does not exist",
            req.collection
        )));
    }

    // Parse events
    let events = parse_events(&req.events)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let trigger = Trigger {
        id: uuid::Uuid::new_v4().to_string(),
        revision: None,
        name: req.name,
        collection: req.collection,
        events,
        script_path: req.script_path,
        queue: req.queue.unwrap_or_else(|| "default".to_string()),
        priority: req.priority.unwrap_or(0),
        max_retries: req.max_retries.unwrap_or(5),
        enabled: req.enabled,
        filter: req.filter,
        created_at: now,
        updated_at: now,
    };

    let doc_val = serde_json::to_value(&trigger)
        .map_err(|e| DbError::InternalError(format!("Failed to serialize trigger: {}", e)))?;
    triggers_coll.insert(doc_val)?;

    Ok(Json(trigger))
}

/// Update an existing trigger
pub async fn update_trigger_handler(
    State(state): State<AppState>,
    Path((db_name, trigger_id)): Path<(String, String)>,
    Json(req): Json<UpdateTriggerRequest>,
) -> Result<Json<Trigger>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let triggers_coll = db.get_collection("_triggers")?;

    let doc = triggers_coll.get(&trigger_id)?;
    let mut trigger: Trigger = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted trigger data".to_string()))?;

    // Apply updates
    if let Some(name) = req.name {
        trigger.name = name;
    }
    if let Some(collection) = req.collection {
        // Validate that the new collection exists
        if db.get_collection(&collection).is_err() {
            return Err(DbError::BadRequest(format!(
                "Collection '{}' does not exist",
                collection
            )));
        }
        trigger.collection = collection;
    }
    if let Some(events) = req.events {
        trigger.events = parse_events(&events)?;
    }
    if let Some(script_path) = req.script_path {
        trigger.script_path = script_path;
    }
    if let Some(queue) = req.queue {
        trigger.queue = queue;
    }
    if let Some(priority) = req.priority {
        trigger.priority = priority;
    }
    if let Some(max_retries) = req.max_retries {
        trigger.max_retries = max_retries;
    }
    if let Some(enabled) = req.enabled {
        trigger.enabled = enabled;
    }
    if req.filter.is_some() {
        trigger.filter = req.filter;
    }

    // Update timestamp
    trigger.updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let rev = trigger.revision.clone().unwrap_or_default();
    let doc_val = serde_json::to_value(&trigger)
        .map_err(|e| DbError::InternalError(format!("Failed to serialize trigger: {}", e)))?;
    triggers_coll.update_with_rev(&trigger_id, &rev, doc_val)?;

    Ok(Json(trigger))
}

/// Delete a trigger
pub async fn delete_trigger_handler(
    State(state): State<AppState>,
    Path((db_name, trigger_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let triggers_coll = db.get_collection("_triggers")?;
    triggers_coll.delete(&trigger_id)?;
    Ok(Json(serde_json::json!({ "success": true })))
}

/// List triggers for a specific collection
pub async fn list_collection_triggers_handler(
    State(state): State<AppState>,
    Path((db_name, collection_name)): Path<(String, String)>,
) -> Result<Json<ListTriggersResponse>, DbError> {
    let db = state.storage.get_database(&db_name)?;

    // If _triggers collection doesn't exist, return empty list
    if db.get_collection("_triggers").is_err() {
        return Ok(Json(ListTriggersResponse {
            triggers: Vec::new(),
            total: 0,
        }));
    }

    let triggers_coll = db.get_collection("_triggers")?;
    let mut triggers = Vec::new();

    for doc in triggers_coll.scan(None) {
        let trigger: Trigger = serde_json::from_value(doc.to_value())
            .map_err(|_| DbError::InternalError("Corrupted trigger data".to_string()))?;
        if trigger.collection == collection_name {
            triggers.push(trigger);
        }
    }

    // Sort by created_at desc
    triggers.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let total = triggers.len();
    Ok(Json(ListTriggersResponse { triggers, total }))
}

/// Toggle trigger enabled/disabled
pub async fn toggle_trigger_handler(
    State(state): State<AppState>,
    Path((db_name, trigger_id)): Path<(String, String)>,
) -> Result<Json<Trigger>, DbError> {
    let db = state.storage.get_database(&db_name)?;
    let triggers_coll = db.get_collection("_triggers")?;

    let doc = triggers_coll.get(&trigger_id)?;
    let mut trigger: Trigger = serde_json::from_value(doc.to_value())
        .map_err(|_| DbError::InternalError("Corrupted trigger data".to_string()))?;

    // Toggle enabled state
    trigger.enabled = !trigger.enabled;
    trigger.updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let rev = trigger.revision.clone().unwrap_or_default();
    let doc_val = serde_json::to_value(&trigger)
        .map_err(|e| DbError::InternalError(format!("Failed to serialize trigger: {}", e)))?;
    triggers_coll.update_with_rev(&trigger_id, &rev, doc_val)?;

    Ok(Json(trigger))
}
