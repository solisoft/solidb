use super::DriverHandler;
use crate::driver::protocol::{DriverError, Response};
use std::collections::HashSet;

// ==================== Script Management ====================

pub async fn handle_script_create(
    handler: &DriverHandler,
    database: String,
    name: String,
    path: String,
    methods: Vec<String>,
    code: String,
    description: Option<String>,
    collection: Option<String>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => {
            let scripts_coll = match db.get_or_create_collection("_scripts") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            let script_doc = serde_json::json!({
                "name": name,
                "path": path,
                "methods": methods,
                "code": code,
                "description": description,
                "collection": collection,
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
    name: Option<String>,
    path: Option<String>,
    methods: Option<Vec<String>>,
    code: Option<String>,
    description: Option<String>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_scripts") {
            Ok(coll) => {
                let mut update = serde_json::Map::new();
                if let Some(v) = name {
                    update.insert("name".to_string(), serde_json::json!(v));
                }
                if let Some(v) = path {
                    update.insert("path".to_string(), serde_json::json!(v));
                }
                if let Some(v) = methods {
                    update.insert("methods".to_string(), serde_json::json!(v));
                }
                if let Some(v) = code {
                    update.insert("code".to_string(), serde_json::json!(v));
                }
                if let Some(v) = description {
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

pub async fn handle_list_queues(handler: &DriverHandler, database: String) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_jobs") {
            Ok(coll) => {
                let jobs: Vec<_> = coll.scan(None);
                let mut queues: HashSet<String> = HashSet::new();
                for job in jobs {
                    if let Some(queue) = job.data.get("queue").and_then(|v| v.as_str()) {
                        queues.insert(queue.to_string());
                    }
                }
                let queue_list: Vec<_> = queues.into_iter().collect();
                Response::ok(serde_json::json!({"queues": queue_list}))
            }
            Err(_) => Response::ok(serde_json::json!({"queues": []})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_list_jobs(
    handler: &DriverHandler,
    database: String,
    queue_name: String,
    status: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_jobs") {
            Ok(coll) => {
                let jobs: Vec<_> = coll
                    .scan(None)
                    .into_iter()
                    .filter(|job| {
                        let queue_match = job
                            .data
                            .get("queue")
                            .and_then(|v| v.as_str())
                            .map(|q| q == queue_name)
                            .unwrap_or(false);
                        let status_match = status.as_ref().map_or(true, |s| {
                            job.data
                                .get("status")
                                .and_then(|v| v.as_str())
                                .map(|js| js == s)
                                .unwrap_or(false)
                        });
                        queue_match && status_match
                    })
                    .skip(offset.unwrap_or(0))
                    .take(limit.unwrap_or(50))
                    .map(|d| d.to_value())
                    .collect();
                Response::ok(serde_json::json!({"jobs": jobs}))
            }
            Err(_) => Response::ok(serde_json::json!({"jobs": []})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_enqueue_job(
    handler: &DriverHandler,
    database: String,
    queue_name: String,
    script_path: String,
    params: Option<serde_json::Value>,
    priority: Option<i32>,
    run_at: Option<i64>,
    max_retries: Option<u32>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => {
            let jobs_coll = match db.get_or_create_collection("_jobs") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            let job_doc = serde_json::json!({
                "queue": queue_name,
                "script_path": script_path,
                "params": params.unwrap_or(serde_json::json!({})),
                "priority": priority.unwrap_or(0),
                "run_at": run_at,
                "max_retries": max_retries.unwrap_or(3),
                "retry_count": 0,
                "status": "pending",
                "created_at": chrono::Utc::now().to_rfc3339(),
            });

            match jobs_coll.insert(job_doc) {
                Ok(doc) => Response::ok(doc.to_value()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_cancel_job(
    handler: &DriverHandler,
    database: String,
    job_id: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_jobs") {
            Ok(coll) => match coll.delete(&job_id) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

// ==================== Cron Job Handlers ====================

pub async fn handle_list_cron_jobs(handler: &DriverHandler, database: String) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_cron_jobs") {
            Ok(coll) => {
                let cron_jobs: Vec<_> = coll.scan(None).into_iter().map(|d| d.to_value()).collect();
                Response::ok(serde_json::json!({"cron_jobs": cron_jobs}))
            }
            Err(_) => Response::ok(serde_json::json!({"cron_jobs": []})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_create_cron_job(
    handler: &DriverHandler,
    database: String,
    name: String,
    cron_expression: String,
    script_path: String,
    params: Option<serde_json::Value>,
    queue: Option<String>,
    priority: Option<i32>,
    max_retries: Option<u32>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => {
            let cron_coll = match db.get_or_create_collection("_cron_jobs") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            let cron_doc = serde_json::json!({
                "name": name,
                "cron_expression": cron_expression,
                "script_path": script_path,
                "params": params.unwrap_or(serde_json::json!({})),
                "queue": queue.unwrap_or_else(|| "default".to_string()),
                "priority": priority.unwrap_or(0),
                "max_retries": max_retries.unwrap_or(3),
                "created_at": chrono::Utc::now().to_rfc3339(),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            });

            match cron_coll.insert(cron_doc) {
                Ok(doc) => Response::ok(doc.to_value()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_update_cron_job(
    handler: &DriverHandler,
    database: String,
    cron_id: String,
    name: Option<String>,
    cron_expression: Option<String>,
    script_path: Option<String>,
    params: Option<serde_json::Value>,
    queue: Option<String>,
    priority: Option<i32>,
    max_retries: Option<u32>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_cron_jobs") {
            Ok(coll) => {
                let mut update = serde_json::Map::new();
                if let Some(v) = name {
                    update.insert("name".to_string(), serde_json::json!(v));
                }
                if let Some(v) = cron_expression {
                    update.insert("cron_expression".to_string(), serde_json::json!(v));
                }
                if let Some(v) = script_path {
                    update.insert("script_path".to_string(), serde_json::json!(v));
                }
                if let Some(v) = params {
                    update.insert("params".to_string(), serde_json::json!(v));
                }
                if let Some(v) = queue {
                    update.insert("queue".to_string(), serde_json::json!(v));
                }
                if let Some(v) = priority {
                    update.insert("priority".to_string(), serde_json::json!(v));
                }
                if let Some(v) = max_retries {
                    update.insert("max_retries".to_string(), serde_json::json!(v));
                }
                update.insert(
                    "updated_at".to_string(),
                    serde_json::json!(chrono::Utc::now().to_rfc3339()),
                );

                match coll.get(&cron_id) {
                    Ok(existing) => {
                        let mut merged = existing.data.clone();
                        if let Some(obj) = merged.as_object_mut() {
                            for (k, v) in update {
                                obj.insert(k, v);
                            }
                        }
                        match coll.update(&cron_id, merged) {
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

pub async fn handle_delete_cron_job(
    handler: &DriverHandler,
    database: String,
    cron_id: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_cron_jobs") {
            Ok(coll) => match coll.delete(&cron_id) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

// ==================== Trigger Handlers ====================

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
    name: String,
    collection: String,
    events: Vec<String>,
    script_path: String,
    filter: Option<String>,
    queue: Option<String>,
    priority: Option<i32>,
    max_retries: Option<u32>,
    enabled: bool,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => {
            let triggers_coll = match db.get_or_create_collection("_triggers") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            let trigger_doc = serde_json::json!({
                "name": name,
                "collection": collection,
                "events": events,
                "script_path": script_path,
                "filter": filter,
                "queue": queue.unwrap_or_else(|| "default".to_string()),
                "priority": priority.unwrap_or(0),
                "max_retries": max_retries.unwrap_or(3),
                "enabled": enabled,
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
    name: Option<String>,
    events: Option<Vec<String>>,
    script_path: Option<String>,
    filter: Option<String>,
    queue: Option<String>,
    priority: Option<i32>,
    max_retries: Option<u32>,
    enabled: Option<bool>,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_triggers") {
            Ok(coll) => {
                let mut update = serde_json::Map::new();
                if let Some(v) = name {
                    update.insert("name".to_string(), serde_json::json!(v));
                }
                if let Some(v) = events {
                    update.insert("events".to_string(), serde_json::json!(v));
                }
                if let Some(v) = script_path {
                    update.insert("script_path".to_string(), serde_json::json!(v));
                }
                if let Some(v) = filter {
                    update.insert("filter".to_string(), serde_json::json!(v));
                }
                if let Some(v) = queue {
                    update.insert("queue".to_string(), serde_json::json!(v));
                }
                if let Some(v) = priority {
                    update.insert("priority".to_string(), serde_json::json!(v));
                }
                if let Some(v) = max_retries {
                    update.insert("max_retries".to_string(), serde_json::json!(v));
                }
                if let Some(v) = enabled {
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
