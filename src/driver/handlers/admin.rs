use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;
use std::collections::HashMap;

// ==================== Environment Variable Handlers ====================

pub async fn handle_list_env_vars(handler: &DriverHandler, database: String) -> Response {
    match handler.storage.get_database(&database) {
        Ok(db) => match db.system_collection("_env") {
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
            let env_coll = match db.get_or_create_system_collection("_env") {
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
        Ok(db) => match db.system_collection("_env") {
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
        Ok(db) => match db.system_collection("_roles") {
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
    // The same checks and stored shape as `POST /_api/auth/roles`. This used
    // to store the permission strings as they came, which no longer read back
    // as a role: a role created here granted nothing.
    let result = permissions
        .iter()
        .map(|p| crate::server::role_handlers::parse_permission_string(p))
        .collect::<Result<Vec<_>, _>>()
        .and_then(|perms| {
            crate::server::role_handlers::store_new_role(
                &handler.storage,
                handler.replication.as_deref(),
                &name,
                None,
                perms,
            )
        });
    match result {
        Ok(role) => Response::ok(serde_json::json!(role)),
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_get_role(handler: &DriverHandler, name: String) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => match db.system_collection("_roles") {
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
        Ok(db) => match db.system_collection("_roles") {
            Ok(coll) => match coll.get(&name) {
                Ok(existing) => {
                    let parsed = match permissions
                        .iter()
                        .map(|p| crate::server::role_handlers::parse_permission_string(p))
                        .collect::<Result<Vec<_>, _>>()
                    {
                        Ok(p) => p,
                        Err(e) => {
                            return Response::error(DriverError::DatabaseError(e.to_string()))
                        }
                    };
                    let mut merged = existing.data.clone();
                    if let Some(obj) = merged.as_object_mut() {
                        obj.insert("permissions".to_string(), serde_json::json!(parsed));
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
    // The built-in roles (the list said "developer", which is not one, and
    // left "editor" deletable).
    if crate::server::authorization::Role::builtin_roles()
        .iter()
        .any(|r| r.name == name)
    {
        return Response::error(DriverError::DatabaseError(
            "Cannot delete built-in role".to_string(),
        ));
    }

    match handler.storage.get_database("_system") {
        Ok(db) => match db.system_collection("_roles") {
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
        Ok(db) => match db.system_collection("_admins") {
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
            let admins_coll = match db.get_or_create_system_collection("_admins") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            // Hash password. This previously embedded the unhandled
            // `Result` into the document, storing `{"Ok": "..."}` as the
            // hash and making the created user unable to log in.
            let password_hash = match crate::server::auth::hash_password_blocking(&password).await {
                Ok(h) => h,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

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
        Ok(db) => match db.system_collection("_admins") {
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
        Ok(db) => match db.system_collection("_user_roles") {
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
    // The same checks and stored shape as `POST /_api/auth/users/{u}/roles`.
    // This used to insert a row without `assigned_by`, which does not parse as
    // an assignment, so it granted nothing; and it checked neither the role,
    // the user nor the database.
    match crate::server::role_handlers::store_role_assignment(
        &handler.storage,
        handler.replication.as_deref(),
        &username,
        &role,
        database,
        &handler.session_subject,
    ) {
        Ok(assignment) => Response::ok(serde_json::json!(assignment)),
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub async fn handle_revoke_role(
    handler: &DriverHandler,
    username: String,
    role: String,
) -> Response {
    match handler.storage.get_database("_system") {
        Ok(db) => match db.system_collection("_user_roles") {
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
                crate::server::auth::AuthService::invalidate_user_roles_cache(&username);
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
        Ok(db) => match db.system_collection("_api_keys") {
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
            let api_keys_coll = match db.get_or_create_system_collection("_api_keys") {
                Ok(c) => c,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            if permissions.is_empty() {
                return Response::error(DriverError::InvalidCommand(
                    "API keys must declare at least one role".to_string(),
                ));
            }

            // Store only the hash, exactly like `create_api_key_handler`: this
            // path used to persist the raw key and no `key_hash`, so the key
            // sat in plaintext in `_api_keys` and never authenticated over HTTP.
            let (raw_key, key_hash) = crate::server::auth::AuthService::generate_api_key();
            // The driver takes a Unix timestamp; milliseconds are accepted too.
            let expires_at = expires_at.and_then(|ts| {
                let secs = if ts > 1_000_000_000_000 {
                    ts / 1000
                } else {
                    ts
                };
                chrono::DateTime::from_timestamp(secs, 0).map(|d| d.to_rfc3339())
            });
            let api_key = crate::server::auth::ApiKey {
                id: uuid::Uuid::new_v4().to_string(),
                name,
                key_hash,
                created_at: chrono::Utc::now().to_rfc3339(),
                roles: permissions,
                scoped_databases: None,
                expires_at,
            };
            let api_key_doc = match serde_json::to_value(&api_key) {
                Ok(v) => v,
                Err(e) => return Response::error(DriverError::DatabaseError(e.to_string())),
            };

            match api_keys_coll.insert(api_key_doc) {
                Ok(doc) => {
                    crate::server::auth::api_key_cache().insert(api_key);
                    // Return the key only on creation
                    let mut val = doc.to_value();
                    if let Some(obj) = val.as_object_mut() {
                        obj.remove("key_hash");
                        obj.insert("key".to_string(), serde_json::json!(raw_key));
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
        Ok(db) => match db.system_collection("_api_keys") {
            Ok(coll) => match coll.delete(&key_id) {
                Ok(_) => {
                    // Without this the deleted key keeps authenticating over
                    // HTTP from the in-memory cache.
                    crate::server::auth::api_key_cache().remove_by_id(&key_id);
                    Response::ok_empty()
                }
                Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
            },
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        },
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::protocol::Response;
    use crate::server::authorization::{AuthorizationService, PermissionAction};
    use crate::storage::StorageEngine;
    use std::sync::Arc;

    fn handler() -> (tempfile::TempDir, DriverHandler) {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
        engine.initialize().unwrap();
        for db in ["tenant_a", "tenant_b"] {
            engine.create_database(db.to_string()).unwrap();
        }
        let system = engine.get_database("_system").unwrap();
        system
            .get_or_create_system_collection("_admins")
            .unwrap()
            .insert(serde_json::json!({"_key": "carol", "password_hash": "x"}))
            .unwrap();
        let mut h = DriverHandler::new(engine, None);
        h.session_subject = "root".to_string();
        (tmp, h)
    }

    fn is_err(r: &Response) -> bool {
        matches!(r, Response::Error { .. })
    }

    fn can(h: &DriverHandler, action: PermissionAction, db: &str) -> bool {
        let roles = crate::server::auth::AuthService::get_user_roles(&h.storage, "carol")
            .unwrap_or_default();
        let perms = AuthorizationService::load_permissions_from_storage(&h.storage, &roles);
        AuthorizationService::check_permission_raw(&perms, action, Some(db), None).is_ok()
    }

    #[tokio::test]
    async fn a_role_created_and_assigned_over_the_driver_grants_its_permissions() {
        let (_t, h) = handler();
        let r = handle_create_role(
            &h,
            "reporter".into(),
            vec!["write:tenant_a".into(), "read".into()],
        )
        .await;
        assert!(!is_err(&r), "{:?}", r);
        let r = handle_assign_role(&h, "carol".into(), "reporter".into(), None).await;
        assert!(!is_err(&r), "{:?}", r);

        assert!(can(&h, PermissionAction::Write, "tenant_a"));
        assert!(can(&h, PermissionAction::Read, "tenant_b"));
        assert!(!can(&h, PermissionAction::Write, "tenant_b"));
    }

    #[tokio::test]
    async fn a_limited_driver_assignment_stops_at_its_database() {
        let (_t, h) = handler();
        let r =
            handle_assign_role(&h, "carol".into(), "editor".into(), Some("tenant_a".into())).await;
        assert!(!is_err(&r), "{:?}", r);
        assert!(can(&h, PermissionAction::Write, "tenant_a"));
        assert!(!can(&h, PermissionAction::Read, "tenant_b"));
    }

    #[tokio::test]
    async fn the_driver_refuses_what_the_http_api_refuses() {
        let (_t, h) = handler();
        // '@' would be read back as a limited assignment of another role.
        assert!(is_err(
            &handle_create_role(&h, "ops@prod".into(), vec!["read".into()]).await
        ));
        assert!(is_err(
            &handle_create_role(&h, "adminish".into(), vec!["read".into()]).await
        ));
        assert!(is_err(
            &handle_create_role(&h, "x".into(), vec!["fly".into()]).await
        ));
        assert!(is_err(
            &handle_assign_role(&h, "carol".into(), "editor".into(), Some("typo_db".into())).await
        ));
        assert!(is_err(
            &handle_assign_role(&h, "carol".into(), "no_such_role".into(), None).await
        ));
        assert!(is_err(
            &handle_assign_role(&h, "nobody".into(), "editor".into(), None).await
        ));
        assert!(is_err(&handle_delete_role(&h, "editor".into()).await));
    }
}
