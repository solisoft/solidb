//! Coverage for `src/server/role_handlers.rs`: RBAC role + user CRUD,
//! role assignment, and self-service endpoints. See COV-001.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::auth::AuthService;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

fn create_app() -> (TempDir, axum::Router, String, String) {
    let tmp_dir = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).expect("engine");
    engine.initialize().expect("initialize _system");
    let script_stats = Arc::new(ScriptStats::default());
    let router = create_router(engine, None, None, None, None, script_stats, None, None, 0);

    let admin_token =
        AuthService::create_jwt_with_roles("admin_user", Some(vec!["admin".to_string()]), None)
            .expect("admin jwt");
    let viewer_token =
        AuthService::create_jwt_with_roles("viewer_user", Some(vec!["viewer".to_string()]), None)
            .expect("viewer jwt");

    (tmp_dir, router, admin_token, viewer_token)
}

fn bearer(token: &str) -> String {
    format!("Bearer {}", token)
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn req_get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(t) = token {
        b = b.header("Authorization", bearer(t));
    }
    b.body(Body::empty()).unwrap()
}

fn req_json(method: &str, uri: &str, token: &str, payload: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", bearer(token))
        .header("Content-Type", "application/json")
        .body(Body::from(payload.to_string()))
        .unwrap()
}

fn req_delete(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(uri)
        .header("Authorization", bearer(token))
        .body(Body::empty())
        .unwrap()
}

// ===========================================================================
// Roles: list / create / get / update / delete
// ===========================================================================

#[tokio::test]
async fn list_roles_returns_three_builtins() {
    let (_tmp, app, admin, _) = create_app();
    let resp = app
        .oneshot(req_get("/_api/auth/roles", Some(&admin)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let names: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"admin"));
    assert!(names.contains(&"editor"));
    assert!(names.contains(&"viewer"));
}

