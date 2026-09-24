//! Security regressions for the offline-sync endpoints (`/_api/sync/*`):
//! audit findings C2 (push bypassed the write tiers), H3 (pull served the
//! credential tier), H9 (device_id became the replication origin) and M2
//! (sessions unbound to their creator).

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
use solidb::sync::{LogEntry, Operation};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

struct App {
    _tmp: TempDir,
    engine: StorageEngine,
    log: Arc<SyncLog>,
    router: axum::Router,
}

fn create_app() -> App {
    let tmp = TempDir::new().expect("temp dir");
    let cluster_cfg = ClusterConfig {
        node_id: "test-node".to_string(),
        peers: vec![],
        replication_port: 6746,
        keyfile: Some("sync-security-cluster-secret".to_string()),
    };
    let engine = StorageEngine::with_cluster_config(tmp.path().to_str().unwrap(), cluster_cfg)
        .expect("engine");
    engine.initialize().expect("initialize _system");
    let log = Arc::new(
        SyncLog::new("test-node".to_string(), tmp.path().to_str().unwrap(), 1024)
            .expect("sync log"),
    );
    let router = create_router(
        engine.clone(),
        None,
        Some(log.clone()),
        None,
        None,
        Arc::new(ScriptStats::default()),
        None,
        None,
        0,
    );
    App {
        _tmp: tmp,
        engine,
        log,
        router,
    }
}

