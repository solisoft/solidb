//! Per-database authorization tests for the data plane.
//!
//! Covers the authz middleware on `/_api/database/{db}/...` routes, the
//! mutating-query write upgrade on `/cursor`, the Admin override for
//! truncate / drop-collection, scoped API-key behavior (no global ops,
//! no cross-database access), and cursor-continuation authorization.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::json;
use solidb::scripting::ScriptStats;
use solidb::server::auth::AuthService;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

struct App {
    _tmp: TempDir,
    router: axum::Router,
    admin: String,
    editor: String,
    viewer: String,
}

fn create_app() -> App {
    let tmp_dir = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).expect("engine");
    engine.initialize().expect("initialize _system");
    let script_stats = Arc::new(ScriptStats::default());
    let router = create_router(engine, None, None, None, None, script_stats, None, None, 0);

    let admin = AuthService::create_jwt_with_roles("adm", Some(vec!["admin".to_string()]), None)
        .expect("admin jwt");
    let editor = AuthService::create_jwt_with_roles("edt", Some(vec!["editor".to_string()]), None)
        .expect("editor jwt");
    let viewer = AuthService::create_jwt_with_roles("vwr", Some(vec!["viewer".to_string()]), None)
        .expect("viewer jwt");

    App {
        _tmp: tmp_dir,
        router,
        admin,
        editor,
        viewer,
    }
}

