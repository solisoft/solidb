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
use solidb::server::auth::AuthService;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use solidb::sync::log::SyncLog;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

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
        engine,
        None,
        Some(log_arc),
        None,
        None,
        script_stats,
        None,
        None,
        0,
    );
    let token =
        AuthService::create_jwt_with_roles("admin_user", Some(vec!["admin".to_string()]), None)
            .expect("admin jwt");
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

/// Conflict listing reports that it is unimplemented rather than returning an
/// empty array. An empty array is indistinguishable from "no conflicts exist",
/// so a client could not tell that nothing is ever recorded. Detection needs
/// per-document version vectors, which storage does not carry.
#[tokio::test]
async fn list_conflicts_reports_unimplemented() {
    let (_tmp, app, token) = create_app();
    let session = register(&app, &token, baseline_register_payload()).await;
    let session_id = session["session_id"].as_str().unwrap();

    // session_id format is "device_id-uuid.hex" — URL-safe, no encoding needed.
    let url = format!("/_api/sync/conflicts?session_id={}", session_id);
    let resp = app.clone().oneshot(auth_get(&url, &token)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
}

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

/// A well-formed resolution request reports that it is unimplemented rather
/// than returning `{"success": true}`. There is no conflict store, so the old
/// response told clients their resolution had been applied when nothing had
/// been touched. Request validation still runs first — see the 400 tests below.
#[tokio::test]
async fn resolve_reports_unimplemented_for_valid_requests() {
    let (_tmp, app, token) = create_app();
    let session = register(&app, &token, baseline_register_payload()).await;
    let session_id = session["session_id"].as_str().unwrap();

    for resolution in ["local", "remote"] {
        let resp = app
            .clone()
            .oneshot(json_post(
                "/_api/sync/resolve",
                &token,
                json!({
                    "session_id": session_id,
                    "document_key": "doc1",
                    "resolution": resolution,
                }),
            ))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_IMPLEMENTED,
            "resolution={resolution}"
        );
    }

    // "merged" with its required data gets the same treatment.
    let resp = app
        .clone()
        .oneshot(json_post(
            "/_api/sync/resolve",
            &token,
            json!({
                "session_id": session_id,
                "document_key": "doc1",
                "resolution": "merged",
                "merged_data": {"v": 9},
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_IMPLEMENTED);
}

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
