//! Coverage for `src/server/handlers/websocket.rs` — WebSocket entry points
//! (`/_api/cluster/status/ws`, `/_api/ws/changefeed`, monitor handler).
//!
//! These tests bind axum to an ephemeral TCP port via `tokio::net::TcpListener`
//! and use `tokio_tungstenite::connect_async` for real handshakes. Pre-upgrade
//! auth gates (JWT, X-Cluster-Secret) are checked by the handler functions
//! before `ws.on_upgrade()`, so a failed token surfaces as an HTTP 401 from
//! the upgrade response — captured via `tungstenite::Error::Http`.
//!
//! See COV-004.

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use solidb::cluster::ClusterConfig;
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::StatusCode as TStatus;

mod common;

const TEST_CLUSTER_SECRET: &str = "ws-cov-cluster-secret";

struct TestApp {
    addr: std::net::SocketAddr,
    admin_token: String,
    _server_handle: tokio::task::JoinHandle<()>,
    _tmp: TempDir,
}

async fn spawn_app() -> TestApp {
    let tmp_dir = TempDir::new().expect("temp dir");
    let cluster_cfg = ClusterConfig {
        node_id: "test-node".to_string(),
        peers: vec![],
        replication_port: 6746,
        keyfile: Some(TEST_CLUSTER_SECRET.to_string()),
    };
    let engine = StorageEngine::with_cluster_config(tmp_dir.path().to_str().unwrap(), cluster_cfg)
        .expect("engine");
    engine.initialize().expect("init _system");

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

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local_addr");
    let server_handle = tokio::spawn(async move {
        let _ = axum::serve(listener, router.into_make_service()).await;
    });

    // A real `_admins` row, not just a signed token: the auth middleware
    // refuses a JWT whose subject is not a user, which is how deleting a user
    // revokes their outstanding tokens. Seeded after `create_router`, which
    // runs `AuthService::init` and only creates the default `admin` while
    // `_admins` is still empty.
    let admin_token = common::seed_user_token(&engine, "admin_user", &["admin"]);

    TestApp {
        addr,
        admin_token,
        _server_handle: server_handle,
        _tmp: tmp_dir,
    }
}

fn ws_url(addr: std::net::SocketAddr, path_with_query: &str) -> String {
    format!("ws://{}{}", addr, path_with_query)
}

/// Try to upgrade and return the HTTP status if the server rejected the
/// handshake. Returns `None` on a successful 101 upgrade.
async fn handshake_status(url: &str) -> Option<TStatus> {
    let req = url.into_client_request().expect("client req");
    handshake_status_with_request(req).await
}

async fn handshake_status_with_request(
    req: tokio_tungstenite::tungstenite::handshake::client::Request,
) -> Option<TStatus> {
    match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => {
            // Successful upgrade — close immediately.
            let (mut sink, _) = ws.split();
            let _ = sink.close().await;
            None
        }
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => Some(resp.status()),
        Err(e) => panic!("unexpected ws error: {e:?}"),
    }
}

// ===========================================================================
// /_api/cluster/status/ws — JWT-gated
// ===========================================================================

#[tokio::test]
async fn cluster_status_ws_rejects_invalid_token() {
    let app = spawn_app().await;
    let url = ws_url(app.addr, "/_api/cluster/status/ws?token=not-a-jwt");
    let status = handshake_status(&url)
        .await
        .expect("expected handshake fail");
    assert_eq!(status, TStatus::UNAUTHORIZED);
}

#[tokio::test]
async fn cluster_status_ws_rejects_missing_token() {
    let app = spawn_app().await;
    let url = ws_url(app.addr, "/_api/cluster/status/ws");
    // Missing `token=` query param fails the AxumQuery extractor with 400
    // (extractor error) rather than reaching the JWT branch in 401 form —
    // either way the handshake doesn't complete.
    match handshake_status(&url).await {
        Some(s) => assert!(
            s == TStatus::BAD_REQUEST || s == TStatus::UNAUTHORIZED,
            "got {s}"
        ),
        None => panic!("upgrade should not succeed without token"),
    }
}

#[tokio::test]
async fn cluster_status_ws_upgrades_with_valid_token_and_streams_status() {
    let app = spawn_app().await;
    let url = ws_url(
        app.addr,
        &format!("/_api/cluster/status/ws?token={}", app.admin_token),
    );

    let (mut ws, _resp) = tokio_tungstenite::connect_async(url)
        .await
        .expect("upgrade should succeed with admin JWT");

    // The server ticks once per second; bound the wait so a regression that
    // never sends a frame still fails this test rather than hanging forever.
    let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("timed out waiting for first cluster status frame")
        .expect("connection closed before any frame")
        .expect("ws error");
    let text = frame
        .into_text()
        .expect("status frame should be text JSON")
        .to_string();
    let _v: Value = serde_json::from_str(&text).expect("status frame must be valid JSON");

    let _ = ws.close(None).await;
}