async fn send(
    app: &axum::Router,
    method: &str,
    uri: &str,
    token: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("Authorization", format!("Bearer {}", token));
    let body = match body {
        Some(v) => {
            builder = builder.header("Content-Type", "application/json");
            Body::from(v.to_string())
        }
        None => Body::empty(),
    };
    let resp = app
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap_or_default();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

/// Admin bootstraps a database with one collection and one document.
async fn setup_db(app: &App, db: &str) {
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database",
        &app.admin,
        Some(json!({"name": db})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create database {}", db);

    let (status, _) = send(
        &app.router,
        "POST",
        &format!("/_api/database/{}/collection", db),
        &app.admin,
        Some(json!({"name": "items"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "create collection");

    let (status, _) = send(
        &app.router,
        "POST",
        &format!("/_api/database/{}/document/items", db),
        &app.admin,
        Some(json!({"_key": "k1", "value": 1})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "insert seed document");
}

#[tokio::test]
async fn viewer_can_read_but_not_write() {
    let app = create_app();
    setup_db(&app, "authz1").await;

    // Read: OK
    let (status, _) = send(
        &app.router,
        "GET",
        "/_api/database/authz1/document/items/k1",
        &app.viewer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Write: forbidden
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/authz1/document/items",
        &app.viewer,
        Some(json!({"value": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Delete: forbidden
    let (status, _) = send(
        &app.router,
        "DELETE",
        "/_api/database/authz1/document/items/k1",
        &app.viewer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn viewer_query_read_ok_mutation_forbidden() {
    let app = create_app();
    setup_db(&app, "authz2").await;

    // Read-only SDBQL: OK
    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/authz2/cursor",
        &app.viewer,
        Some(json!({"query": "FOR doc IN items RETURN doc"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "read query should pass: {}", body);

    // Mutating SDBQL through the same read endpoint: forbidden
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/authz2/cursor",
        &app.viewer,
        Some(json!({"query": "INSERT {value: 3} INTO items"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Editor may run the same mutation
    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/authz2/cursor",
        &app.editor,
        Some(json!({"query": "INSERT {value: 3} INTO items"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "editor mutation: {}", body);
}

#[tokio::test]
async fn truncate_and_drop_require_admin() {
    let app = create_app();
    setup_db(&app, "authz3").await;

    // Editor (global write) may NOT truncate
    let (status, _) = send(
        &app.router,
        "PUT",
        "/_api/database/authz3/collection/items/truncate",
        &app.editor,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // ... nor drop the collection
    let (status, _) = send(
        &app.router,
        "DELETE",
        "/_api/database/authz3/collection/items",
        &app.editor,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Admin may truncate
    let (status, body) = send(
        &app.router,
        "PUT",
        "/_api/database/authz3/collection/items/truncate",
        &app.admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin truncate: {}", body);
}

#[tokio::test]
async fn scoped_key_cannot_do_global_ops_or_cross_db() {
    let app = create_app();
    setup_db(&app, "scoped1").await;
    setup_db(&app, "scoped2").await;

    // Token shaped like a db-scoped API key with the admin role.
    let scoped = AuthService::create_jwt_with_roles(
        "scoped_key",
        Some(vec!["admin".to_string()]),
        Some(vec!["scoped1".to_string()]),
    )
    .expect("scoped jwt");

    // Inside scope: write works
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/scoped1/document/items",
        &scoped,
        Some(json!({"value": 10})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Cross-database: forbidden
    let (status, _) = send(
        &app.router,
        "GET",
        "/_api/database/scoped2/document/items/k1",
        &scoped,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // Global ops: forbidden even with the admin role
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database",
        &scoped,
        Some(json!({"name": "evil_db"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _) = send(
        &app.router,
        "DELETE",
        "/_api/database/scoped2",
        &scoped,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // delete inside scope is allowed (db-scoped admin)
    let (status, _) = send(
        &app.router,
        "DELETE",
        "/_api/database/scoped1",
        &scoped,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn list_databases_filtered_by_permission() {
    let app = create_app();
    setup_db(&app, "visible1").await;
    setup_db(&app, "hidden1").await;

    let scoped = AuthService::create_jwt_with_roles(
        "scoped_key2",
        Some(vec!["admin".to_string()]),
        Some(vec!["visible1".to_string()]),
    )
    .expect("scoped jwt");

    let (status, body) = send(&app.router, "GET", "/_api/databases", &scoped, None).await;
    assert_eq!(status, StatusCode::OK);
    let dbs: Vec<String> = body
        .get("databases")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    assert!(dbs.contains(&"visible1".to_string()), "got {:?}", dbs);
    assert!(!dbs.contains(&"hidden1".to_string()), "got {:?}", dbs);

    // Admin still sees everything
    let (status, body) = send(&app.router, "GET", "/_api/databases", &app.admin, None).await;
    assert_eq!(status, StatusCode::OK);
    let dbs: Vec<String> = body
        .get("databases")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    assert!(dbs.contains(&"visible1".to_string()));
    assert!(dbs.contains(&"hidden1".to_string()));
}

#[tokio::test]
async fn cursor_continuation_checks_db_permission() {
    let app = create_app();
    setup_db(&app, "cursordb").await;

    // Seed enough documents to force a cursor (batch_size 2, 5 docs).
    for i in 2..=5 {
        let (status, _) = send(
            &app.router,
            "POST",
            "/_api/database/cursordb/document/items",
            &app.admin,
            Some(json!({"_key": format!("k{}", i), "value": i})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/cursordb/cursor",
        &app.viewer,
        Some(json!({"query": "FOR doc IN items RETURN doc", "batchSize": 2})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "query: {}", body);
    let cursor_id = body
        .get("id")
        .and_then(|v| v.as_str())
        .expect("cursor id for paged result")
        .to_string();

    // A principal with no roles can NOT continue someone's cursor.
    let nobody = AuthService::create_jwt_with_roles("nobody", None, None).expect("role-less jwt");
    let (status, _) = send(
        &app.router,
        "PUT",
        &format!("/_api/cursor/{}", cursor_id),
        &nobody,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The viewer continues fine.
    let (status, body) = send(
        &app.router,
        "PUT",
        &format!("/_api/cursor/{}", cursor_id),
        &app.viewer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "continuation: {}", body);
}

#[tokio::test]
async fn role_less_token_denied_on_data_plane() {
    let app = create_app();
    setup_db(&app, "noroles").await;

    let nobody = AuthService::create_jwt_with_roles("ghost", None, None).expect("jwt");

    let (status, _) = send(
        &app.router,
        "GET",
        "/_api/database/noroles/document/items/k1",
        &nobody,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// ==================== SEC-176: credential collections ====================
//
// `_env` is where SoliDB tells users to put provider API keys, `_admins`
// holds argon2 password hashes and `_api_keys` holds key hashes — but they
// are ordinary collections, so every generic read path served them to any
// principal with Read. These tests pin each of the four paths shut.

/// Admin stores a secret in `_env` through the admin-only endpoint.
async fn seed_env_secret(app: &App, db: &str, key: &str, value: &str) {
    let (status, body) = send(
        &app.router,
        "PUT",
        &format!("/_api/database/{}/env/{}", db, key),
        &app.admin,
        Some(json!({ "value": value })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin seeds env var: {}", body);
}

#[tokio::test]
async fn env_endpoints_require_admin_not_read_or_write() {
    let app = create_app();
    setup_db(&app, "envperm").await;
    seed_env_secret(&app, "envperm", "OPENAI_API_KEY", "sk-secret").await;

    // Admin still round-trips the value — the endpoint has to stay usable.
    let (status, body) = send(
        &app.router,
        "GET",
        "/_api/database/envperm/env",
        &app.admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["OPENAI_API_KEY"], "sk-secret");

    // Read is no longer enough to list.
    let (status, _) = send(
        &app.router,
        "GET",
        "/_api/database/envperm/env",
        &app.viewer,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "viewer must not list env vars"
    );

    // Write is no longer enough to set — this is also what kept a tenant from
    // pointing OLLAMA_URL at an internal host (SEC-177).
    let (status, _) = send(
        &app.router,
        "PUT",
        "/_api/database/envperm/env/OLLAMA_URL",
        &app.editor,
        Some(json!({ "value": "http://169.254.169.254" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "editor must not set env vars"
    );

    let (status, _) = send(
        &app.router,
        "DELETE",
        "/_api/database/envperm/env/OPENAI_API_KEY",
        &app.editor,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "editor must not delete env vars"
    );
}

#[tokio::test]
async fn credential_collections_are_unreachable_via_document_api() {
    let app = create_app();
    setup_db(&app, "envdoc").await;
    seed_env_secret(&app, "envdoc", "OPENAI_API_KEY", "sk-secret").await;

    for token in [&app.viewer, &app.editor, &app.admin] {
        let (status, body) = send(
            &app.router,
            "GET",
            "/_api/database/envdoc/document/_env/OPENAI_API_KEY",
            token,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "document API leaked: {}",
            body
        );
        assert!(
            !body.to_string().contains("sk-secret"),
            "secret present in body: {}",
            body
        );
    }
}

#[tokio::test]
async fn credential_collections_are_unreachable_via_sdbql() {
    let app = create_app();
    setup_db(&app, "envq").await;
    seed_env_secret(&app, "envq", "OPENAI_API_KEY", "sk-secret").await;

    // Every shape that names the collection: a FOR source, DOCUMENT() with a
    // relative id, and DOCUMENT() with a fully-qualified `db:collection` id
    // (which used to reach the storage engine directly, bypassing the
    // database context entirely).
    for query in [
        "FOR d IN _env RETURN d",
        "RETURN DOCUMENT(\"_env/OPENAI_API_KEY\")",
        "RETURN DOCUMENT(\"envq:_env/OPENAI_API_KEY\")",
        "RETURN DOCUMENT(\"_env\", \"OPENAI_API_KEY\")",
        "FOR d IN _admins RETURN d",
        "RETURN DOCUMENT(\"_system:_admins/admin\")",
    ] {
        let (status, body) = send(
            &app.router,
            "POST",
            "/_api/database/envq/cursor",
            &app.viewer,
            Some(json!({ "query": query })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "query leaked: {} => {}",
            query,
            body
        );
        assert!(
            !body.to_string().contains("sk-secret"),
            "secret present for {}: {}",
            query,
            body
        );
        assert!(
            !body.to_string().contains("password_hash"),
            "password hash present for {}: {}",
            query,
            body
        );
    }
}

#[tokio::test]
async fn ordinary_collections_are_unaffected_by_the_guard() {
    let app = create_app();
    setup_db(&app, "envok").await;

    // The guard is name-based, so prove it does not over-match: a normal
    // collection and an unrelated underscore collection both still work.
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/envok/cursor",
        &app.viewer,
        Some(json!({ "query": "FOR d IN items RETURN d" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(
        &app.router,
        "GET",
        "/_api/database/envok/document/items/k1",
        &app.viewer,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/envok/collection",
        &app.admin,
        Some(json!({"name": "_env_like"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "near-miss name must be creatable");

    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/envok/cursor",
        &app.viewer,
        Some(json!({ "query": "FOR d IN _env_like RETURN d" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "near-miss name must be queryable: {}",
        body
    );
}

#[tokio::test]
async fn credential_collections_are_unreachable_via_transactions() {
    let app = create_app();
    setup_db(&app, "envtx").await;
    seed_env_secret(&app, "envtx", "OPENAI_API_KEY", "sk-secret").await;

    // The transactional handlers resolve the collection through the storage
    // engine rather than through a Database, so the guard has to exist at
    // both levels. Worse, the engine falls back to `_system:{name}` for an
    // unqualified name, so this path reached the instance-wide credentials
    // from any database (see SEC-179 for that defect itself).
    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/envtx/transaction/begin",
        &app.editor,
        Some(json!({})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "editor begins transaction: {}",
        body
    );
    let tx_id = body["transaction_id"]
        .as_str()
        .or_else(|| body["id"].as_str())
        .expect("transaction id")
        .to_string();

    let (status, body) = send(
        &app.router,
        "PUT",
        &format!(
            "/_api/database/envtx/transaction/{}/document/_env/OPENAI_API_KEY",
            tx_id
        ),
        &app.editor,
        Some(json!({ "value": "sk-hijacked" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "transactional write to _env: {}",
        body
    );

    let (status, body) = send(
        &app.router,
        "POST",
        &format!("/_api/database/envtx/transaction/{}/query", tx_id),
        &app.editor,
        Some(json!({ "query": "FOR d IN _env RETURN d" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "transactional query on _env: {}",
        body
    );
    assert!(
        !body.to_string().contains("sk-secret"),
        "secret present: {}",
        body
    );
}
