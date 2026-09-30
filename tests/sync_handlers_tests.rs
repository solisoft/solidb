//! Coverage for `src/server/handlers/sync.rs` — offline-first sync endpoints
//! (`/_api/sync/session`, `/pull`, `/push`, `/ack`, `/conflicts`, `/resolve`).
//! See COV-003.
//!
//! `create_app` configures a cluster keyfile so the HMAC-signed session-id
//! path is exercised (otherwise `verify_session_id` no-ops).

use axum::{
    body::Body,
    http::{header, Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::cluster::ClusterConfig;
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use solidb::sync::log::SyncLog;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

const TEST_CLUSTER_SECRET: &str = "sync-handlers-cluster-secret";

fn create_app() -> (TempDir, axum::Router, String) {
    let tmp_dir = TempDir::new().expect("temp dir");
    let cluster_cfg = ClusterConfig {
        node_id: "test-node".to_string(),
        peers: vec![],
        replication_port: 6746,
        keyfile: Some(TEST_CLUSTER_SECRET.to_string()),
    };
    let engine = StorageEngine::with_cluster_config(tmp_dir.path().to_str().unwrap(), cluster_cfg)
        .expect("engine");
    engine.initialize().expect("initialize _system");

    // pull_changes errors out without a replication log, so plumb one in.
    let sync_log = SyncLog::new(
        "test-node".to_string(),
        tmp_dir.path().to_str().unwrap(),
        128,
    )
    .expect("sync log");
    let log_arc = Arc::new(sync_log);

    let script_stats = Arc::new(ScriptStats::default());
    let router = create_router(
        engine.clone(),
        None,
        Some(log_arc),
        None,
        None,
        script_stats,
        None,
        None,
        0,
    );
    // A real `_admins` row, not just a signed token: the auth middleware
    // refuses a JWT whose subject is not a user, which is how deleting a user
    // revokes their outstanding tokens. Seeded after `create_router`, which
    // runs `AuthService::init` and only creates the default `admin` while
    // `_admins` is still empty.
    let token = common::seed_user_token(&engine, "admin_user", &["admin"]);
    (tmp_dir, router, token)
}

fn bearer(token: &str) -> String {
    format!("Bearer {}", token)
}

fn json_post(uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, bearer(token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn auth_get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::AUTHORIZATION, bearer(token))
        .body(Body::empty())
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Register a session and return its (session_id, server_vector).
async fn register(app: &axum::Router, token: &str, payload: Value) -> Value {
    let resp = app
        .clone()
        .oneshot(json_post("/_api/sync/session", token, payload))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "register failed");
    body_json(resp).await
}

fn baseline_register_payload() -> Value {
    json!({
        "device_id": "dev-A",
        "api_key": "sk_test_abc",
        "subscriptions": [],
    })
}

/// `VersionVector` requires all three fields; `{}` fails to deserialize and
/// silently strips embedding fields (e.g. SyncChange.vector → SyncChange skipped).
fn empty_version_vector() -> Value {
    json!({"versions": {}, "hlc_timestamp": 0, "hlc_counter": 0})
}

fn sync_change(coll: &str, key: &str, op: &str, ts: u64) -> Value {
    json!({
        "database": "appdb",
        "collection": coll,
        "document_key": key,
        "operation": op,
        "document_data": {"name": key},
        "parent_vectors": [],
        "vector": empty_version_vector(),
        "timestamp": ts,
        "is_delta": false,
        "delta_patch": null,
    })
}

// ===========================================================================
// register_sync_session — POST /_api/sync/session
// ===========================================================================

#[tokio::test]
async fn register_session_returns_signed_id_and_capabilities() {
    let (_tmp, app, token) = create_app();
    let body = register(&app, &token, baseline_register_payload()).await;
    let session_id = body["session_id"].as_str().unwrap();
    // HMAC-signed format: <device>-<uuid>-<hex_signature>. The signature is
    // longer than 32 hex chars, so the signed id is much longer than the
    // dev-id+uuid prefix alone.
    assert!(session_id.starts_with("dev-A-"), "got {session_id}");
    assert!(
        session_id.len() > "dev-A-".len() + 36,
        "expected signed id, got {session_id}"
    );
    assert_eq!(body["capabilities"]["delta_sync"], true);
    assert_eq!(body["capabilities"]["max_batch_size"], 1_048_576);
    assert!(body["server_vector"].is_object());
}

#[tokio::test]
async fn register_session_missing_device_id_returns_400() {
    let (_tmp, app, token) = create_app();
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/session",
            &token,
            json!({"api_key": "k"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn register_session_missing_api_key_returns_400() {
    let (_tmp, app, token) = create_app();
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/session",
            &token,
            json!({"device_id": "dev"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// pull_changes — POST /_api/sync/pull
// ===========================================================================

#[tokio::test]
async fn pull_with_unmatched_subscription_returns_no_changes() {
    let (_tmp, app, token) = create_app();
    // Subscribe only to a collection nothing else writes to → guaranteed empty pull
    // (engine.initialize bootstraps _system._roles etc., which would otherwise show up).
    let session = register(
        &app,
        &token,
        json!({
            "device_id": "dev-empty",
            "api_key": "k",
            "subscriptions": ["never_used_coll"],
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/pull",
            &token,
            json!({
                "session_id": session_id,
                "client_vector": empty_version_vector(),
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(body["changes"].as_array().unwrap().is_empty());
    assert_eq!(body["has_more"], false);
    assert!(body["conflicts"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn pull_after_push_returns_changes() {
    let (_tmp, app, token) = create_app();
    let session = register(&app, &token, baseline_register_payload()).await;
    let session_id = session["session_id"].as_str().unwrap();

    // Push one change so the replication log has something to pull.
    let push_payload = json!({
        "session_id": session_id,
        "client_vector": empty_version_vector(),
        "changes": [sync_change("items", "k1", "Insert", 1)]
    });
    let push_resp = app
        .clone()
        .oneshot(json_post("/_api/sync/push", &token, push_payload))
        .await
        .unwrap();
    assert_eq!(push_resp.status(), StatusCode::OK);
    let push_body = body_json(push_resp).await;
    assert_eq!(push_body["accepted"], 1);

    // Now pull, scoped to our collection so the bootstrapped _system entries
    // don't make it into the assertion.
    let scoped = register(
        &app,
        &token,
        json!({
            "device_id": "dev-scoped",
            "api_key": "k",
            "subscriptions": ["items"],
        }),
    )
    .await;
    let scoped_id = scoped["session_id"].as_str().unwrap();
    let pull_resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/pull",
            &token,
            json!({"session_id": scoped_id, "client_vector": empty_version_vector()}),
        ))
        .await
        .unwrap();
    assert_eq!(pull_resp.status(), StatusCode::OK);
    let pull_body = body_json(pull_resp).await;
    let changes = pull_body["changes"].as_array().unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["collection"], "items");
    assert_eq!(changes[0]["document_key"], "k1");
}

#[tokio::test]
async fn pull_subscription_filter_excludes_other_collections() {
    let (_tmp, app, token) = create_app();
    let session = register(
        &app,
        &token,
        json!({
            "device_id": "dev-sub",
            "api_key": "k",
            "subscriptions": ["only_this"],
        }),
    )
    .await;
    let session_id = session["session_id"].as_str().unwrap();

    // Push two changes to different collections.
    for coll in ["only_this", "other"] {
        let _ = app
            .clone()
            .oneshot(json_post(
                "/_api/sync/push",
                &token,
                json!({
                    "session_id": session_id,
                    "client_vector": empty_version_vector(),
                    "changes": [sync_change(coll, &format!("k-{coll}"), "Insert", 1)]
                }),
            ))
            .await
            .unwrap();
    }

    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/pull",
            &token,
            json!({"session_id": session_id, "client_vector": empty_version_vector()}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let changes = body["changes"].as_array().unwrap();
    assert!(
        changes.iter().all(|c| c["collection"] == "only_this"),
        "subscription filter leaked: {changes:?}"
    );
}

#[tokio::test]
async fn pull_unknown_session_returns_400() {
    let (_tmp, app, token) = create_app();
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/pull",
            &token,
            json!({"session_id": "ghost", "client_vector": {}}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn pull_missing_session_id_returns_400() {
    let (_tmp, app, token) = create_app();
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/pull",
            &token,
            json!({"client_vector": {}}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// push_changes — POST /_api/sync/push
// ===========================================================================

#[tokio::test]
async fn push_happy_path_increments_accepted() {
    let (_tmp, app, token) = create_app();
    let session = register(&app, &token, baseline_register_payload()).await;
    let session_id = session["session_id"].as_str().unwrap();

    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/push",
            &token,
            json!({
                "session_id": session_id,
                "client_vector": empty_version_vector(),
                "changes": [
                    sync_change("c", "a", "Insert", 100),
                    sync_change("c", "a", "Update", 101),
                ]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["accepted"], 2);
    assert_eq!(body["rejected"], 0);
    assert!(body["server_vector"].is_object());
}

#[tokio::test]
async fn push_missing_session_id_returns_400() {
    let (_tmp, app, token) = create_app();
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/push",
            &token,
            json!({"client_vector": {}, "changes": []}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn push_unknown_session_returns_400() {
    let (_tmp, app, token) = create_app();
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/push",
            &token,
            json!({"session_id": "ghost", "client_vector": {}, "changes": []}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// acknowledge_changes — POST /_api/sync/ack
// ===========================================================================

#[tokio::test]
async fn ack_happy_path() {
    let (_tmp, app, token) = create_app();
    let session = register(&app, &token, baseline_register_payload()).await;
    let session_id = session["session_id"].as_str().unwrap();

    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/ack",
            &token,
            json!({"session_id": session_id, "applied_vector": {}}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["success"], true);
}

#[tokio::test]
async fn ack_missing_session_id_returns_400() {
    let (_tmp, app, token) = create_app();
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/ack",
            &token,
            json!({"applied_vector": {}}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn ack_unknown_session_returns_400() {
    let (_tmp, app, token) = create_app();
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/ack",
            &token,
            json!({"session_id": "ghost", "applied_vector": {}}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// push_changes — storage effects
// ===========================================================================

/// Pushed changes must land in storage, not just in the replication log.
///
/// This is the assertion the suite was missing. `push_changes` used to count a
/// change as accepted without writing anything, and no test could catch it:
/// `pull` reads back from the replication log, so push→pull round-tripped
/// while the document never existed. Reading through the document endpoint
/// goes to storage and is the only thing that distinguishes the two.
#[tokio::test]
async fn pushed_document_is_readable_from_storage() {
    let (_tmp, app, token) = create_app();
    let session = register(&app, &token, baseline_register_payload()).await;
    let session_id = session["session_id"].as_str().unwrap();

    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/push",
            &token,
            json!({
                "session_id": session_id,
                "client_vector": empty_version_vector(),
                "changes": [sync_change("items", "k-store", "Insert", 1)],
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["accepted"], 1);

    let resp = app
        .clone()
        .oneshot(auth_get(
            "/_api/database/appdb/document/items/k-store",
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "pushed document should exist in storage"
    );
    let doc = body_json(resp).await;
    assert_eq!(doc["_key"], "k-store");
    assert_eq!(doc["name"], "k-store");
}

/// A pushed delete removes the document from storage.
#[tokio::test]
async fn pushed_delete_removes_document_from_storage() {
    let (_tmp, app, token) = create_app();
    let session = register(&app, &token, baseline_register_payload()).await;
    let session_id = session["session_id"].as_str().unwrap();

    let push = |changes: Value| {
        json_post(
            "/_api/sync/push",
            &token,
            json!({
                "session_id": session_id,
                "client_vector": empty_version_vector(),
                "changes": changes,
            }),
        )
    };

    let resp = app
        .clone()
        .oneshot(push(json!([sync_change("items", "k-del", "Insert", 1)])))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = app
        .clone()
        .oneshot(push(json!([sync_change("items", "k-del", "Delete", 2)])))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["accepted"], 1);

    let resp = app
        .clone()
        .oneshot(auth_get(
            "/_api/database/appdb/document/items/k-del",
            &token,
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "deleted document should be gone from storage"
    );
}

/// Delta changes are rejected rather than counted as accepted — no patch
/// application exists, so accepting one would silently drop the write.
#[tokio::test]
async fn pushed_delta_change_is_rejected() {
    let (_tmp, app, token) = create_app();
    let session = register(&app, &token, baseline_register_payload()).await;
    let session_id = session["session_id"].as_str().unwrap();

    let mut change = sync_change("items", "k-delta", "Update", 1);
    change["is_delta"] = json!(true);
    change["delta_patch"] = json!({"name": "patched"});

    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/push",
            &token,
            json!({
                "session_id": session_id,
                "client_vector": empty_version_vector(),
                "changes": [change],
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["accepted"], 0);
    assert_eq!(body["rejected"], 1);
}

// ===========================================================================
// list_conflicts — GET /_api/sync/conflicts?session_id=...
// ===========================================================================

#[tokio::test]
async fn list_conflicts_unknown_session_returns_400() {
    let (_tmp, app, token) = create_app();
    let resp = app
        .clone()
        .oneshot(auth_get("/_api/sync/conflicts?session_id=ghost", &token))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// resolve_conflict — POST /_api/sync/resolve
// ===========================================================================

#[tokio::test]
async fn resolve_invalid_resolution_returns_400() {
    let (_tmp, app, token) = create_app();
    let session = register(&app, &token, baseline_register_payload()).await;
    let session_id = session["session_id"].as_str().unwrap();

    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/resolve",
            &token,
            json!({
                "session_id": session_id,
                "document_key": "doc1",
                "resolution": "ignore", // invalid
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn resolve_merged_without_data_returns_400() {
    let (_tmp, app, token) = create_app();
    let session = register(&app, &token, baseline_register_payload()).await;
    let session_id = session["session_id"].as_str().unwrap();

    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/resolve",
            &token,
            json!({
                "session_id": session_id,
                "document_key": "doc1",
                "resolution": "merged", // missing merged_data
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn resolve_missing_required_fields_returns_400() {
    let (_tmp, app, token) = create_app();
    // missing session_id
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/resolve",
            &token,
            json!({"document_key": "x", "resolution": "local"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // missing document_key
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/resolve",
            &token,
            json!({"session_id": "x", "resolution": "local"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // missing resolution
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/resolve",
            &token,
            json!({"session_id": "x", "document_key": "y"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ===========================================================================
// AuthZ — every sync route requires a JWT
// ===========================================================================

#[tokio::test]
async fn sync_routes_reject_missing_jwt() {
    let (_tmp, app, _token) = create_app();
    for (method, uri, body) in [
        ("POST", "/_api/sync/session", Some(json!({}))),
        ("POST", "/_api/sync/pull", Some(json!({}))),
        ("POST", "/_api/sync/push", Some(json!({}))),
        ("POST", "/_api/sync/ack", Some(json!({}))),
        ("GET", "/_api/sync/conflicts?session_id=x", None),
        ("POST", "/_api/sync/resolve", Some(json!({}))),
    ] {
        let mut b = Request::builder().method(method).uri(uri);
        let body_obj = if let Some(payload) = body {
            b = b.header(header::CONTENT_TYPE, "application/json");
            Body::from(payload.to_string())
        } else {
            Body::empty()
        };
        let req = b.body(body_obj).unwrap();
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

// ===========================================================================
// Conflict detection, listing and resolution
// ===========================================================================

/// A vector saying "I have seen this node's log up to `seq`" for `device`.
fn vector(device: &str, counter: u64, seen_server: u64) -> Value {
    let mut versions = serde_json::Map::new();
    versions.insert(device.to_string(), json!(counter));
    if seen_server > 0 {
        versions.insert("test-node".to_string(), json!(seen_server));
    }
    json!({"versions": versions, "hlc_timestamp": 0, "hlc_counter": 0})
}

fn change_with(key: &str, data: Value, vector: Value) -> Value {
    json!({
        "database": "appdb",
        "collection": "items",
        "document_key": key,
        "operation": "Update",
        "document_data": data,
        "parent_vectors": [],
        "vector": vector,
        "timestamp": 1,
        "is_delta": false,
        "delta_patch": null,
    })
}

async fn session_for(app: &axum::Router, token: &str, device: &str) -> String {
    let s = register(
        app,
        token,
        json!({"device_id": device, "api_key": "sk_test_abc", "subscriptions": []}),
    )
    .await;
    s["session_id"].as_str().unwrap().to_string()
}

async fn push(app: &axum::Router, token: &str, session: &str, change: Value) -> Value {
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/push",
            token,
            json!({"session_id": session, "client_vector": empty_version_vector(), "changes": [change]}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await
}

async fn conflicts(app: &axum::Router, token: &str, session: &str) -> Vec<Value> {
    let url = format!("/_api/sync/conflicts?session_id={}", session);
    let resp = app.clone().oneshot(auth_get(&url, token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await["conflicts"]
        .as_array()
        .unwrap()
        .clone()
}

async fn resolve(app: &axum::Router, token: &str, body: Value) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(json_post("/_api/sync/resolve", token, body))
        .await
        .unwrap();
    let status = resp.status();
    (status, body_json(resp).await)
}

async fn stored(app: &axum::Router, token: &str, key: &str) -> Value {
    let resp = app
        .clone()
        .oneshot(auth_get(
            &format!("/_api/database/appdb/document/items/{}", key),
            token,
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    body_json(resp).await
}

#[tokio::test]
async fn a_second_device_that_has_not_pulled_conflicts_and_can_be_resolved() {
    let (_tmp, app, token) = create_app();
    let a = session_for(&app, &token, "dev-A").await;
    let b = session_for(&app, &token, "dev-B").await;

    let first = push(
        &app,
        &token,
        &a,
        change_with("k1", json!({"v": "from-A"}), vector("dev-A", 1, 0)),
    )
    .await;
    assert_eq!(first["accepted"], 1);
    assert!(first["conflicts"].as_array().unwrap().is_empty());

    // B never pulled A's write, so its edit is concurrent with it.
    let second = push(
        &app,
        &token,
        &b,
        change_with("k1", json!({"v": "from-B"}), vector("dev-B", 1, 0)),
    )
    .await;
    assert_eq!(second["accepted"], 0, "{}", second);
    let held = second["conflicts"].as_array().unwrap();
    assert_eq!(held.len(), 1, "{}", second);
    assert_eq!(held[0]["document_key"], "k1");
    assert_eq!(held[0]["remote_data"]["v"], "from-B");
    assert_eq!(held[0]["local_data"]["v"], "from-A");
    assert_eq!(
        stored(&app, &token, "k1").await["v"],
        "from-A",
        "not applied"
    );

    // Listed for B's session only.
    let listed = conflicts(&app, &token, &b).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], held[0]["id"]);
    assert!(conflicts(&app, &token, &a).await.is_empty());

    // Resolving with the client's change applies it and clears the conflict.
    let (status, body) = resolve(
        &app,
        &token,
        json!({"session_id": b, "document_key": "k1", "resolution": "remote"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", body);
    assert_eq!(stored(&app, &token, "k1").await["v"], "from-B");
    assert!(conflicts(&app, &token, &b).await.is_empty());

    // Nothing is left to resolve.
    let (status, _) = resolve(
        &app,
        &token,
        json!({"session_id": b, "document_key": "k1", "resolution": "remote"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn keeping_the_server_copy_or_merging_are_both_honoured() {
    let (_tmp, app, token) = create_app();
    let a = session_for(&app, &token, "dev-A").await;
    let b = session_for(&app, &token, "dev-B").await;

    for key in ["keep", "merge"] {
        push(
            &app,
            &token,
            &a,
            change_with(key, json!({"v": "A"}), vector("dev-A", 1, 0)),
        )
        .await;
        push(
            &app,
            &token,
            &b,
            change_with(key, json!({"v": "B"}), vector("dev-B", 1, 0)),
        )
        .await;
    }
    assert_eq!(conflicts(&app, &token, &b).await.len(), 2);

    let (status, _) = resolve(
        &app,
        &token,
        json!({"session_id": b, "document_key": "keep", "resolution": "local"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored(&app, &token, "keep").await["v"], "A");

    let (status, _) = resolve(
        &app,
        &token,
        json!({"session_id": b, "document_key": "merge", "resolution": "merged",
               "merged_data": {"v": "A+B"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stored(&app, &token, "merge").await["v"], "A+B");
    assert!(conflicts(&app, &token, &b).await.is_empty());

    // Having settled, B's next edit is not a conflict.
    let next = push(
        &app,
        &token,
        &b,
        change_with("merge", json!({"v": "B2"}), vector("dev-B", 2, 0)),
    )
    .await;
    assert_eq!(next["accepted"], 1, "{}", next);
}

#[tokio::test]
async fn a_device_does_not_conflict_with_itself_and_a_client_that_pulled_does_not_either() {
    let (_tmp, app, token) = create_app();
    let a = session_for(&app, &token, "dev-A").await;
    let b = session_for(&app, &token, "dev-B").await;

    for n in 1..=3 {
        let r = push(
            &app,
            &token,
            &a,
            change_with("k", json!({"n": n}), vector("dev-A", n, 0)),
        )
        .await;
        assert_eq!(r["accepted"], 1, "push {} conflicted with itself: {}", n, r);
    }

    // B pulled up to a sequence far past A's write before editing.
    let r = push(
        &app,
        &token,
        &b,
        change_with("k", json!({"n": 99}), vector("dev-B", 1, 1_000_000)),
    )
    .await;
    assert_eq!(r["accepted"], 1, "{}", r);
    assert_eq!(stored(&app, &token, "k").await["n"], 99);
}

#[tokio::test]
async fn an_ordinary_write_after_a_sync_makes_the_next_push_conflict() {
    let (_tmp, app, token) = create_app();
    let a = session_for(&app, &token, "dev-A").await;

    push(
        &app,
        &token,
        &a,
        change_with("k", json!({"v": "synced"}), vector("dev-A", 1, 0)),
    )
    .await;

    // Someone edits the document through the normal API.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/_api/database/appdb/document/items/k")
                .header(header::AUTHORIZATION, bearer(&token))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"v": "server-edit"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Same device, but it has not seen that edit.
    let r = push(
        &app,
        &token,
        &a,
        change_with("k", json!({"v": "stale"}), vector("dev-A", 2, 0)),
    )
    .await;
    assert_eq!(r["conflicts"].as_array().unwrap().len(), 1, "{}", r);
    assert_eq!(stored(&app, &token, "k").await["v"], "server-edit");

    // A client that has pulled past it is fine.
    let r = push(
        &app,
        &token,
        &a,
        change_with("k", json!({"v": "informed"}), vector("dev-A", 3, 1_000_000)),
    )
    .await;
    assert_eq!(r["accepted"], 1, "{}", r);
}

#[tokio::test]
async fn conflict_bookkeeping_is_not_reachable_by_name() {
    let (_tmp, app, token) = create_app();
    let a = session_for(&app, &token, "dev-A").await;
    let b = session_for(&app, &token, "dev-B").await;
    push(
        &app,
        &token,
        &a,
        change_with("k", json!({"v": 1}), vector("dev-A", 1, 0)),
    )
    .await;
    push(
        &app,
        &token,
        &b,
        change_with("k", json!({"v": 2}), vector("dev-B", 1, 0)),
    )
    .await;

    for coll in ["_sync_conflicts", "_sync_versions"] {
        let resp = app
            .clone()
            .oneshot(json_post(
                "/_api/database/appdb/cursor",
                &token,
                json!({"query": format!("FOR d IN {} RETURN d", coll)}),
            ))
            .await
            .unwrap();
        assert!(
            resp.status().is_client_error(),
            "{} readable by name: {}",
            coll,
            resp.status()
        );
    }
}

#[tokio::test]
async fn a_delta_push_reaches_the_log_with_the_patched_document() {
    let (_tmp, app, token) = create_app();
    let a = session_for(&app, &token, "dev-A").await;
    push(
        &app,
        &token,
        &a,
        change_with("d1", json!({"name": "x", "n": 1}), empty_version_vector()),
    )
    .await;

    let mut delta = change_with("d1", Value::Null, empty_version_vector());
    delta["is_delta"] = json!(true);
    delta["delta_patch"] = json!([{"op": "replace", "path": "/n", "value": 2}]);
    let r = push(&app, &token, &a, delta).await;
    assert_eq!(r["accepted"], 1, "{}", r);

    // A reader of the log (a peer, or another device) must see the patched
    // document. The entry used to carry the client's `document_data`, which a
    // delta leaves empty, and peers drop an update with no data.
    let reader = register(
        &app,
        &token,
        json!({"device_id": "dev-reader", "api_key": "k", "subscriptions": ["items"]}),
    )
    .await;
    let pulled = body_json(
        app.clone()
            .oneshot(json_post(
                "/_api/sync/pull",
                &token,
                json!({"session_id": reader["session_id"], "client_vector": empty_version_vector()}),
            ))
            .await
            .unwrap(),
    )
    .await;
    let last = pulled["changes"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|c| c["document_key"] == "d1")
        .cloned()
        .expect("the delta is in the log");
    assert_eq!(last["document_data"]["n"], 2, "{}", last);
    assert_eq!(last["document_data"]["name"], "x", "{}", last);
}
