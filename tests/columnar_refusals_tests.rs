//! What cannot be declared on a columnar collection, and what the server
//! answers (card #897): a trigger and a foreign key work on documents. Each
//! was refused as "collection not found", which the collection is not.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

async fn call(
    app: &axum::Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, String) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .body(match body {
            Some(b) => Body::from(b.to_string()),
            None => Body::empty(),
        })
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn triggers_and_foreign_keys_on_a_columnar_collection_say_why_they_are_refused() {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    let app = create_router(
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
    let token = common::seed_user_token(&engine, "columnar_admin", &["admin"]);
    let t = token.as_str();
    let base = "/_api/database/_system";
    let (status, body) = call(
        &app,
        t,
        "POST",
        &format!("{base}/columnar"),
        Some(json!({"name": "metrics", "columns": [
            {"name": "device_id", "type": "string"}, {"name": "v", "type": "float64"}]})),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    for name in ["devices", "readings"] {
        let (status, body) = call(
            &app,
            t,
            "POST",
            &format!("{base}/collection"),
            Some(json!({ "name": name })),
        )
        .await;
        assert!(status.is_success(), "{status} {body}");
    }

    let columnar = "'metrics' is a columnar collection";

    // A trigger.
    let trigger = |collection: &str| {
        json!({"name": "t", "collection": collection, "events": ["insert"],
               "webhook_url": "https://203.0.113.5/hook"})
    };
    let (status, body) = call(
        &app,
        t,
        "POST",
        &format!("{base}/triggers"),
        Some(trigger("metrics")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.contains(columnar) && body.contains("sends no event"),
        "{body}"
    );
    // A collection that is not there is still reported as such.
    let (status, body) = call(
        &app,
        t,
        "POST",
        &format!("{base}/triggers"),
        Some(trigger("nope")),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains("does not exist"), "{body}");

    // A foreign key from it, and towards it.
    let (status, body) = call(
        &app,
        t,
        "POST",
        &format!("{base}/collection/metrics/foreign-keys"),
        Some(json!({"field": "device_id", "references": "devices"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.contains(columnar) && body.contains("foreign key"),
        "{body}"
    );
    let (status, body) = call(
        &app,
        t,
        "POST",
        &format!("{base}/collection/readings/foreign-keys"),
        Some(json!({"field": "m", "references": "metrics"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains(columnar), "{body}");
    let (status, body) = call(
        &app,
        t,
        "GET",
        &format!("{base}/collection/metrics/foreign-keys"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.contains(columnar), "{body}");

    // Between documents a key is declared as before, and a missing parent
    // is still "not found".
    let (status, body) = call(
        &app,
        t,
        "POST",
        &format!("{base}/collection/readings/foreign-keys"),
        Some(json!({"field": "device", "references": "devices"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let (status, body) = call(
        &app,
        t,
        "POST",
        &format!("{base}/collection/readings/foreign-keys"),
        Some(json!({"field": "x", "references": "nowhere"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}