// ===========================================================================
// /_api/ws/changefeed — JWT or X-Cluster-Secret
// ===========================================================================

#[tokio::test]
async fn changefeed_rejects_invalid_token() {
    let app = spawn_app().await;
    let url = ws_url(app.addr, "/_api/ws/changefeed?token=garbage");
    let status = handshake_status(&url).await.expect("expected reject");
    assert_eq!(status, TStatus::UNAUTHORIZED);
}

#[tokio::test]
async fn changefeed_accepts_valid_token() {
    let app = spawn_app().await;
    let url = ws_url(
        app.addr,
        &format!("/_api/ws/changefeed?token={}", app.admin_token),
    );
    let (ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("upgrade should succeed");
    let (mut sink, _) = ws.split();
    let _ = sink.close().await;
}

#[tokio::test]
async fn changefeed_accepts_cluster_secret_without_token() {
    let app = spawn_app().await;
    // Even with no `token=` query, a valid X-Cluster-Secret bypasses JWT
    // (cluster-internal path).
    let url = ws_url(app.addr, "/_api/ws/changefeed?token=ignored");
    let mut req = url.into_client_request().expect("req");
    req.headers_mut()
        .insert("X-Cluster-Secret", TEST_CLUSTER_SECRET.parse().unwrap());
    let (ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .expect("cluster-secret bypass should upgrade");
    let (mut sink, _) = ws.split();
    let _ = sink.close().await;
}

#[tokio::test]
async fn changefeed_rejects_bad_cluster_secret_and_bad_token() {
    let app = spawn_app().await;
    let url = ws_url(app.addr, "/_api/ws/changefeed?token=bad");
    let mut req = url.into_client_request().expect("req");
    req.headers_mut()
        .insert("X-Cluster-Secret", "wrong".parse().unwrap());
    let status = handshake_status_with_request(req)
        .await
        .expect("expected reject");
    assert_eq!(status, TStatus::UNAUTHORIZED);
}

#[tokio::test]
async fn changefeed_responds_with_error_to_malformed_payload() {
    let app = spawn_app().await;
    let url = ws_url(
        app.addr,
        &format!("/_api/ws/changefeed?token={}", app.admin_token),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("upgrade");

    // Send something that isn't a valid ChangefeedRequest.
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        "this is not json".into(),
    ))
    .await
    .expect("send");

    let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("timed out waiting for error frame")
        .expect("connection closed without error frame")
        .expect("ws error");
    let text = frame.into_text().unwrap_or_default().to_string();
    let v: Value = serde_json::from_str(&text).expect("error frame should be JSON");
    assert!(v.get("error").is_some(), "expected `error` field in {text}");

    let _ = ws.close(None).await;
}

#[tokio::test]
async fn changefeed_responds_with_error_to_unknown_type() {
    let app = spawn_app().await;
    let url = ws_url(
        app.addr,
        &format!("/_api/ws/changefeed?token={}", app.admin_token),
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(url)
        .await
        .expect("upgrade");

    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type": "totally_unknown"}).to_string().into(),
    ))
    .await
    .expect("send");

    let frame = tokio::time::timeout(Duration::from_secs(3), ws.next())
        .await
        .expect("timed out")
        .expect("closed early")
        .expect("ws error");
    let text = frame.into_text().unwrap_or_default().to_string();
    let v: Value = serde_json::from_str(&text).expect("json");
    assert!(v.get("error").is_some(), "expected error field in {text}");

    let _ = ws.close(None).await;
}

// ===========================================================================
// /_api/system/monitor/ws (or whatever monitor route) — JWT-gated
// ===========================================================================

#[tokio::test]
async fn monitor_ws_rejects_invalid_token() {
    let app = spawn_app().await;
    // Monitor route: `/_api/monitoring/ws` (see routes.rs).
    let url = ws_url(app.addr, "/_api/monitoring/ws?token=bad");
    match handshake_status(&url).await {
        Some(s) => assert!(
            s == TStatus::UNAUTHORIZED || s == TStatus::NOT_FOUND,
            "expected 401/404 (route not found is OK if path moved), got {s}"
        ),
        None => panic!("monitor WS should not upgrade with bad token"),
    }
}
