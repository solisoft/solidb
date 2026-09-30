//! `PUT …/document/{c}/{key}?replace=true` swaps the document and says so in
//! `x-replace-applied`; a plain PUT merges and says it did not. A node forwarding
//! a sharded REPLACE relies on that header to detect a peer that predates it.
//! A replace honours `If-Match` like an update, and is refused where it would
//! silently merge instead (inside a transaction).

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

struct Reply {
    status: StatusCode,
    applied: Option<String>,
    body: Value,
}

struct App {
    router: axum::Router,
    token: String,
    engine: StorageEngine,
    _tmp: TempDir,
}

fn app() -> App {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.create_database("d".to_string()).unwrap();
    let db = engine.get_database("d").unwrap();
    db.create_collection("c".to_string(), None).unwrap();
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
    let token = common::seed_user_token(&engine, "root_user", &["admin"]);
    db.get_collection("c")
        .unwrap()
        .insert(json!({"_key": "k", "a": 1, "b": 2}))
        .unwrap();
    App {
        router,
        token,
        engine,
        _tmp: tmp,
    }
}

async fn put(app: &App, query: &str, body: Value, extra: &[(&str, &str)]) -> Reply {
    let mut req = Request::builder()
        .method("PUT")
        .uri(format!("/_api/database/d/document/c/k{}", query))
        .header(header::AUTHORIZATION, format!("Bearer {}", app.token))
        .header(header::CONTENT_TYPE, "application/json");
    for (name, value) in extra {
        req = req.header(*name, *value);
    }
    let resp = app
        .router
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let applied = resp
        .headers()
        .get("x-replace-applied")
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    Reply {
        status,
        applied,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

fn stored(app: &App) -> Value {
    app.engine
        .get_database("d")
        .unwrap()
        .get_collection("c")
        .unwrap()
        .get("k")
        .unwrap()
        .to_value()
}

#[tokio::test]
async fn replace_query_param_replaces_and_is_acknowledged() {
    let app = app();

    let merged = put(&app, "", json!({"b": 20}), &[]).await;
    assert_eq!(merged.applied.as_deref(), Some("false"));
    assert_eq!(merged.body["a"], 1, "a plain PUT merges: {}", merged.body);

    let replaced = put(&app, "?replace=true", json!({"c": 3}), &[]).await;
    assert_eq!(replaced.applied.as_deref(), Some("true"));
    assert_eq!(replaced.body["c"], 3);
    assert!(
        replaced.body.get("a").is_none(),
        "replace drops fields: {}",
        replaced.body
    );
}

#[tokio::test]
async fn replace_honours_if_match() {
    let app = app();
    let current = stored(&app)["_rev"].as_str().unwrap().to_string();

    let stale = put(
        &app,
        "?replace=true",
        json!({"c": 3}),
        &[("If-Match", "\"not-the-current-rev\"")],
    )
    .await;
    assert_eq!(stale.status, StatusCode::CONFLICT, "{}", stale.body);
    assert_eq!(stored(&app)["a"], 1, "a stale replace must not write");

    let fresh = put(
        &app,
        "?replace=true",
        json!({"c": 3}),
        &[("If-Match", current.as_str())],
    )
    .await;
    assert_eq!(fresh.status, StatusCode::OK, "{}", fresh.body);
    assert!(stored(&app).get("a").is_none());
}

#[tokio::test]
async fn replace_of_a_missing_document_is_not_found() {
    let app = app();
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/_api/database/d/document/c/nope?replace=true")
                .header(header::AUTHORIZATION, format!("Bearer {}", app.token))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"c": 3}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn replace_inside_a_transaction_is_refused_rather_than_merged() {
    let app = app();
    let tx = app
        .engine
        .transaction_manager()
        .unwrap()
        .begin(solidb::transaction::IsolationLevel::ReadCommitted)
        .unwrap()
        .to_string();
    let reply = put(
        &app,
        "?replace=true",
        json!({"c": 3}),
        &[("X-Transaction-ID", tx.as_str())],
    )
    .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.body);
    assert!(
        reply.body.to_string().contains("transaction"),
        "{}",
        reply.body
    );
}
