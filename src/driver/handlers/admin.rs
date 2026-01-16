use super::DriverHandler;
use crate::driver::protocol::{DriverError, Response};
use std::collections::HashMap;

// ==================== Environment Variable Handlers ====================

pub async fn handle_list_env_vars(handler: &DriverHandler, database: String) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_env") {
            Ok(coll) => {
                let mut vars: HashMap<String, String> = HashMap::new();
                for doc in coll.scan(None) {
                    if let (Some(key), Some(value)) = (
                        doc.data.get("key").and_then(|v| v.as_str()),
                        doc.data.get("value").and_then(|v| v.as_str()),
                    ) {
                        vars.insert(key.to_string(), value.to_string());
                    }
                }
                Response::ok(serde_json::json!({"variables": vars}))
            }
            Err(_) => Response::ok(serde_json::json!({"variables": {}})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_set_env_var(
    handler: &DriverHandler,
    database: String,
    key: String,
    value: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => {
            let env_coll = match db.get_or_create_collection("_env") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            // Use key as _key for easy lookup
            let env_doc = serde_json::json!({
                "_key": key,
                "key": key,
                "value": value,
                "updated_at": chrono::Utc::now().to_rfc3339(),
            });

            // Try update first, then insert
            match env_coll.update(&key, env_doc.clone()) {
                Ok(_) => Response::ok_empty(),
                Err(_) => match env_coll.insert(env_doc) {
                    Ok(_) => Response::ok_empty(),
                    Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
                },
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_delete_env_var(
    handler: &DriverHandler,
    database: String,
    key: String,
) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.get_collection("_env") {
            Ok(coll) => match coll.delete(&key) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

// ==================== Role Management Handlers ====================

pub async fn handle_list_roles(handler: &DriverHandler) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => match db.get_collection("_roles") {
            Ok(coll) => {
                let roles: Vec<_> = coll.scan(None).into_iter().map(|d| d.to_value()).collect();
                Response::ok(serde_json::json!({"roles": roles}))
            }
            Err(_) => Response::ok(serde_json::json!({"roles": []})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_create_role(
    handler: &DriverHandler,
    name: String,
    permissions: Vec<String>,
) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => {
            let roles_coll = match db.get_or_create_collection("_roles") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            let role_doc = serde_json::json!({
                "_key": name,
                "name": name,
                "permissions": permissions,
                "created_at": chrono::Utc::now().to_rfc3339(),
            });

            match roles_coll.insert(role_doc) {
                Ok(doc) => Response::ok(doc.to_value()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_get_role(handler: &DriverHandler, name: String) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => match db.get_collection("_roles") {
            Ok(coll) => match coll.get(&name) {
                Ok(doc) => Response::ok(doc.to_value()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_update_role(
    handler: &DriverHandler,
    name: String,
    permissions: Vec<String>,
) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => match db.get_collection("_roles") {
            Ok(coll) => match coll.get(&name) {
                Ok(existing) => {
                    let mut merged = existing.data.clone();
                    if let Some(obj) = merged.as_object_mut() {
                        obj.insert("permissions".to_string(), serde_json::json!(permissions));
                        obj.insert(
                            "updated_at".to_string(),
                            serde_json::json!(chrono::Utc::now().to_rfc3339()),
                        );
                    }
                    match coll.update(&name, merged) {
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

pub async fn handle_delete_role(handler: &DriverHandler, name: String) -> Response {
    // Prevent deleting built-in roles
    if name == "admin" || name == "developer" || name == "viewer" {
        return Response::error(DriverError::DatabaseError(
            "Cannot delete built-in role".to_string(),
        ));
    }

    match handler.storage.get_database("_system") {
        Ok(db) => match db.get_collection("_roles") {
            Ok(coll) => match coll.delete(&name) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

// ==================== User Management Handlers ====================

pub async fn handle_list_users(handler: &DriverHandler) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => match db.get_collection("_admins") {
            Ok(coll) => {
                let users: Vec<_> = coll
                    .scan(None)
                    .into_iter()
                    .map(|d| {
                        // Strip password_hash from response
                        let mut val = d.to_value();
                        if let Some(obj) = val.as_object_mut() {
                            obj.remove("password_hash");
                        }
                        val
                    })
                    .collect();
                Response::ok(serde_json::json!({"users": users}))
            }
            Err(_) => Response::ok(serde_json::json!({"users": []})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_create_user(
    handler: &DriverHandler,
    username: String,
    password: String,
    roles: Vec<String>,
) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => {
            let admins_coll = match db.get_or_create_collection("_admins") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            // Hash password
            let password_hash = crate::server::auth::AuthService::hash_password(&password);

            let user_doc = serde_json::json!({
                "_key": username,
                "username": username,
                "password_hash": password_hash,
                "roles": roles,
                "created_at": chrono::Utc::now().to_rfc3339(),
            });

            match admins_coll.insert(user_doc) {
                Ok(doc) => {
                    let mut val = doc.to_value();
                    if let Some(obj) = val.as_object_mut() {
                        obj.remove("password_hash");
                    }
                    Response::ok(val)
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_delete_user(handler: &DriverHandler, username: String) -> Response {
    // Prevent deleting admin user
    if username == "admin" {
        return Response::error(DriverError::DatabaseError(
            "Cannot delete admin user".to_string(),
        ));
    }

    match handler.storage.get_database("_system") {
        Ok(db) => match db.get_collection("_admins") {
            Ok(coll) => match coll.delete(&username) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_get_user_roles(handler: &DriverHandler, username: String) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => match db.get_collection("_user_roles") {
            Ok(coll) => {
                let roles: Vec<_> = coll
                    .scan(None)
                    .into_iter()
                    .filter(|d| {
                        d.data
                            .get("username")
                            .and_then(|v| v.as_str())
                            .map(|u| u == username)
                            .unwrap_or(false)
                    })
                    .map(|d| d.to_value())
                    .collect();
                Response::ok(serde_json::json!({"roles": roles}))
            }
            Err(_) => Response::ok(serde_json::json!({"roles": []})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_assign_role(
    handler: &DriverHandler,
    username: String,
    role: String,
    database: Option<String>,
) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => {
            let user_roles_coll = match db.get_or_create_collection("_user_roles") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            let role_doc = serde_json::json!({
                "username": username,
                "role": role,
                "database": database,
                "assigned_at": chrono::Utc::now().to_rfc3339(),
            });

            match user_roles_coll.insert(role_doc) {
                Ok(doc) => Response::ok(doc.to_value()),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_revoke_role(
    handler: &DriverHandler,
    username: String,
    role: String,
) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => match db.get_collection("_user_roles") {
            Ok(coll) => {
                // Find and delete the role assignment
                for doc in coll.scan(None) {
                    let matches = doc
                        .data
                        .get("username")
                        .and_then(|v| v.as_str())
                        .map(|u| u == username)
                        .unwrap_or(false)
                        && doc
                            .data
                            .get("role")
                            .and_then(|v| v.as_str())
                            .map(|r| r == role)
                            .unwrap_or(false);
                    if matches {
                        if let Some(key) = doc.data.get("_key").and_then(|v| v.as_str()) {
                            let _ = coll.delete(key);
                        }
                    }
                }
                Response::ok_empty()
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

// ==================== API Key Management Handlers ====================

pub async fn handle_list_api_keys(handler: &DriverHandler) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => match db.get_collection("_api_keys") {
            Ok(coll) => {
                let keys: Vec<_> = coll
                    .scan(None)
                    .into_iter()
                    .map(|d| {
                        // Strip the actual key value from response
                        let mut val = d.to_value();
                        if let Some(obj) = val.as_object_mut() {
                            obj.remove("key");
                        }
                        val
                    })
                    .collect();
                Response::ok(serde_json::json!({"api_keys": keys}))
            }
            Err(_) => Response::ok(serde_json::json!({"api_keys": []})),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_create_api_key(
    handler: &DriverHandler,
    name: String,
    permissions: Vec<String>,
    expires_at: Option<i64>,
) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => {
            let api_keys_coll = match db.get_or_create_collection("_api_keys") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            // Generate a random API key
            let key = format!("sdb_{}", uuid::Uuid::new_v4().to_string().replace("-", ""));

            let api_key_doc = serde_json::json!({
                "name": name,
                "key": key,
                "permissions": permissions,
                "expires_at": expires_at, // Use i64 directly, serialization will handle it
                "created_at": chrono::Utc::now().to_rfc3339(),
            });

            match api_keys_coll.insert(api_key_doc) {
                Ok(doc) => {
                    // Return the key only on creation
                    let mut val = doc.to_value();
                    if let Some(obj) = val.as_object_mut() {
                        obj.insert("key".to_string(), serde_json::json!(key));
                    }
                    Response::ok(val)
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            }
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_delete_api_key(handler: &DriverHandler, key_id: String) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => match db.get_collection("_api_keys") {
            Ok(coll) => match coll.delete(&key_id) {
                Ok(_) => Response::ok_empty(),
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}
