//! Foreign keys over HTTP: the endpoints, and the constraint reached
//! through the document API and SDBQL.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use solidb::sync::log::SyncLog;
use solidb::sync::protocol::Operation;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

fn create_test_app() -> (axum::Router, TempDir, String) {
    let (router, tmp_dir, token, _) = create_app(false);
    (router, tmp_dir, token)
}

/// The app, with an active replication log when `replicating`.
fn create_app(replicating: bool) -> (axum::Router, TempDir, String, Option<Arc<SyncLog>>) {
    let tmp_dir = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp_dir.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    let log = replicating.then(|| {
        let dir = tmp_dir.path().join("log");
        Arc::new(SyncLog::new("n1".to_string(), dir.to_str().unwrap(), 64).unwrap())
    });
    let router = create_router(
        engine.clone(),
        None,
        log.clone(),
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let token = common::seed_user_token(&engine, "test_admin", &["admin"]);
    (router, tmp_dir, token, log)
}

async fn call(
    app: &axum::Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token));
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let response = app
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn query(app: &axum::Router, token: &str, text: &str) -> (StatusCode, Value) {
    call(
        app,
        token,
        "POST",
        "/_api/database/shop/cursor",
        Some(json!({ "query": text })),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn foreign_keys_over_http() {
    let (app, _tmp, token) = create_test_app();
    let t = token.as_str();
    let keys = "/_api/database/shop/collection/orders/foreign-keys";

    call(
        &app,
        t,
        "POST",
        "/_api/database",
        Some(json!({"name": "shop"})),
    )
    .await;
    for name in ["customers", "orders"] {
        let (status, _) = call(
            &app,
            t,
            "POST",
            "/_api/database/shop/collection",
            Some(json!({ "name": name })),
        )
        .await;
        assert!(status.is_success());
    }
    let docs = "/_api/database/shop/document";
    call(
        &app,
        t,
        "POST",
        &format!("{docs}/customers"),
        Some(json!({"_key": "alice"})),
    )
    .await;
    call(
        &app,
        t,
        "POST",
        &format!("{docs}/orders"),
        Some(json!({"_key": "ghost", "customer": "nobody"})),
    )
    .await;

    // What declaring the key would be refused for, then the refusal.
    let spec = json!({"field": "customer", "references": "customers", "on_delete": "cascade"});
    let (status, orphans) = call(
        &app,
        t,
        "POST",
        &format!("{keys}/check"),
        Some(spec.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(orphans["count"], json!(1));
    assert_eq!(orphans["sample"][0]["_key"], json!("ghost"));
    let (status, refused) = call(&app, t, "POST", keys, Some(spec.clone())).await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");

    call(&app, t, "DELETE", &format!("{docs}/orders/ghost"), None).await;
    let (status, key) = call(&app, t, "POST", keys, Some(spec)).await;
    assert_eq!(status, StatusCode::CREATED, "{key}");
    assert_eq!(
        key,
        json!({"name": "fk_customer", "field": "customer", "references": "customers",
               "on_delete": "cascade", "index": "fk_customer"})
    );

    let (_, listed) = call(&app, t, "GET", keys, None).await;
    assert_eq!(listed["foreign_keys"][0]["name"], json!("fk_customer"));
    let (_, parent) = call(
        &app,
        t,
        "GET",
        "/_api/database/shop/collection/customers/foreign-keys",
        None,
    )
    .await;
    assert_eq!(parent["referenced_by"][0]["collection"], json!("orders"));
    assert_eq!(parent["referenced_by"][0]["field"], json!("customer"));

    // The document API and SDBQL both meet it.
    let (status, body) = call(
        &app,
        t,
        "POST",
        &format!("{docs}/orders"),
        Some(json!({"_key": "o0", "customer": "nobody"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) = query(
        &app,
        t,
        "INSERT {_key: 'o1', customer: 'nobody'} INTO orders",
    )
    .await;
    assert!(!status.is_success(), "{body}");
    assert!(body.to_string().contains("Foreign key violation"), "{body}");
    let (status, body) = query(
        &app,
        t,
        "FOR i IN 1..3 INSERT {_key: CONCAT('o', i), customer: 'alice'} INTO orders",
    )
    .await;
    assert!(status.is_success(), "{body}");
    let (status, body) = query(
        &app,
        t,
        "FOR o IN orders FILTER o._key == 'o1' UPDATE o WITH {customer: 'nobody'} IN orders",
    )
    .await;
    assert!(!status.is_success(), "{body}");

    // Nor can the index the key stands on.
    let (status, body) = call(
        &app,
        t,
        "DELETE",
        "/_api/database/shop/index/orders/fk_customer",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    // The collection that is referred to cannot be dropped.
    let (status, _) = call(
        &app,
        t,
        "DELETE",
        "/_api/database/shop/collection/customers",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // A delete through SDBQL cascades.
    let (status, body) = query(&app, t, "REMOVE 'alice' IN customers").await;
    assert!(status.is_success(), "{body}");
    let (_, left) = query(&app, t, "RETURN LENGTH(FOR o IN orders RETURN 1)").await;
    assert_eq!(left["result"], json!([0]), "{left}");

    let (status, _) = call(&app, t, "DELETE", &format!("{keys}/fk_customer"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, listed) = call(&app, t, "GET", keys, None).await;
    assert_eq!(listed["foreign_keys"], json!([]));
}

/// On a node that replicates, the log holds the key's definition and every
/// write a delete cascades into, after the delete that caused them.
#[tokio::test(flavor = "multi_thread")]
async fn a_cascade_is_in_the_replication_log() {
    let (app, _tmp, token, log) = create_app(true);
    let log = log.unwrap();
    let t = token.as_str();
    call(
        &app,
        t,
        "POST",
        "/_api/database",
        Some(json!({"name": "shop"})),
    )
    .await;
    for name in ["customers", "orders", "notes"] {
        call(
            &app,
            t,
            "POST",
            "/_api/database/shop/collection",
            Some(json!({ "name": name })),
        )
        .await;
    }
    let docs = "/_api/database/shop/document";
    call(
        &app,
        t,
        "POST",
        &format!("{docs}/customers"),
        Some(json!({"_key": "ann"})),
    )
    .await;
    for (collection, field, on_delete) in [
        ("orders", "customer", "cascade"),
        ("notes", "about", "set_null"),
    ] {
        let (status, body) = call(
            &app,
            t,
            "POST",
            &format!("/_api/database/shop/collection/{collection}/foreign-keys"),
            Some(json!({"field": field, "references": "customers", "on_delete": on_delete})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
    }
    call(
        &app,
        t,
        "POST",
        &format!("{docs}/orders"),
        Some(json!({"_key": "o1", "customer": "ann"})),
    )
    .await;
    call(
        &app,
        t,
        "POST",
        &format!("{docs}/notes"),
        Some(json!({"_key": "n1", "about": "ann"})),
    )
    .await;

    let before = log.current_sequence();
    let (status, body) = call(&app, t, "DELETE", &format!("{docs}/customers/ann"), None).await;
    assert!(status.is_success(), "{body}");

    let all = log.get_entries_after(0, 1000);
    let definitions: Vec<_> = all
        .iter()
        .filter(|e| e.operation == Operation::CreateForeignKey)
        .map(|e| (e.collection.as_str(), e.key.as_str()))
        .collect();
    assert_eq!(
        definitions,
        [("orders", "fk_customer"), ("notes", "fk_about")]
    );

    let caused: Vec<_> = log
        .get_entries_after(before, 1000)
        .into_iter()
        .filter(|e| e.database == "shop")
        .collect();
    let mut seen: Vec<_> = caused
        .iter()
        .map(|e| (e.collection.clone(), e.key.clone(), e.operation))
        .collect();
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        seen,
        [
            (
                "customers".to_string(),
                "ann".to_string(),
                Operation::Delete
            ),
            ("notes".to_string(), "n1".to_string(), Operation::Update),
            ("orders".to_string(), "o1".to_string(), Operation::Delete),
        ]
    );
    let note = caused.iter().find(|e| e.collection == "notes").unwrap();
    let stored: Value = serde_json::from_slice(note.data.as_ref().unwrap()).unwrap();
    assert_eq!(stored["about"], json!(null));
    assert_eq!(stored["_key"], json!("n1"));

    let (status, _) = call(
        &app,
        t,
        "DELETE",
        "/_api/database/shop/collection/orders/foreign-keys/fk_customer",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(log
        .get_entries_after(0, 1000)
        .iter()
        .any(|e| e.operation == Operation::DropForeignKey && e.key == "fk_customer"));
}
