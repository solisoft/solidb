//! An import into a collection that has a foreign key, with its references
//! checked once the file is in (`?foreign_keys=defer`, card #947): children
//! may come before their parents, and the answer lists the orphans.

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

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&bytes).to_string();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

async fn call(
    app: &axum::Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::from(body.to_string()))
        .unwrap();
    send(app, request).await
}

/// `lines` imported as a JSONL file into `collection`.
async fn import(
    app: &axum::Router,
    token: &str,
    collection: &str,
    query: &str,
    lines: &str,
) -> (StatusCode, Value) {
    const BOUNDARY: &str = "fk-import-boundary";
    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"docs.jsonl\"\r\nContent-Type: application/x-ndjson\r\n\r\n{lines}\r\n--{BOUNDARY}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri(format!(
            "/_api/database/_system/collection/{collection}/import{query}"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::from(body))
        .unwrap();
    send(app, request).await
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_may_check_its_references_once_everything_is_in() {
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
    let admin = common::seed_user_token(&engine, "import_admin", &["admin"]);
    let base = "/_api/database/_system";
    for name in ["customers", "orders"] {
        let (status, body) = call(
            &app,
            &admin,
            "POST",
            &format!("{base}/collection"),
            json!({"name": name}),
        )
        .await;
        assert!(status.is_success(), "{status} {body}");
    }
    let (status, body) = call(
        &app,
        &admin,
        "POST",
        &format!("{base}/collection/orders/foreign-keys"),
        json!({"field": "customer", "references": "customers"}),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    let orders = "{\"_key\":\"o1\",\"customer\":\"alice\"}\n{\"_key\":\"o2\",\"customer\":\"bob\"}";
    let stored = || {
        engine
            .get_database("_system")
            .unwrap()
            .get_collection("orders")
            .unwrap()
            .count()
    };

    // Checked as they are written: the children come before their parents
    // and are refused.
    let (status, body) = import(&app, &admin, "orders", "", orders).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["count"].clone(), body["failed"].clone()),
        (json!(0), json!(2))
    );
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .contains("Foreign key violation"),
        "{body}"
    );
    assert_eq!(stored(), 0);

    // Deferred: they are stored, and the answer names them as orphans.
    let (status, body) = import(&app, &admin, "orders", "?foreign_keys=defer", orders).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        (body["count"].clone(), body["failed"].clone()),
        (json!(2), json!(0))
    );
    assert_eq!(body["orphans"][0]["field"], json!("customer"));
    assert_eq!(body["orphans"][0]["references"], json!("customers"));
    assert_eq!(body["orphans"][0]["count"], json!(2));
    assert_eq!(stored(), 2);

    // One parent arrives; a later deferred import leaves one orphan.
    let (status, body) = import(&app, &admin, "customers", "", "{\"_key\":\"alice\"}").await;
    assert_eq!(
        (status, body["count"].clone()),
        (StatusCode::OK, json!(1)),
        "{body}"
    );
    let (status, body) = import(
        &app,
        &admin,
        "orders",
        "?mode=upsert&foreign_keys=defer",
        "{\"_key\":\"o3\",\"customer\":\"alice\"}",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["orphans"][0]["count"], json!(1));
    assert_eq!(body["orphans"][0]["sample"][0]["_key"], json!("o2"));
    assert_eq!(stored(), 3);

    // Outside such an import the key is enforced as before.
    let (status, _) = call(
        &app,
        &admin,
        "POST",
        &format!("{base}/document/orders"),
        json!({"customer": "nobody"}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // A mode that does not exist is refused; a collection without a key
    // answers an empty list.
    let (status, _) = import(&app, &admin, "orders", "?foreign_keys=later", orders).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = import(
        &app,
        &admin,
        "customers",
        "?foreign_keys=defer",
        "{\"_key\":\"carol\"}",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["orphans"], json!([]));
}
