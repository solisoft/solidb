use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;

// ==================== Configuration Structs ====================

/// Configuration for creating a script
pub struct ScriptCreateConfig {
    pub name: String,
    pub path: String,
    pub methods: Vec<String>,
    pub code: String,
    pub description: Option<String>,
    pub collection: Option<String>,
}

/// Configuration for updating a script
pub struct ScriptUpdateConfig {
    pub name: Option<String>,
    pub path: Option<String>,
    pub methods: Option<Vec<String>>,
    pub code: Option<String>,
    pub description: Option<String>,
}

/// Configuration for listing jobs
pub struct TriggerCreateConfig {
    pub name: String,
    pub collection: String,
    pub events: Vec<String>,
    pub script_path: String,
    pub filter: Option<String>,
    pub queue: Option<String>,
    pub priority: Option<i32>,
    pub max_retries: Option<u32>,
    pub enabled: bool,
}

/// Configuration for updating a trigger
pub struct TriggerUpdateConfig {
    pub name: Option<String>,
    pub events: Option<Vec<String>>,
    pub script_path: Option<String>,
    pub filter: Option<String>,
    pub queue: Option<String>,
    pub priority: Option<i32>,
    pub max_retries: Option<u32>,
    pub enabled: Option<bool>,
}

// ==================== Script Management ====================

pub async fn handle_script_create(
    handler: &DriverHandler,
    database: String,
    config: ScriptCreateConfig,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => {
            let scripts_coll = match db.get_or_create_collection("_scripts") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            let script_doc = serde_json::json!({
                "name": config.name,
                "path": config.path,
                "methods": config.methods,
                "code": config.code,
                "description": config.description,
                "collection": config.collection,
                "created_at": chrono::Utc::now().to_rfc3339(),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            });

            match scripts_coll.insert(script_doc) {
                Ok(doc) => Response::ok(doc.to_value()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_script_list(handler: &DriverHandler, database: String) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_scripts") {
            Ok(coll) => {
                let scripts: Vec<_> = coll.scan(None).into_iter().map(|d| d.to_value()).collect();
                Response::ok(serde_json::json!({"scripts": scripts}))
            }
            Err(_) => Response::ok(serde_json::json!({"scripts": []})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_script_get(
    handler: &DriverHandler,
    database: String,
    script_id: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_scripts") {
            Ok(coll) => match coll.get(&script_id) {
                Ok(doc) => Response::ok(doc.to_value()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_script_update(
    handler: &DriverHandler,
    database: String,
    script_id: String,
    config: ScriptUpdateConfig,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_scripts") {
            Ok(coll) => {
                let mut update = serde_json::Map::new();
                if let Some(v) = config.name {
                    update.insert("name".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.path {
                    update.insert("path".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.methods {
                    update.insert("methods".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.code {
                    update.insert("code".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.description {
                    update.insert("description".to_string(), serde_json::json!(v));
                }
                update.insert(
                    "updated_at".to_string(),
                    serde_json::json!(chrono::Utc::now().to_rfc3339()),
                );

                match coll.get(&script_id) {
                    Ok(existing) => {
                        let mut merged = existing.data.clone();
                        if let Some(obj) = merged.as_object_mut() {
                            for (k, v) in update {
                                obj.insert(k, v);
                            }
                        }
                        match coll.update(&script_id, merged) {
                            Ok(doc) => Response::ok(doc.to_value()),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_script_delete(
    handler: &DriverHandler,
    database: String,
    script_id: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_scripts") {
            Ok(coll) => match coll.delete(&script_id) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

// ==================== Job/Queue Handlers ====================

pub async fn handle_list_triggers(handler: &DriverHandler, database: String) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_triggers") {
            Ok(coll) => {
                let triggers: Vec<_> = coll.scan(None).into_iter().map(|d| d.to_value()).collect();
                Response::ok(serde_json::json!({"triggers": triggers}))
            }
            Err(_) => Response::ok(serde_json::json!({"triggers": []})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_list_collection_triggers(
    handler: &DriverHandler,
    database: String,
    collection: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_triggers") {
            Ok(coll) => {
                let triggers: Vec<_> = coll
                    .scan(None)
                    .into_iter()
                    .filter(|t| {
                        t.data
                            .get("collection")
                            .and_then(|v| v.as_str())
                            .map(|c| c == collection)
                            .unwrap_or(false)
                    })
                    .map(|d| d.to_value())
                    .collect();
                Response::ok(serde_json::json!({"triggers": triggers}))
            }
            Err(_) => Response::ok(serde_json::json!({"triggers": []})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_create_trigger(
    handler: &DriverHandler,
    database: String,
    config: TriggerCreateConfig,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => {
            let triggers_coll = match db.get_or_create_collection("_triggers") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            let trigger_doc = serde_json::json!({
                "name": config.name,
                "collection": config.collection,
                "events": config.events,
                "script_path": config.script_path,
                "filter": config.filter,
                "queue": config.queue.unwrap_or_else(|| "default".to_string()),
                "priority": config.priority.unwrap_or(0),
                "max_retries": config.max_retries.unwrap_or(3),
                "enabled": config.enabled,
                "created_at": chrono::Utc::now().to_rfc3339(),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            });

            match triggers_coll.insert(trigger_doc) {
                Ok(doc) => Response::ok(doc.to_value()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_get_trigger(
    handler: &DriverHandler,
    database: String,
    trigger_id: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_triggers") {
            Ok(coll) => match coll.get(&trigger_id) {
                Ok(doc) => Response::ok(doc.to_value()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_update_trigger(
    handler: &DriverHandler,
    database: String,
    trigger_id: String,
    config: TriggerUpdateConfig,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_triggers") {
            Ok(coll) => {
                let mut update = serde_json::Map::new();
                if let Some(v) = config.name {
                    update.insert("name".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.events {
                    update.insert("events".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.script_path {
                    update.insert("script_path".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.filter {
                    update.insert("filter".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.queue {
                    update.insert("queue".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.priority {
                    update.insert("priority".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.max_retries {
                    update.insert("max_retries".to_string(), serde_json::json!(v));
                }
                if let Some(v) = config.enabled {
                    update.insert("enabled".to_string(), serde_json::json!(v));
                }
                update.insert(
                    "updated_at".to_string(),
                    serde_json::json!(chrono::Utc::now().to_rfc3339()),
                );

                match coll.get(&trigger_id) {
                    Ok(existing) => {
                        let mut merged = existing.data.clone();
                        if let Some(obj) = merged.as_object_mut() {
                            for (k, v) in update {
                                obj.insert(k, v);
                            }
                        }
                        match coll.update(&trigger_id, merged) {
                            Ok(doc) => Response::ok(doc.to_value()),
                            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                        }
                    }
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                }
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_delete_trigger(
    handler: &DriverHandler,
    database: String,
    trigger_id: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_triggers") {
            Ok(coll) => match coll.delete(&trigger_id) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_toggle_trigger(
    handler: &DriverHandler,
    database: String,
    trigger_id: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_triggers") {
            Ok(coll) => match coll.get(&trigger_id) {
                Ok(existing) => {
                    let current_enabled = existing
                        .data
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let mut merged = existing.data.clone();
                    if let Some(obj) = merged.as_object_mut() {
                        obj.insert("enabled".to_string(), serde_json::json!(!current_enabled));
                        obj.insert(
                            "updated_at".to_string(),
                            serde_json::json!(chrono::Utc::now().to_rfc3339()),
                        );
                    }
                    match coll.update(&trigger_id, merged) {
                        Ok(doc) => Response::ok(doc.to_value()),
                        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                    }
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}