fn json_post(uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn body_json(resp: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn vv() -> Value {
    json!({"versions": {}, "hlc_timestamp": 0, "hlc_counter": 0})
}

fn change(db: &str, coll: &str, key: &str, data: Value) -> Value {
    json!({
        "database": db,
        "collection": coll,
        "document_key": key,
        "operation": "Insert",
        "document_data": data,
        "parent_vectors": [],
        "vector": vv(),
        "timestamp": 1,
        "is_delta": false,
        "delta_patch": null,
    })
}

async fn register(app: &App, token: &str, payload: Value) -> (StatusCode, Value) {
    let resp = app
        .router
        .clone()
        .oneshot(json_post("/_api/sync/session", token, payload))
        .await
        .unwrap();
    let status = resp.status();
    (status, body_json(resp).await)
}

async fn session_for(app: &App, token: &str, device: &str) -> String {
    let (status, body) = register(
        app,
        token,
        json!({"device_id": device, "api_key": "k", "subscriptions": []}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "register failed: {}", body);
    body["session_id"].as_str().unwrap().to_string()
}

async fn push(app: &App, token: &str, session: &str, changes: Vec<Value>) -> (StatusCode, Value) {
    let resp = app
        .router
        .clone()
        .oneshot(json_post(
            "/_api/sync/push",
            token,
            json!({"session_id": session, "client_vector": vv(), "changes": changes}),
        ))
        .await
        .unwrap();
    let status = resp.status();
    (status, body_json(resp).await)
}

/// C2: a Write principal cannot push into `_jobs` (executed as `_system`) or
/// the write-protected tier.
#[tokio::test]
async fn push_refuses_protected_collections_for_non_admin() {
    let app = create_app();
    let editor = common::seed_user_token(&app.engine, "sync_editor", &["editor"]);
    let session = session_for(&app, &editor, "dev-editor").await;

    let (status, body) = push(
        &app,
        &editor,
        &session,
        vec![
            change(
                "_system",
                "_jobs",
                "evil-job",
                json!({"status": "pending", "script_path": "x"}),
            ),
            change("_system", "_scripts", "evil-script", json!({"code": "x"})),
            change(
                "_system",
                "_admins",
                "evil-admin",
                json!({"password_hash": "x"}),
            ),
        ],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["accepted"], 0, "{}", body);
    assert_eq!(body["rejected"], 3, "{}", body);

    let db = app.engine.get_database("_system").unwrap();
    if let Ok(jobs) = db.system_collection("_jobs") {
        assert!(jobs.get("evil-job").is_err(), "_jobs row was written");
    }
    if let Ok(scripts) = db.system_collection("_scripts") {
        assert!(
            scripts.get("evil-script").is_err(),
            "_scripts row was written"
        );
    }
    assert!(db
        .system_collection("_admins")
        .unwrap()
        .get("evil-admin")
        .is_err());
}

/// C2: pushing to a database that does not exist no longer creates it for a
/// principal without instance Admin.
#[tokio::test]
async fn push_does_not_create_database_without_admin() {
    let app = create_app();
    let editor = common::seed_user_token(&app.engine, "sync_editor2", &["editor"]);
    let session = session_for(&app, &editor, "dev-editor2").await;

    let (status, body) = push(
        &app,
        &editor,
        &session,
        vec![change("brand_new_db", "items", "k1", json!({"a": 1}))],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["accepted"], 0, "{}", body);
    assert!(app.engine.get_database("brand_new_db").is_err());
}

/// H9: a pushed change is logged under the local node id, never the
/// client-chosen device id.
#[tokio::test]
async fn push_logs_under_local_node_id() {
    let app = create_app();
    let admin = common::seed_user_token(&app.engine, "sync_admin", &["admin"]);
    let session = session_for(&app, &admin, "node-b").await;

    let (status, body) = push(
        &app,
        &admin,
        &session,
        vec![change("appdb", "items", "h9-key", json!({"a": 1}))],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["accepted"], 1, "{}", body);

    let entries = app.log.get_entries_after(0, 10_000);
    let pushed: Vec<_> = entries.iter().filter(|e| e.key == "h9-key").collect();
    assert_eq!(pushed.len(), 1);
    assert_eq!(pushed[0].node_id, "test-node");
}

/// Invalid device ids are refused at registration.
#[tokio::test]
async fn register_rejects_invalid_device_id() {
    let app = create_app();
    let admin = common::seed_user_token(&app.engine, "sync_admin2", &["admin"]);
    for bad in ["", "has space", "10.0.0.2:6746"] {
        let (status, _) = register(&app, &admin, json!({"device_id": bad, "api_key": "k"})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "device_id {:?}", bad);
    }
    let long = "a".repeat(200);
    let (status, _) = register(&app, &admin, json!({"device_id": long, "api_key": "k"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// H3: credential-tier entries in the replication log are never served, and
/// subscribing to them is refused.
#[tokio::test]
async fn pull_never_serves_credential_collections() {
    let app = create_app();
    app.log.append(LogEntry::new_op(
        "_system",
        "_admins",
        Operation::Insert,
        "victim",
        Some(br#"{"_key":"victim","password_hash":"$argon2id$secret"}"#.to_vec()),
    ));
    app.log.append(LogEntry::new_op(
        "_system",
        "_api_keys",
        Operation::Insert,
        "k1",
        Some(br#"{"_key":"k1","key_hash":"secret"}"#.to_vec()),
    ));

    let viewer = common::seed_user_token(&app.engine, "sync_viewer", &["viewer"]);

    let (status, _) = register(
        &app,
        &viewer,
        json!({"device_id": "dev-v", "api_key": "k", "subscriptions": ["_admins"]}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let session = session_for(&app, &viewer, "dev-v").await;
    let resp = app
        .router
        .clone()
        .oneshot(json_post(
            "/_api/sync/pull",
            &viewer,
            json!({"session_id": session, "client_vector": vv(), "limit": 1_000_000_000u64}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let changes = body["changes"].as_array().unwrap();
    assert!(changes.len() <= 1000);
    for c in changes {
        let coll = c["collection"].as_str().unwrap();
        assert!(
            !matches!(
                coll,
                "_admins" | "_api_keys" | "_env" | "_roles" | "_user_roles"
            ),
            "pull served {}",
            coll
        );
    }
    assert!(!body.to_string().contains("$argon2id$secret"));
}

/// M2: a session can only be used by the principal that registered it.
#[tokio::test]
async fn session_is_bound_to_its_creator() {
    let app = create_app();
    let alice = common::seed_user_token(&app.engine, "sync_alice", &["admin"]);
    let bob = common::seed_user_token(&app.engine, "sync_bob", &["admin"]);
    let session = session_for(&app, &alice, "dev-alice").await;

    for uri in ["/_api/sync/pull", "/_api/sync/ack"] {
        let resp = app
            .router
            .clone()
            .oneshot(json_post(
                uri,
                &bob,
                json!({"session_id": session, "client_vector": vv(), "applied_vector": vv()}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "{}", uri);
    }

    let (status, _) = push(
        &app,
        &bob,
        &session,
        vec![change("appdb", "items", "k", json!({}))],
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The creator still can.
    let (status, _) = push(
        &app,
        &alice,
        &session,
        vec![change("appdb", "items", "k", json!({}))],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}
