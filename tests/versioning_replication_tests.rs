//! Document versioning across nodes (card #998): turning it on or off is
//! logged for the peers, and a replica records a replicated write at the
//! instant its origin made it, so `AS OF` answers the same on every node.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::collection::versioning::{at_origin, origin_micros};
use solidb::storage::StorageEngine;
use solidb::sync::log::SyncLog;
use solidb::sync::protocol::Operation;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

async fn call(app: &axum::Router, token: &str, method: &str, uri: &str, body: Value) -> StatusCode {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", token))
        .body(Body::from(body.to_string()))
        .unwrap();
    app.clone().oneshot(request).await.unwrap().status()
}

fn enabled(entry_data: Option<&[u8]>) -> bool {
    let v: Value = serde_json::from_slice(entry_data.unwrap()).unwrap();
    v["enabled"].as_bool().unwrap()
}

#[tokio::test]
async fn versioning_is_logged_and_applied_by_a_peer() {
    let tmp = TempDir::new().unwrap();
    let origin = StorageEngine::new(tmp.path().join("a").to_str().unwrap()).unwrap();
    origin.initialize().unwrap();
    let log = Arc::new(
        SyncLog::new(
            "n1".to_string(),
            tmp.path().join("log").to_str().unwrap(),
            64,
        )
        .unwrap(),
    );
    let app = create_router(
        origin.clone(),
        None,
        Some(log.clone()),
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    let token = common::seed_user_token(&origin, "versioning_admin", &["admin"]);
    let base = "/_api/database/_system";

    // Created versioned: the flag follows the creation in the log.
    let status = call(
        &app,
        &token,
        "POST",
        &format!("{base}/collection"),
        json!({"name": "orders", "versioning": true}),
    )
    .await;
    assert!(status.is_success(), "{status}");
    let born: Vec<Operation> = log
        .get_entries_after(0, 100)
        .into_iter()
        .filter(|e| e.collection == "orders")
        .map(|e| e.operation)
        .collect();
    assert_eq!(
        born,
        [Operation::CreateCollection, Operation::SetVersioning]
    );

    // Turned off, then on again, through the properties.
    let props = format!("{base}/collection/orders/properties");
    assert_eq!(
        call(&app, &token, "PUT", &props, json!({"versioning": false})).await,
        StatusCode::OK
    );
    assert_eq!(
        call(&app, &token, "PUT", &props, json!({"versioning": true})).await,
        StatusCode::OK
    );
    let flags: Vec<bool> = log
        .get_entries_after(0, 100)
        .into_iter()
        .filter(|e| e.collection == "orders" && e.operation == Operation::SetVersioning)
        .map(|e| enabled(e.data.as_deref()))
        .collect();
    assert_eq!(flags, [true, false, true]);

    // A peer with the collection applies them in order.
    let peer = StorageEngine::new(tmp.path().join("b").to_str().unwrap()).unwrap();
    peer.initialize().unwrap();
    let db = peer.get_database("_system").unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    let orders = db.get_collection("orders").unwrap();
    orders.insert(json!({"_key": "before", "n": 1})).unwrap();
    let mut seen = Vec::new();
    for entry in log.get_entries_after(0, 100) {
        if entry.collection == "orders" && entry.operation == Operation::SetVersioning {
            orders.apply_versioning(entry.data.as_deref()).unwrap();
            seen.push(orders.is_versioned());
        }
    }
    assert_eq!(seen, [true, false, true]);
    // Each enabling there recorded the peer's own documents as a baseline.
    assert_eq!(orders.doc_history("before").len(), 2);
    assert!(orders.apply_versioning(Some(b"{}")).is_err());
}

/// A replicated write and delete are versioned at the origin's time, not
/// at the instant the replica applied them.
#[test]
fn a_replica_versions_writes_at_the_origin_time() {
    let tmp = TempDir::new().unwrap();
    let replica = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    replica
        .create_collection("orders".to_string(), None)
        .unwrap();
    let orders = replica.get_collection("orders").unwrap();
    orders.enable_versioning().unwrap();

    // HLC milliseconds, the logical counter below them.
    let written = origin_micros(1_700_000_000_000, 3);
    assert_eq!(written, 1_700_000_000_000_003);
    assert!(origin_micros(1_700_000_000_000, 4) > written);
    assert!(origin_micros(1_700_000_000_001, 0) > origin_micros(1_700_000_000_000, 5_000));

    at_origin(HashMap::from([("o1".to_string(), written)]), || {
        orders
            .upsert_replicated(vec![("o1".to_string(), json!({"_key": "o1", "v": 1}))])
            .unwrap();
    });
    let history = orders.doc_history("o1");
    assert_eq!(history[0]["ts_micros"], json!(written));
    assert!(orders.get_as_of("o1", written - 1).unwrap().is_none());
    assert_eq!(
        orders.get_as_of("o1", written).unwrap().unwrap()["v"],
        json!(1)
    );

    let removed = origin_micros(1_700_000_000_500, 0);
    at_origin(HashMap::from([("o1".to_string(), removed)]), || {
        orders.delete_batch(vec!["o1".to_string()]).unwrap();
    });
    assert_eq!(
        orders.get_as_of("o1", removed - 1).unwrap().unwrap()["v"],
        json!(1)
    );
    assert!(orders.get_as_of("o1", removed).unwrap().is_none());

    // Outside `at_origin`, this node's clock again.
    orders.insert(json!({"_key": "local", "v": 1})).unwrap();
    let local = orders.doc_history("local")[0]["ts_micros"]
        .as_u64()
        .unwrap();
    assert!(local > removed);
}
