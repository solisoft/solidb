//! A database-scoped admin may write its own database's `_jobs`, the
//! admin-write tier (`storage::protected`). This is what lets a Soli app run
//! its background jobs with an account limited to its own database, instead of
//! a global admin. The credential tier (`_env`) and other databases stay out
//! of reach, and a plain writer (editor) is still refused.
//!
//! The driver already resolved `can_admin` per database
//! (`query_principal(db)`); these tests pin the HTTP document API to the same
//! rule. The queue worker's own writes (`_jobs` claim/update) run as `_system`
//! and are unaffected.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::json;
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

struct Fixture {
    _tmp: TempDir,
    router: axum::Router,
    /// Global admin.
    root: String,
    /// `admin` limited to database `app`.
    app_admin: String,
    /// `editor` limited to database `app`.
    app_editor: String,
}

fn fixture() -> Fixture {
    let tmp = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).expect("engine");
    engine.initialize().expect("initialize _system");
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
    // Seed after create_router, which only creates the default admin while
    // `_admins` is empty (see rbac_admin_endpoints_tests).
    let root = common::seed_user_token(&engine, "root", &["admin"]);
    let app_admin = common::seed_user_token(&engine, "app_admin", &["admin@app"]);
    let app_editor = common::seed_user_token(&engine, "app_editor", &["editor@app"]);
    Fixture {
        _tmp: tmp,
        router,
        root,
        app_admin,
        app_editor,
    }
}

async fn send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> StatusCode {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {}", token));
    let body = match body {
        Some(v) => {
            builder = builder.header("Content-Type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    router
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
        .status()
}

/// Create `app` and `other` as the global admin.
async fn create_databases(router: &axum::Router, root: &str) {
    for name in ["app", "other"] {
        let status = send(
            router,
            "POST",
            "/_api/database",
            root,
            Some(json!({ "name": name })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "create database {}", name);
    }
}

/// SDBQL does not create a collection it writes to; the document API does.
async fn create_collections(router: &axum::Router, root: &str, paths: &[&str]) {
    for path in paths {
        let uri = format!("/_api/database/{}", path);
        let status = send(router, "POST", &uri, root, Some(json!({ "seed": true }))).await;
        assert_eq!(status, StatusCode::OK, "seed {}", path);
    }
}

#[tokio::test]
async fn database_scoped_admin_may_write_its_own_jobs() {
    let f = fixture();
    create_databases(&f.router, &f.root).await;

    // The scoped admin enqueues into its own database's `_jobs`.
    let status = send(
        &f.router,
        "POST",
        "/_api/database/app/document/_jobs",
        &f.app_admin,
        Some(json!({ "state": "pending", "marker": "by-app-admin" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "scoped admin writes app/_jobs");

    // And the row is actually there.
    let got = send(
        &f.router,
        "POST",
        "/_api/database/app/cursor",
        &f.root,
        Some(json!({ "query": "FOR j IN _jobs FILTER j.marker == \"by-app-admin\" RETURN 1" })),
    )
    .await;
    assert_eq!(got, StatusCode::OK);
}

#[tokio::test]
async fn global_admin_still_writes_jobs() {
    let f = fixture();
    create_databases(&f.router, &f.root).await;
    let status = send(
        &f.router,
        "POST",
        "/_api/database/app/document/_jobs",
        &f.root,
        Some(json!({ "state": "pending" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_scoped_admin_cannot_reach_another_databases_jobs() {
    let f = fixture();
    create_databases(&f.router, &f.root).await;
    // admin@app is nothing on `other`.
    let status = send(
        &f.router,
        "POST",
        "/_api/database/other/document/_jobs",
        &f.app_admin,
        Some(json!({ "state": "pending" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_editor_is_still_refused_jobs() {
    let f = fixture();
    create_databases(&f.router, &f.root).await;
    let status = send(
        &f.router,
        "POST",
        "/_api/database/app/document/_jobs",
        &f.app_editor,
        Some(json!({ "state": "pending" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "editor must not write _jobs");

    // ...while an ordinary write in the same database is allowed, so the
    // refusal is about the tier, not the database.
    let ok = send(
        &f.router,
        "POST",
        "/_api/database/app/document/orders",
        &f.app_editor,
        Some(json!({ "item": "x" })),
    )
    .await;
    assert_eq!(ok, StatusCode::OK, "editor writes an ordinary collection");
}

#[tokio::test]
async fn the_credential_tier_stays_closed_to_a_scoped_admin() {
    let f = fixture();
    create_databases(&f.router, &f.root).await;
    // `_env` is the protected tier: never writable by name, admin or not.
    let status = send(
        &f.router,
        "POST",
        "/_api/database/app/document/_env",
        &f.app_admin,
        Some(json!({ "_key": "SECRET", "value": "x" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "_env stays closed");
}

/// The same rule through SDBQL (card #928): the document API let a
/// database's admin write its `_jobs` while `INSERT … INTO _jobs` on
/// `/cursor` was still refused.
#[tokio::test]
async fn a_scoped_admin_writes_its_jobs_through_sdbql() {
    let f = fixture();
    create_databases(&f.router, &f.root).await;
    create_collections(
        &f.router,
        &f.root,
        &["app/document/_jobs", "other/document/_jobs"],
    )
    .await;
    let insert = json!({ "query": "INSERT { state: \"pending\", marker: \"q\" } INTO _jobs" });

    let status = send(
        &f.router,
        "POST",
        "/_api/database/app/cursor",
        &f.app_admin,
        Some(insert.clone()),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "scoped admin inserts into app/_jobs"
    );

    // A query that reads then writes: the long-running branch of /cursor.
    let status = send(
        &f.router,
        "POST",
        "/_api/database/app/cursor",
        &f.app_admin,
        Some(json!({
            "query": "FOR j IN _jobs FILTER j.marker == \"q\" UPDATE j WITH { state: \"done\" } IN _jobs"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "scoped admin updates app/_jobs");

    // Not another database's, and not for a writer who is no admin.
    let status = send(
        &f.router,
        "POST",
        "/_api/database/other/cursor",
        &f.app_admin,
        Some(insert.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "other/_jobs stays closed");
    let status = send(
        &f.router,
        "POST",
        "/_api/database/app/cursor",
        &f.app_editor,
        Some(insert),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "an editor's query is refused"
    );
}

/// What the per-database admin gains through SDBQL is `_jobs` and nothing
/// else: the collections the server executes stay closed, and so does a
/// collection's row policy, which only a global admin may change.
#[tokio::test]
async fn a_scoped_admin_gains_only_jobs_through_sdbql() {
    let f = fixture();
    create_databases(&f.router, &f.root).await;
    create_collections(&f.router, &f.root, &["app/document/things"]).await;

    for query in [
        "INSERT { a: 1 } INTO _scripts",
        "INSERT { a: 1 } INTO _env",
        "INSERT { a: 1 } INTO things RETURN ROW_POLICY(\"things\", \"true\")",
    ] {
        let status = send(
            &f.router,
            "POST",
            "/_api/database/app/cursor",
            &f.app_admin,
            Some(json!({ "query": query })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{}", query);
    }
}
