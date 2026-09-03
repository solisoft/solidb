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

mod common;

struct App {
    _tmp: TempDir,
    engine: StorageEngine,
    router: axum::Router,
    admin: String,
    editor: String,
    viewer: String,
}

impl App {
    /// Mint a token for a principal that actually exists.
    ///
    /// The auth middleware refuses a JWT whose subject is not an `_admins`
    /// row — that is what makes deleting a user revoke their tokens — so a
    /// test principal has to be seeded, not just signed for.
    fn token(&self, username: &str, roles: &[&str]) -> String {
        common::seed_user_token(&self.engine, username, roles)
    }

    /// Same, but with `scoped_databases` on the claims.
    fn scoped_token(&self, username: &str, roles: &[&str], databases: &[&str]) -> String {
        common::seed_user_token(&self.engine, username, roles);
        AuthService::create_jwt_with_roles(
            username,
            Some(roles.iter().map(|r| r.to_string()).collect()),
            Some(databases.iter().map(|d| d.to_string()).collect()),
        )
        .expect("scoped jwt")
    }
}

fn create_app() -> App {
    let tmp_dir = TempDir::new().expect("temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap()).expect("engine");
    engine.initialize().expect("initialize _system");

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

    // Seed *after* `create_router`, which runs `AuthService::init`: that only
    // creates the default `admin` user while `_admins` is empty, so inserting
    // test principals first would silently suppress it.
    //
    // Real `_admins` rows, not just signed tokens: the auth middleware now
    // refuses a JWT whose subject is not a user, which is how deleting a user
    // revokes their outstanding tokens.
    let admin = common::seed_user_token(&engine, "adm", &["admin"]);
    let editor = common::seed_user_token(&engine, "edt", &["editor"]);
    let viewer = common::seed_user_token(&engine, "vwr", &["viewer"]);

    App {
        _tmp: tmp_dir,
        engine,
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
    let scoped = app.scoped_token("scoped_key", &["admin"], &["scoped1"]);

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

/// SEC-178: `DOCUMENT("otherdb:collection/key")` used to hand a caller-supplied
/// `{db}:{collection}` name straight to the storage engine, which resolves a
/// column family by literal name. A read-only key scoped to one database could
/// therefore read every other database on the instance — including
/// `_system:_admins` password hashes — because per-database authorization is
/// checked once against the `{db}` path parameter, which `DOCUMENT()` never
/// touches.
#[tokio::test]
async fn document_qualified_name_cannot_cross_database_boundary() {
    let app = create_app();
    setup_db(&app, "victim").await;
    setup_db(&app, "attacker").await;

    // Plant a document in the victim database, as its owner.
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/victim/document/items",
        &app.admin,
        Some(json!({"_key": "k1", "card": "4111-1111-1111-1111"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // A read-only key scoped to `attacker` only.
    let scoped = app.scoped_token("attacker_key", &["viewer"], &["attacker"]);

    // Control: direct access to the victim database is refused, so anything the
    // query path returns is a genuine bypass rather than a mis-scoped key.
    let (status, _) = send(
        &app.router,
        "GET",
        "/_api/database/victim/document/items/k1",
        &scoped,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // The attack: query the key's *own* database, reach across with a qualified
    // name. Every shape that accepts one.
    for query in [
        "RETURN DOCUMENT(\"victim:items/k1\")",
        "RETURN DOCUMENT([\"victim:items/k1\"])",
        "RETURN DOCUMENT(\"victim:items\", \"k1\")",
        "RETURN DOCUMENT(\"_system:_admins/admin\")",
        "RETURN DOCUMENT(\"victim:_env/OPENAI_API_KEY\")",
    ] {
        let (_, body) = send(
            &app.router,
            "POST",
            "/_api/database/attacker/cursor",
            &scoped,
            Some(json!({"query": query})),
        )
        .await;

        let serialized = body.to_string();
        assert!(
            !serialized.contains("4111-1111-1111-1111"),
            "victim document leaked via `{}`: {}",
            query,
            serialized
        );
        assert!(
            !serialized.contains("argon2"),
            "password hash leaked via `{}`: {}",
            query,
            serialized
        );
    }

    // The qualified form still works for the executor's own database, so the
    // fix removed the boundary crossing rather than the feature.
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/attacker/document/items",
        &app.admin,
        Some(json!({"_key": "mine", "value": "own-data"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/attacker/cursor",
        &scoped,
        Some(json!({"query": "RETURN DOCUMENT(\"attacker:items/mine\")"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.to_string().contains("own-data"),
        "same-database qualified lookup broke: {}",
        body
    );
}

/// SEC-178: the refusal must not double as a directory of the instance. A
/// foreign database and a database that does not exist have to answer alike,
/// or `DOCUMENT()` becomes an oracle for enumerating tenants.
#[tokio::test]
async fn document_cross_database_refusal_does_not_confirm_existence() {
    let app = create_app();
    setup_db(&app, "real_neighbour").await;
    setup_db(&app, "mine").await;

    let scoped = app.scoped_token("probe_key", &["viewer"], &["mine"]);

    let ask = |query: &'static str| {
        let router = app.router.clone();
        let token = scoped.clone();
        async move {
            send(
                &router,
                "POST",
                "/_api/database/mine/cursor",
                &token,
                Some(json!({ "query": query })),
            )
            .await
        }
    };

    let (existing_status, existing_body) =
        ask("RETURN DOCUMENT(\"real_neighbour:items/k1\")").await;
    let (absent_status, absent_body) = ask("RETURN DOCUMENT(\"no_such_db:items/k1\")").await;

    assert_eq!(
        existing_status, absent_status,
        "status differs between an existing and an absent database: {} vs {}",
        existing_body, absent_body
    );
    assert!(
        !existing_body.to_string().contains("real_neighbour")
            || absent_body.to_string().contains("no_such_db"),
        "error body reveals which databases exist: {} vs {}",
        existing_body,
        absent_body
    );
}

/// SEC-179: the transactional document endpoints bound the `{db}` path segment
/// to `_db_name` and never used it, passing the bare collection name to the
/// storage engine — which tries the literal name, then falls back to
/// `_system:{name}`. A key scoped to one database could therefore write into
/// `_system` collections, and ordinary collections were unreachable because
/// `{db}:{collection}` was never tried.
#[tokio::test]
async fn transactional_document_ops_are_scoped_to_the_path_database() {
    let app = create_app();
    setup_db(&app, "tenant_x").await;

    let scoped = app.scoped_token("tenant_key", &["editor"], &["tenant_x"]);

    let begin = |token: String| {
        let router = app.router.clone();
        async move {
            let (status, body) = send(
                &router,
                "POST",
                "/_api/database/tenant_x/transaction/begin",
                &token,
                Some(json!({})),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "begin transaction: {}", body);
            body["id"].as_str().expect("transaction id").to_string()
        }
    };

    // The correctness half of the ticket: a transactional insert into an
    // ordinary collection of the named database must work. It used to 404,
    // because only the bare name and `_system:` were ever tried.
    let tx = begin(scoped.clone()).await;
    let (status, body) = send(
        &app.router,
        "POST",
        &format!("/_api/database/tenant_x/transaction/{}/document/items", tx),
        &scoped,
        Some(json!({"_key": "in_tx", "value": 1})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "transactional insert into an ordinary collection failed: {}",
        body
    );
    assert!(
        body["_id"]
            .as_str()
            .unwrap_or_default()
            .starts_with("tenant_x:"),
        "document landed outside the path database: {}",
        body
    );

    let (status, _) = send(
        &app.router,
        "POST",
        &format!("/_api/database/tenant_x/transaction/{}/commit", tx),
        &scoped,
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The security half: naming a `_system` collection must not reach it. The
    // request is scoped to tenant_x, so it can only ever mean
    // `tenant_x:_env` — which the credential guard refuses.
    let tx = begin(scoped.clone()).await;
    let (status, body) = send(
        &app.router,
        "PUT",
        &format!(
            "/_api/database/tenant_x/transaction/{}/document/_env/OPENAI_API_KEY",
            tx
        ),
        &scoped,
        Some(json!({"value": "sk-HIJACKED"})),
    )
    .await;
    assert!(
        status != StatusCode::OK,
        "transactional write reached a credential collection: {} {}",
        status,
        body
    );
    assert!(
        !body.to_string().contains("_system:"),
        "response references a _system collection: {}",
        body
    );

    // And the instance-wide credential is untouched.
    let (_, body) = send(
        &app.router,
        "GET",
        "/_api/database/_system/env",
        &app.admin,
        None,
    )
    .await;
    assert!(
        !body.to_string().contains("sk-HIJACKED"),
        "global _env was overwritten from a scoped transaction: {}",
        body
    );
}

#[tokio::test]
async fn list_databases_filtered_by_permission() {
    let app = create_app();
    setup_db(&app, "visible1").await;
    setup_db(&app, "hidden1").await;

    let scoped = app.scoped_token("scoped_key2", &["admin"], &["visible1"]);

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
    let nobody = app.token("nobody", &[]);
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

    let nobody = app.token("ghost", &[]);

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

    // Every shape that names the collection *within this database*: a FOR
    // source, DOCUMENT() with a relative id, and DOCUMENT() with a qualified
    // `envq:collection` id (which used to reach the storage engine directly,
    // bypassing the database context entirely).
    //
    // `_system:_admins` is deliberately not in this list: since SEC-178 a
    // qualified name pointing at another database is rejected as
    // `CollectionNotFound` before the credential guard is consulted, which
    // leaks strictly less — 403 would confirm that `_system:_admins` exists.
    // It is asserted separately below.
    for query in [
        "FOR d IN _env RETURN d",
        "RETURN DOCUMENT(\"_env/OPENAI_API_KEY\")",
        "RETURN DOCUMENT(\"envq:_env/OPENAI_API_KEY\")",
        "RETURN DOCUMENT(\"_env\", \"OPENAI_API_KEY\")",
        "FOR d IN _admins RETURN d",
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

    // A qualified name reaching into `_system` is refused as a foreign
    // database, and must still surrender nothing.
    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/envq/cursor",
        &app.viewer,
        Some(json!({"query": "RETURN DOCUMENT(\"_system:_admins/admin\")"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unexpected status: {}", body);
    assert!(
        !body.to_string().contains("password_hash") && !body.to_string().contains("argon2"),
        "password hash present: {}",
        body
    );
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

#[tokio::test]
async fn credential_collections_do_not_break_the_collection_listing() {
    let app = create_app();
    setup_db(&app, "envlist").await;

    // Before the env var exists the listing works, so the assertion below is
    // about `_env` specifically and not about the endpoint being broken.
    let (status, body) = send(
        &app.router,
        "GET",
        "/_api/database/envlist/collection",
        &app.viewer,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "listing before any env var: {}",
        body
    );

    seed_env_secret(&app, "envlist", "OPENAI_API_KEY", "sk-secret").await;

    // Setting one env var creates the `_env` column family, which
    // `Database::list_collections` enumerates like any other. The guard then
    // refused `get_collection`, and the `?` in the listing handler turned that
    // into a 403 for the *whole* request: one unlistable collection made the
    // database's collection list unreachable, for every principal including
    // admin. Visiting the Env page once was enough to trigger it.
    for (label, token) in [
        ("viewer", &app.viewer),
        ("editor", &app.editor),
        ("admin", &app.admin),
    ] {
        let (status, body) = send(
            &app.router,
            "GET",
            "/_api/database/envlist/collection",
            token,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "listing 403'd for {} because _env exists: {}",
            label,
            body
        );

        // Hidden, not merely non-fatal: the listing carries a document count
        // and storage stats per collection, and `_env` is not readable through
        // this API at all.
        let names: Vec<&str> = body["collections"]
            .as_array()
            .expect("collections array")
            .iter()
            .filter_map(|c| c["name"].as_str())
            .collect();
        assert!(
            !names.contains(&"_env"),
            "_env listed for {}: {:?}",
            label,
            names
        );
        assert!(
            !body.to_string().contains("sk-secret"),
            "secret present for {}: {}",
            label,
            body
        );
    }
}

// ==================== Audit 2026-09: credential and boundary guards ====================

/// `_copy_shard` forwards this node's `X-Cluster-Secret` to the address in the
/// request body, and that secret grants admin on every route through
/// `auth_middleware`'s `X-Shard-Direct` bypass. Without a check that the
/// *caller* already holds it, any principal with Write on any database could
/// point the endpoint at a listener they control and read the secret straight
/// out of the outgoing request.
#[tokio::test]
async fn copy_shard_refuses_callers_without_the_cluster_secret() {
    let app = create_app();
    setup_db(&app, "shardsrc").await;

    for token in [&app.editor, &app.admin] {
        let (status, _) = send(
            &app.router,
            "POST",
            "/_api/database/shardsrc/collection/items/_copy_shard",
            token,
            Some(json!({"source_address": "attacker.example:8080"})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "_copy_shard must require the cluster secret, even from an admin"
        );
    }
}

/// `_user_roles` and `_roles` are the authorization decision: `get_user_roles`
/// trusts every matching row, and a stored `_roles` definition wins over the
/// built-in one. With only Write on `_system`, inserting one document into
/// either made the caller an admin.
#[tokio::test]
async fn authorization_state_collections_are_not_writable() {
    let app = create_app();

    for collection in ["_user_roles", "_roles"] {
        let (status, _) = send(
            &app.router,
            "POST",
            &format!("/_api/database/_system/document/{}", collection),
            &app.admin,
            Some(json!({"username": "edt", "role": "admin"})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "writing {} through the document API must be refused",
            collection
        );

        let (status, _) = send(
            &app.router,
            "POST",
            "/_api/database/_system/cursor",
            &app.admin,
            Some(json!({
                "query": format!(
                    "INSERT {{username: \"edt\", role: \"admin\"}} INTO {}",
                    collection
                )
            })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "writing {} through SDBQL must be refused",
            collection
        );
    }
}

/// Collections the server later executes or schedules stay listable but must
/// not be writable by name: a `_scripts` row plus a `_services` row installs
/// Lua that the service router runs, and a `_triggers` row schedules work
/// that runs as `_system`.
#[tokio::test]
async fn executable_collections_are_readable_but_not_writable() {
    let app = create_app();
    setup_db(&app, "execguard").await;

    for collection in ["_scripts", "_services", "_triggers", "_views"] {
        let (status, _) = send(
            &app.router,
            "POST",
            &format!("/_api/database/execguard/document/{}", collection),
            &app.admin,
            Some(json!({"_key": "planted", "code": "return 1"})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "writing {} through the document API must be refused",
            collection
        );

        // Reading stays available — the admin console browses these.
        let (status, _) = send(
            &app.router,
            "POST",
            "/_api/database/execguard/cursor",
            &app.admin,
            Some(json!({"query": format!("FOR d IN {} RETURN d", collection)})),
        )
        .await;
        assert_ne!(
            status,
            StatusCode::FORBIDDEN,
            "reading {} must stay allowed",
            collection
        );
    }
}

/// Same primitive as SEC-178, in the materialized-view clauses: a
/// backtick-quoted `db:collection` name went straight to the storage engine,
/// which opens any column family by literal name. `CREATE` planted a
/// collection in another tenant's database and `REFRESH` truncated one.
#[tokio::test]
async fn materialized_views_cannot_cross_the_database_boundary() {
    let app = create_app();
    setup_db(&app, "mvvictim").await;
    setup_db(&app, "mvattacker").await;

    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/mvattacker/cursor",
        &app.admin,
        Some(json!({
            "query": "CREATE MATERIALIZED VIEW `mvvictim:planted` AS FOR d IN items RETURN d"
        })),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "qualified view name must be refused, got {:?}",
        body
    );

    // The victim database gained nothing.
    let (status, body) = send(
        &app.router,
        "GET",
        "/_api/database/mvvictim/collection",
        &app.admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let listing = body.to_string();
    assert!(
        !listing.contains("planted"),
        "no collection should have appeared in the victim database: {}",
        listing
    );

    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/mvattacker/cursor",
        &app.admin,
        Some(json!({"query": "REFRESH MATERIALIZED VIEW `mvvictim:items`"})),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "qualified REFRESH must be refused");
}

/// `has_mutations()` decides whether `/cursor` upgrades a caller from Read to
/// Write. It used to walk only `body_clauses`, so a mutation parked in a
/// subquery or behind a catalog builtin ran under a read-only principal.
#[tokio::test]
async fn viewer_cannot_mutate_through_a_subquery_or_catalog_builtin() {
    let app = create_app();
    setup_db(&app, "hidden_mut").await;

    for query in [
        "RETURN (FOR e IN items INSERT {x: 1} INTO items)",
        "FOR d IN items LET y = (FOR e IN items REMOVE e IN items) RETURN y",
        "RETURN CREATE_VIEW(\"sneaky\", {collection: \"items\"})",
        "RETURN DROP_GRAPH(\"anything\")",
    ] {
        let (status, _) = send(
            &app.router,
            "POST",
            "/_api/database/hidden_mut/cursor",
            &app.viewer,
            Some(json!({"query": query})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a read-only principal must not run: {}",
            query
        );
    }
}

/// `_jobs` is the Soli framework's job store: `perform_later` inserts into it
/// by name and the worker updates rows in place, always with an admin
/// credential. The trigger dispatcher also executes its `pending` rows as
/// `_system`, so plain Write must not reach it. Admin yes, editor no.
#[tokio::test]
async fn jobs_is_writable_by_admins_only() {
    let app = create_app();
    setup_db(&app, "jobsdb").await;

    // Admin: the document API (what Soli's enqueue uses) and SDBQL both work.
    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/jobsdb/document/_jobs",
        &app.admin,
        Some(json!({"_key": "j1", "state": "queued", "class": "MailJob"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin enqueue: {:?}", body);

    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/jobsdb/cursor",
        &app.admin,
        Some(json!({"query": "UPDATE {_key: \"j1\"} WITH {state: \"running\"} IN _jobs"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "admin claim: {:?}", body);

    // Editor (Write): refused on every write path.
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/jobsdb/document/_jobs",
        &app.editor,
        Some(json!({"_key": "j2", "state": "queued"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "editor document insert");

    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/jobsdb/cursor",
        &app.editor,
        Some(json!({"query": "INSERT {state: \"queued\"} INTO _jobs"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "editor SDBQL insert");

    let (status, _) = send(
        &app.router,
        "DELETE",
        "/_api/database/jobsdb/document/_jobs/j1",
        &app.editor,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "editor delete");

    // Reading stays open to the editor: the jobs dashboard lists them.
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/jobsdb/cursor",
        &app.editor,
        Some(json!({"query": "FOR j IN _jobs RETURN j"})),
    )
    .await;
    assert_ne!(status, StatusCode::FORBIDDEN, "editor read");
}

/// The write tiers used to be enforced only on the document API and SDBQL.
/// Import, truncate and blob upload write too, and reached the collection
/// through the plain getter.
#[tokio::test]
async fn write_tiers_cover_import_truncate_and_blobs() {
    let app = create_app();
    setup_db(&app, "tiers").await;

    let (status, body) = send(
        &app.router,
        "PUT",
        "/_api/database/tiers/collection/_scripts/truncate",
        &app.admin,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "truncate _scripts: {:?}",
        body
    );

    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/blob/tiers/_scripts/upload",
        &app.admin,
        Some(json!({"file_name": "x", "total_size": 10, "chunk_size": 10, "total_chunks": 1})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "blob upload session on _scripts: {:?}",
        body
    );
}

/// A script writes as its caller. Deploy a public "proxy insert" script — the
/// realistic shape of the hole — and check that the caller's role, not the
/// script author's, decides what it may write.
#[tokio::test]
async fn scripts_write_as_their_caller() {
    let app = create_app();
    setup_db(&app, "luadb").await;
    // `items` exists from setup_db; `_jobs` is created by the first write
    // path that resolves it (`get_or_create` semantics on the executor side),
    // but the document API needs it present.
    let (status, _) = send(
        &app.router,
        "POST",
        "/_api/database/luadb/collection",
        &app.admin,
        Some(json!({"name": "_jobs"})),
    )
    .await;
    assert!(
        status == StatusCode::OK || status == StatusCode::CREATED,
        "{status}"
    );

    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/luadb/services",
        &app.admin,
        Some(json!({"key": "pub", "name": "pub", "require_auth": false})),
    )
    .await;
    assert!(status.is_success(), "create service: {:?}", body);
    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/luadb/scripts",
        &app.admin,
        Some(json!({
            "name": "proxy",
            "path": "proxy",
            "methods": ["POST"],
            "service": "pub",
            "code": "return db:collection(request.body.collection):insert(request.body.doc)"
        })),
    )
    .await;
    assert!(status.is_success(), "create script: {:?}", body);

    let call = |token: &str, collection: &str| {
        let body = json!({"collection": collection, "doc": {"planted": true}});
        let token = token.to_string();
        let router = app.router.clone();
        async move { send(&router, "POST", "/api/luadb/pub/proxy", &token, Some(body)).await }
    };

    // Viewer and admin alike: the write-protected tier stays closed.
    for token in [&app.viewer, &app.admin] {
        let (status, body) = call(token, "_scripts").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{:?}", body);
    }
    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/luadb/cursor",
        &app.admin,
        Some(json!({"query": "FOR s IN _scripts RETURN s.name"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"], json!(["proxy"]), "nothing was planted");

    // Ordinary collection: fine for everyone.
    let (status, body) = call(&app.viewer, "items").await;
    assert_eq!(status, StatusCode::OK, "{:?}", body);

    // `_jobs`: the caller's role decides.
    let (status, _) = call(&app.viewer, "_jobs").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body) = call(&app.admin, "_jobs").await;
    assert_eq!(status, StatusCode::OK, "{:?}", body);
}

/// The response contract: a script chooses its status, headers and body.
#[tokio::test]
async fn scripts_control_status_headers_and_body() {
    let app = create_app();
    setup_db(&app, "respdb").await;
    let (status, body) = send(
        &app.router,
        "POST",
        "/_api/database/respdb/services",
        &app.admin,
        Some(json!({"key": "r", "name": "r", "require_auth": false})),
    )
    .await;
    assert!(status.is_success(), "{:?}", body);

    let scripts = [
        ("plain", r#"return { a = 1 }"#),
        (
            "created",
            r#"solidb.status(201) solidb.header("X-Made", "yes") return { ok = true }"#,
        ),
        ("missing", r#"solidb.error("nope", 404)"#),
        ("page", r#"return response.html("<h1>x</h1>")"#),
        ("go", r#"return response.redirect("/elsewhere", 302)"#),
        (
            "accepted",
            r#"return response.json({ queued = true }, 202, { ["X-Q"] = "1" })"#,
        ),
        (
            "open",
            r#"return response.cors({ ok = true }, { origin = "https://a.example" })"#,
        ),
    ];
    for (path, code) in scripts {
        let (status, body) = send(
            &app.router,
            "POST",
            "/_api/database/respdb/scripts",
            &app.admin,
            Some(json!({"name": path, "path": path, "methods": ["GET"], "service": "r", "code": code})),
        )
        .await;
        assert!(status.is_success(), "deploy {path}: {:?}", body);
    }

    async fn get(router: &axum::Router, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
        let resp = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri(format!("/api/respdb/r/{path}"))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, headers, String::from_utf8_lossy(&bytes).to_string())
    }
    let ct = |h: &axum::http::HeaderMap| {
        h.get("content-type")
            .map(|v| v.to_str().unwrap_or("").to_string())
            .unwrap_or_default()
    };

    let (status, headers, body) = get(&app.router, "plain").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct(&headers).starts_with("application/json"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        json!({"a": 1})
    );

    let (status, headers, body) = get(&app.router, "created").await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(headers.get("x-made").unwrap(), "yes");

    let (status, _, body) = get(&app.router, "missing").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let err: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(err["error"], json!("nope"));

    let (status, headers, body) = get(&app.router, "page").await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct(&headers).starts_with("text/html"), "{}", ct(&headers));
    assert_eq!(body, "<h1>x</h1>");

    let (status, headers, _) = get(&app.router, "go").await;
    assert_eq!(status, StatusCode::FOUND);
    assert_eq!(headers.get("location").unwrap(), "/elsewhere");

    let (status, headers, body) = get(&app.router, "accepted").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(headers.get("x-q").unwrap(), "1");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap(),
        json!({"queued": true})
    );

    let (status, headers, _) = get(&app.router, "open").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get("access-control-allow-origin").unwrap(),
        "https://a.example"
    );

    // A second call must not inherit the first's status: overrides are
    // per request, and the pooled state is reused.
    let (status, _, _) = get(&app.router, "plain").await;
    assert_eq!(status, StatusCode::OK);
}
