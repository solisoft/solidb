//! `PUT …/document/{c}/{key}?replace=true` swaps the document and says so in
//! `x-replace-applied`; a plain PUT merges and says it did not. A node forwarding
//! a sharded REPLACE relies on that header to detect a peer that predates it.

use axum::{
    body::Body,
    http::{header, Request},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

#[tokio::test]
async fn replace_query_param_replaces_and_is_acknowledged() {
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

    let put = |query: &'static str, body: Value| {
        let router = router.clone();
        let token = token.clone();
        async move {
            let resp = router
                .oneshot(
                    Request::builder()
                        .method("PUT")
                        .uri(format!("/_api/database/d/document/c/k{}", query))
                        .header(header::AUTHORIZATION, format!("Bearer {}", token))
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let applied = resp
                .headers()
                .get("x-replace-applied")
                .map(|v| v.to_str().unwrap().to_string());
            let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
                .await
                .unwrap();
            (applied, serde_json::from_slice::<Value>(&bytes).unwrap())
        }
    };

    let (applied, merged) = put("", json!({"b": 20})).await;
    assert_eq!(applied.as_deref(), Some("false"));
    assert_eq!(merged["a"], 1, "a plain PUT merges: {}", merged);

    let (applied, replaced) = put("?replace=true", json!({"c": 3})).await;
    assert_eq!(applied.as_deref(), Some("true"));
    assert_eq!(replaced["c"], 3);
    assert!(
        replaced.get("a").is_none(),
        "replace drops fields: {}",
        replaced
    );
}