#[tokio::test]
async fn create_role_happy_path() {
    let (_tmp, app, admin, _) = create_app();
    let payload = json!({
        "name": "ops_reader",
        "description": "ops folks: read everything",
        "permissions": [
            {"action": "read", "scope": "global"},
            {"action": "read", "scope": "database", "database": "metrics"}
        ]
    });
    let resp = app
        .oneshot(req_json("POST", "/_api/auth/roles", &admin, payload))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["name"], "ops_reader");
    assert_eq!(body["is_builtin"], false);
    assert_eq!(body["permissions"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn create_role_rejects_reserved_prefix() {
    let (_tmp, app, admin, _) = create_app();
    let payload = json!({
        "name": "admin_extra",
        "permissions": [{"action": "read", "scope": "global"}]
    });
    let resp = app
        .oneshot(req_json("POST", "/_api/auth/roles", &admin, payload))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_role_rejects_invalid_action() {
    let (_tmp, app, admin, _) = create_app();
    let payload = json!({
        "name": "weird",
        "permissions": [{"action": "delete", "scope": "global"}]
    });
    let resp = app
        .oneshot(req_json("POST", "/_api/auth/roles", &admin, payload))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_role_rejects_database_scope_without_database() {
    let (_tmp, app, admin, _) = create_app();
    let payload = json!({
        "name": "dbscoped",
        "permissions": [{"action": "read", "scope": "database"}]
    });
    let resp = app
        .oneshot(req_json("POST", "/_api/auth/roles", &admin, payload))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_role_duplicate_returns_conflict() {
    let (_tmp, app, admin, _) = create_app();
    let payload = json!({
        "name": "dup_role",
        "permissions": [{"action": "read", "scope": "global"}]
    });
    let resp1 = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/roles",
            &admin,
            payload.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::CREATED);

    let resp2 = app
        .oneshot(req_json("POST", "/_api/auth/roles", &admin, payload))
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn get_role_found_and_not_found() {
    let (_tmp, app, admin, _) = create_app();
    // Builtin admin role exists.
    let resp = app
        .clone()
        .oneshot(req_get("/_api/auth/roles/admin", Some(&admin)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["name"], "admin");
    assert_eq!(body["is_builtin"], true);

    // Missing → 404.
    let resp = app
        .oneshot(req_get("/_api/auth/roles/no_such_role", Some(&admin)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn update_role_custom_then_reject_builtin() {
    let (_tmp, app, admin, _) = create_app();
    // Create a custom role.
    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/roles",
            &admin,
            json!({
                "name": "auditor",
                "permissions": [{"action": "read", "scope": "global"}]
            }),
        ))
        .await
        .unwrap();

    // Update permissions + description.
    let resp = app
        .clone()
        .oneshot(req_json(
            "PUT",
            "/_api/auth/roles/auditor",
            &admin,
            json!({
                "description": "auditing",
                "permissions": [
                    {"action": "read", "scope": "global"},
                    {"action": "write", "scope": "database", "database": "audit_log"}
                ]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["description"], "auditing");
    assert_eq!(body["permissions"].as_array().unwrap().len(), 2);

    // Builtin role can't be modified.
    let resp = app
        .oneshot(req_json(
            "PUT",
            "/_api/auth/roles/admin",
            &admin,
            json!({"description": "hijack"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn update_role_not_found() {
    let (_tmp, app, admin, _) = create_app();
    let resp = app
        .oneshot(req_json(
            "PUT",
            "/_api/auth/roles/ghost",
            &admin,
            json!({"description": "x"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_role_custom_then_reject_builtin_and_missing() {
    let (_tmp, app, admin, _) = create_app();
    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/roles",
            &admin,
            json!({
                "name": "throwaway",
                "permissions": [{"action": "read", "scope": "global"}]
            }),
        ))
        .await
        .unwrap();

    let resp = app
        .clone()
        .oneshot(req_delete("/_api/auth/roles/throwaway", &admin))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Builtin → 403.
    let resp = app
        .clone()
        .oneshot(req_delete("/_api/auth/roles/viewer", &admin))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Missing → 404.
    let resp = app
        .oneshot(req_delete("/_api/auth/roles/throwaway", &admin))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ===========================================================================
// Users: list / create / delete
// ===========================================================================

#[tokio::test]
async fn list_users_includes_default_admin() {
    let (_tmp, app, admin, _) = create_app();
    let resp = app
        .oneshot(req_get("/_api/auth/users", Some(&admin)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let users = body["users"].as_array().unwrap();
    assert!(users.iter().any(|u| u["username"] == "admin"));
}

#[tokio::test]
async fn create_user_happy_path_with_initial_role() {
    let (_tmp, app, admin, _) = create_app();
    let resp = app
        .oneshot(req_json(
            "POST",
            "/_api/auth/users",
            &admin,
            json!({
                "username": "alice",
                "password": "alice-secret",
                "initial_role": "viewer"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["username"], "alice");
    let roles: Vec<&str> = body["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert_eq!(roles, vec!["viewer"]);
}

#[tokio::test]
async fn create_user_rejects_short_password_and_empty_username() {
    let (_tmp, app, admin, _) = create_app();

    let resp = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/users",
            &admin,
            json!({"username": "bob", "password": "short"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let resp = app
        .oneshot(req_json(
            "POST",
            "/_api/auth/users",
            &admin,
            json!({"username": "", "password": "valid_password"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_user_duplicate_returns_conflict() {
    let (_tmp, app, admin, _) = create_app();
    let payload = json!({"username": "dup", "password": "something_long"});
    let r1 = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/users",
            &admin,
            payload.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::CREATED);
    let r2 = app
        .oneshot(req_json("POST", "/_api/auth/users", &admin, payload))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn delete_user_rejects_self_default_admin_and_missing() {
    let (_tmp, app, admin, _) = create_app();

    // Self-delete (claims.sub = "admin_user", so create that user first then try to delete it).
    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/users",
            &admin,
            json!({"username": "admin_user", "password": "long_enough1"}),
        ))
        .await
        .unwrap();
    let resp = app
        .clone()
        .oneshot(req_delete("/_api/auth/users/admin_user", &admin))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Default-admin guard.
    let resp = app
        .clone()
        .oneshot(req_delete("/_api/auth/users/admin", &admin))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // Missing user → 404.
    let resp = app
        .oneshot(req_delete("/_api/auth/users/nobody", &admin))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_user_happy_path_cleans_role_assignments() {
    let (_tmp, app, admin, _) = create_app();
    // Create user with a role.
    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/users",
            &admin,
            json!({
                "username": "to_delete",
                "password": "long_enough1",
                "initial_role": "viewer"
            }),
        ))
        .await
        .unwrap();

    let resp = app
        .clone()
        .oneshot(req_delete("/_api/auth/users/to_delete", &admin))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Subsequent get_user_roles returns empty.
    let resp = app
        .oneshot(req_get("/_api/auth/users/to_delete/roles", Some(&admin)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body.as_array().unwrap().is_empty());
}

// ===========================================================================
// User-role assignment: assign / revoke / list
// ===========================================================================

#[tokio::test]
async fn assign_role_happy_path_then_duplicate_is_conflict() {
    let (_tmp, app, admin, _) = create_app();
    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/users",
            &admin,
            json!({"username": "carol", "password": "long_enough1"}),
        ))
        .await
        .unwrap();

    let r1 = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/users/carol/roles",
            &admin,
            json!({"role": "editor"}),
        ))
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::CREATED);
    let body = body_json(r1).await;
    assert_eq!(body["username"], "carol");
    assert_eq!(body["role"], "editor");

    let r2 = app
        .oneshot(req_json(
            "POST",
            "/_api/auth/users/carol/roles",
            &admin,
            json!({"role": "editor"}),
        ))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn assign_role_unknown_role_or_user() {
    let (_tmp, app, admin, _) = create_app();
    // Unknown role.
    let resp = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/users/admin/roles",
            &admin,
            json!({"role": "ghost_role"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Unknown user.
    let resp = app
        .oneshot(req_json(
            "POST",
            "/_api/auth/users/no_user/roles",
            &admin,
            json!({"role": "viewer"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn revoke_role_happy_path_and_not_assigned() {
    let (_tmp, app, admin, _) = create_app();
    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/users",
            &admin,
            json!({
                "username": "dave",
                "password": "long_enough1",
                "initial_role": "viewer"
            }),
        ))
        .await
        .unwrap();

    // Revoke (no `database` query → matches role with database=None).
    let resp = app
        .clone()
        .oneshot(req_delete("/_api/auth/users/dave/roles/viewer", &admin))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Second revoke → 404.
    let resp = app
        .oneshot(req_delete("/_api/auth/users/dave/roles/viewer", &admin))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_user_roles_lists_assignments() {
    let (_tmp, app, admin, _) = create_app();
    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/users",
            &admin,
            json!({
                "username": "eve",
                "password": "long_enough1",
                "initial_role": "editor"
            }),
        ))
        .await
        .unwrap();

    // Add a second role.
    let _ = app
        .clone()
        .oneshot(req_json(
            "POST",
            "/_api/auth/users/eve/roles",
            &admin,
            json!({"role": "viewer"}),
        ))
        .await
        .unwrap();

    let resp = app
        .oneshot(req_get("/_api/auth/users/eve/roles", Some(&admin)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let roles: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["role"].as_str().unwrap())
        .collect();
    assert!(roles.contains(&"viewer"));
    assert!(roles.contains(&"editor"));
}

// ===========================================================================
// Self-service
// ===========================================================================

#[tokio::test]
async fn get_current_user_returns_username_and_roles() {
    let (_tmp, app, admin, _) = create_app();
    let resp = app
        .oneshot(req_get("/_api/auth/me", Some(&admin)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["username"], "admin_user");
    let roles: Vec<&str> = body["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert_eq!(roles, vec!["admin"]);
    // Admin's effective permissions include global admin.
    assert!(!body["permissions"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_my_permissions_for_viewer_is_read_only() {
    let (_tmp, app, _admin, viewer) = create_app();
    let resp = app
        .oneshot(req_get("/_api/auth/me/permissions", Some(&viewer)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let perms = body.as_array().unwrap();
    assert!(!perms.is_empty());
    // Viewer must not have admin or write permissions.
    for p in perms {
        let action = p["action"].as_str().unwrap();
        assert!(
            action != "admin" && action != "write",
            "viewer leaked elevated permission: {:?}",
            p
        );
    }
}

// ===========================================================================
// AuthZ boundaries — viewer is rejected, missing JWT is rejected.
// (Spot checks; the same code path runs in every admin endpoint.)
// ===========================================================================

#[tokio::test]
async fn admin_endpoints_reject_viewer() {
    let (_tmp, app, _admin, viewer) = create_app();
    for (method, uri) in [
        ("GET", "/_api/auth/roles"),
        ("GET", "/_api/auth/users"),
        ("GET", "/_api/auth/users/admin/roles"),
    ] {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("Authorization", bearer(&viewer))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{} {} should be 403 for viewer",
            method,
            uri
        );
    }
}

#[tokio::test]
async fn admin_endpoints_reject_missing_jwt() {
    let (_tmp, app, _admin, _viewer) = create_app();
    for (method, uri) in [
        ("GET", "/_api/auth/roles"),
        ("GET", "/_api/auth/users"),
        ("GET", "/_api/auth/me"),
    ] {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{} {} should be 401 without token",
            method,
            uri
        );
    }
}
