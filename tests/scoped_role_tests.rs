//! A role assigned with a `database` grants that role's actions on that one
//! database, nowhere else.

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

struct App {
    router: axum::Router,
    admin: String,
    alice: String,
    /// Unique per test: the roles cache is process-wide and keyed by username.
    user: String,
    _tmp: TempDir,
}

fn app() -> App {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    for db in ["tenant_a", "tenant_b"] {
        engine.create_database(db.to_string()).unwrap();
        engine
            .get_database(db)
            .unwrap()
            .create_collection("items".to_string(), None)
            .unwrap();
    }
    let router = create_router(
        engine.clone(),
        None,
        None,
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    // Seeded after `create_router` (see role_handlers_tests).
    let admin = common::seed_user_token(&engine, "root_user", &["admin"]);
    let user = format!("alice_{}", uuid::Uuid::new_v4().simple());
    let alice = common::seed_user_token(&engine, &user, &[]);
    App {
        user,
        router,
        admin,
        alice,
        _tmp: tmp,
    }
}

async fn call(
    app: &App,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {}", token));
    let body = match body {
        Some(b) => {
            req = req.header(header::CONTENT_TYPE, "application/json");
            Body::from(b.to_string())
        }
        None => Body::empty(),
    };
    let resp = app
        .router
        .clone()
        .oneshot(req.body(body).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn assign(app: &App, role: &str, database: Option<&str>) -> StatusCode {
    let mut body = json!({"role": role});
    if let Some(d) = database {
        body["database"] = json!(d);
    }
    call(
        app,
        &app.admin,
        "POST",
        &format!("/_api/auth/users/{}/roles", app.user),
        Some(body),
    )
    .await
    .0
}

async fn can_read(app: &App, db: &str) -> bool {
    call(
        app,
        &app.alice,
        "GET",
        &format!("/_api/database/{}/collection", db),
        None,
    )
    .await
    .0
    .is_success()
}

async fn can_write(app: &App, db: &str) -> bool {
    call(
        app,
        &app.alice,
        "POST",
        &format!("/_api/database/{}/document/items", db),
        Some(json!({"v": 1})),
    )
    .await
    .0
    .is_success()
}

#[tokio::test]
async fn an_editor_limited_to_one_database_works_there_and_nowhere_else() {
    let app = app();
    assert!(!can_read(&app, "tenant_a").await, "no role, no access");

    assert_eq!(
        assign(&app, "editor", Some("tenant_a")).await,
        StatusCode::CREATED
    );

    assert!(can_read(&app, "tenant_a").await);
    assert!(can_write(&app, "tenant_a").await);
    assert!(!can_read(&app, "tenant_b").await, "other database readable");
    assert!(
        !can_write(&app, "tenant_b").await,
        "other database writable"
    );
    assert!(!can_read(&app, "_system").await, "_system readable");

    // Not a global principal either: instance-level operations stay closed.
    let (status, _) = call(
        &app,
        &app.alice,
        "POST",
        "/_api/database",
        Some(json!({"name": "mine"})),
    )
    .await;
    assert!(status.is_client_error(), "created a database: {}", status);
    let (status, _) = call(&app, &app.alice, "GET", "/_api/auth/users", None).await;
    assert!(status.is_client_error(), "listed users: {}", status);
}

#[tokio::test]
async fn a_limited_viewer_reads_but_does_not_write() {
    let app = app();
    assert_eq!(
        assign(&app, "viewer", Some("tenant_a")).await,
        StatusCode::CREATED
    );
    assert!(can_read(&app, "tenant_a").await);
    assert!(!can_write(&app, "tenant_a").await);
    assert!(!can_read(&app, "tenant_b").await);
}

#[tokio::test]
async fn limited_and_global_assignments_combine_and_revoking_removes_the_limit() {
    let app = app();
    assert_eq!(assign(&app, "viewer", None).await, StatusCode::CREATED);
    assert_eq!(
        assign(&app, "editor", Some("tenant_a")).await,
        StatusCode::CREATED
    );

    // Global read everywhere, write only in tenant_a.
    assert!(can_read(&app, "tenant_b").await);
    assert!(can_write(&app, "tenant_a").await);
    assert!(!can_write(&app, "tenant_b").await);

    let (status, _) = call(
        &app,
        &app.admin,
        "DELETE",
        &format!(
            "/_api/auth/users/{}/roles/editor?database=tenant_a",
            app.user
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(
        !can_write(&app, "tenant_a").await,
        "limit outlived its revoke"
    );
    assert!(
        can_read(&app, "tenant_a").await,
        "the global viewer remains"
    );
}

#[tokio::test]
async fn a_limited_assignment_needs_a_real_database_and_role_names_cannot_forge_one() {
    let app = app();
    assert_eq!(
        assign(&app, "editor", Some("nope")).await,
        StatusCode::BAD_REQUEST
    );

    let (status, _) = call(
        &app,
        &app.admin,
        "POST",
        "/_api/auth/roles",
        Some(json!({"name": "evil@tenant_a", "permissions": [{"action": "read", "scope": "global"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn editing_a_role_takes_effect_for_its_limited_assignments() {
    let app = app();
    let (status, _) = call(
        &app,
        &app.admin,
        "POST",
        "/_api/auth/roles",
        Some(json!({"name": "auditor", "permissions": [{"action": "read", "scope": "global"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        assign(&app, "auditor", Some("tenant_a")).await,
        StatusCode::CREATED
    );
    assert!(can_read(&app, "tenant_a").await);
    assert!(!can_write(&app, "tenant_a").await);

    // Grant write; the cached permissions of the limited assignment must follow.
    let (status, body) = call(
        &app,
        &app.admin,
        "PUT",
        "/_api/auth/roles/auditor",
        Some(json!({"permissions": [{"action": "write", "scope": "global"}]})),
    )
    .await;
    assert!(status.is_success(), "{} {}", status, body);
    assert!(
        can_write(&app, "tenant_a").await,
        "cache not invalidated for role@db"
    );
    assert!(!can_write(&app, "tenant_b").await);
}

#[tokio::test]
async fn a_custom_role_grants_its_permissions_when_assigned_globally() {
    let app = app();
    let (status, _) = call(
        &app,
        &app.admin,
        "POST",
        "/_api/auth/roles",
        Some(json!({"name": "auditor", "permissions": [{"action": "read", "scope": "global"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(assign(&app, "auditor", None).await, StatusCode::CREATED);
    // A stored role used to fail to load (its name lives in `_key`, which the
    // document body omits), so a custom role granted nothing at all.
    assert!(can_read(&app, "tenant_a").await);
    assert!(can_read(&app, "tenant_b").await);
    assert!(!can_write(&app, "tenant_a").await);
}
