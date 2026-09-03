//! RBAC enforcement tests for admin-only endpoints.
//!
//! Covers SEC-126 follow-up fixes: a non-admin (viewer) JWT must be
//! rejected by `DELETE /_api/database/{name}` and `GET /_api/auth/api_keys`.
//! Cluster admin endpoints are exercised in `cluster_tests.rs`.

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

fn create_app() -> (TempDir, axum::Router, String, String) {
    let tmp_dir = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).expect("engine");
    engine.initialize().expect("initialize _system");
    let script_stats = Arc::new(ScriptStats::default());
    let router = create_router(
        engine.clone(),
        None,
        None,
        None,
        None,
        script_stats,
        None,
        None,
        0,
    );

    // Seed *after* `create_router`, which runs `AuthService::init`: that only
    // creates the default `admin` user while `_admins` is empty, so inserting
    // test principals first would silently suppress it.
    //
    // Real `_admins` rows, not just signed tokens: the auth middleware now
    // refuses a JWT whose subject is not a user, which is how deleting a user
    // revokes their outstanding tokens.
    let admin_token = common::seed_user_token(&engine, "admin_user", &["admin"]);
    let viewer_token = common::seed_user_token(&engine, "viewer_user", &["viewer"]);

    (tmp_dir, router, admin_token, viewer_token)
}

fn bearer(token: &str) -> String {
    format!("Bearer {}", token)
}

#[tokio::test]
async fn delete_database_rejects_viewer() {
    let (_tmp, app, admin_token, viewer_token) = create_app();

    // Admin creates the database.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", bearer(&admin_token))
                .body(Body::from(json!({"name": "victim_db"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Viewer attempts DELETE — must be forbidden.
    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/_api/database/victim_db")
                .header("Authorization", bearer(&viewer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn list_api_keys_rejects_viewer() {
    let (_tmp, app, _admin_token, viewer_token) = create_app();

    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/_api/auth/api-keys")
                .header("Authorization", bearer(&viewer_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn delete_database_allows_admin() {
    let (_tmp, app, admin_token, _viewer_token) = create_app();

    // Create the DB.
    let _ = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", bearer(&admin_token))
                .body(Body::from(json!({"name": "ok_to_delete"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/_api/database/ok_to_delete")
                .header("Authorization", bearer(&admin_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}
