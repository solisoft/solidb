//! SDBQL Transaction API Tests
//!
//! Verifies:
//! - Transaction lifecycle (Begin, Commit, Rollback)
//! - Transactional SDBQL execution
//! - Isolation (Visibility)

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

fn create_test_app() -> (axum::Router, TempDir, String) {
    let tmp_dir = TempDir::new().expect("Failed to create temp dir");
    let engine = StorageEngine::new(tmp_dir.path().to_str().unwrap())
        .expect("Failed to create storage engine");
    engine
        .initialize()
        .expect("Failed to initialize storage engine");

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

    // A real `_admins` row, not just a signed token: the auth middleware
    // refuses a JWT whose subject is not a user, which is how deleting a user
    // revokes their outstanding tokens. Seeded after `create_router`, which
    // runs `AuthService::init` and only creates the default `admin` while
    // `_admins` is still empty.
    let token = common::seed_user_token(&engine, "test_admin", &["admin"]);

    (router, tmp_dir, token)
}

fn auth_header(token: &str) -> String {
    format!("Bearer {}", token)
}

async fn response_json(response: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test]
async fn test_sdbql_transaction_commit() {
    let (app, _tmp, token) = create_test_app();

    // 1. Setup DB and Collection
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "tx_db" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "users" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // 2. Begin Transaction
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db/transaction/begin")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({ "isolation": "read_committed" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let json = response_json(response).await;
    let tx_id = json["id"].as_str().unwrap().to_string();

    // 3. Execute Transactional SDBQL Insert
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/_api/database/tx_db/transaction/{}/query", tx_id))
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "INSERT { name: 'Alice' } INTO users"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    // 4. Verify NOT visible outside transaction
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "FOR u IN users RETURN u"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let json = response_json(response).await;
    let result = json["result"].as_array().unwrap();
    assert_eq!(result.len(), 0, "Data should not be visible before commit");

    // 5. Commit
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/_api/database/tx_db/transaction/{}/commit", tx_id))
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // 6. Verify visible NOW
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "FOR u IN users RETURN u"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let json = response_json(response).await;
    let result = json["result"].as_array().unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["name"], "Alice");
}

#[tokio::test]
async fn test_sdbql_transaction_rollback() {
    let (app, _tmp, token) = create_test_app();

    // Setup
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "tx_db_rb" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db_rb/collection")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(json!({ "name": "items" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Begin
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db_rb/transaction/begin")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    let json = response_json(response).await;
    let tx_id = json["id"].as_str().unwrap().to_string();

    // Insert
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/_api/database/tx_db_rb/transaction/{}/query",
                    tx_id
                ))
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "INSERT { item: 'temp' } INTO items"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Rollback
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/_api/database/tx_db_rb/transaction/{}/rollback",
                    tx_id
                ))
                .header("Authorization", auth_header(&token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Verify empty
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/_api/database/tx_db_rb/cursor")
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(&token))
                .body(Body::from(
                    json!({
                        "query": "FOR i IN items RETURN i"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let json = response_json(response).await;
    let result = json["result"].as_array().unwrap();
    assert_eq!(result.len(), 0, "Data should be gone after rollback");
}

// ---------------------------------------------------------------------------
// Mutating queries with clauses the old hand-rolled pipeline refused (COLLECT,
// window, ...) run on the ordinary executor, staged on the transaction.
// ---------------------------------------------------------------------------

async fn call(
    app: &axum::Router,
    token: &str,
    method: &str,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("Content-Type", "application/json")
                .header("Authorization", auth_header(token))
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

async fn tx_test_setup(app: &axum::Router, token: &str) -> String {
    call(app, token, "POST", "/_api/database", json!({"name": "txq"})).await;
    for c in ["orders", "totals"] {
        call(
            app,
            token,
            "POST",
            "/_api/database/txq/collection",
            json!({"name": c}),
        )
        .await;
    }
    let (status, _) = call(
        app,
        token,
        "POST",
        "/_api/database/txq/cursor",
        json!({"query": "FOR r IN [{c:'a',n:5},{c:'a',n:7},{c:'b',n:3}] INSERT {cust: r.c, amt: r.n} INTO orders"}),
    )
    .await;
    assert!(status.is_success(), "seeding failed: {}", status);
    let (_, begin) = call(
        app,
        token,
        "POST",
        "/_api/database/txq/transaction/begin",
        json!({"isolation": "read_committed"}),
    )
    .await;
    begin["id"].as_str().unwrap().to_string()
}

async fn count(app: &axum::Router, token: &str, coll: &str) -> usize {
    let (_, j) = call(
        app,
        token,
        "POST",
        "/_api/database/txq/cursor",
        json!({"query": format!("FOR d IN {} RETURN d", coll)}),
    )
    .await;
    j["result"].as_array().map(|a| a.len()).unwrap_or(0)
}

#[tokio::test]
async fn collect_then_insert_is_staged_and_committed() {
    let (app, _tmp, token) = create_test_app();
    let tx = tx_test_setup(&app, &token).await;

    let (status, out) = call(
        &app,
        &token,
        "POST",
        &format!("/_api/database/txq/transaction/{}/query", tx),
        json!({"query": "FOR o IN orders COLLECT c = o.cust AGGREGATE s = SUM(o.amt) INSERT {_key: c, total: s} INTO totals RETURN NEW.total"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", out);
    assert_eq!(out["mutationCount"], 2, "{}", out);
    let mut totals: Vec<i64> = out["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap() as i64)
        .collect();
    totals.sort();
    assert_eq!(totals, vec![3, 12]);

    assert_eq!(
        count(&app, &token, "totals").await,
        0,
        "staged, not applied"
    );
    let (status, _) = call(
        &app,
        &token,
        "POST",
        &format!("/_api/database/txq/transaction/{}/commit", tx),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(count(&app, &token, "totals").await, 2);
}

#[tokio::test]
async fn rolled_back_query_writes_nothing_and_options_are_refused() {
    let (app, _tmp, token) = create_test_app();
    let tx = tx_test_setup(&app, &token).await;
    let uri = format!("/_api/database/txq/transaction/{}/query", tx);

    let (status, _) = call(
        &app,
        &token,
        "POST",
        &uri,
        json!({"query": "FOR o IN orders UPDATE o WITH {seen: true} IN orders"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // OPTIONS need a read-modify-write a staged operation cannot express.
    let (status, refused) = call(
        &app,
        &token,
        "POST",
        &uri,
        json!({"query": "INSERT {_key: 'x'} INTO totals OPTIONS {overwriteMode: 'replace'}"}),
    )
    .await;
    assert!(
        status.is_client_error() || status == StatusCode::NOT_IMPLEMENTED,
        "expected a refusal, got {} {}",
        status,
        refused
    );
    assert!(
        refused.to_string().contains("transaction"),
        "the refusal should say why: {}",
        refused
    );

    call(
        &app,
        &token,
        "POST",
        &format!("/_api/database/txq/transaction/{}/rollback", tx),
        json!({}),
    )
    .await;
    let (_, j) = call(
        &app,
        &token,
        "POST",
        "/_api/database/txq/cursor",
        json!({"query": "FOR o IN orders FILTER o.seen == true RETURN o"}),
    )
    .await;
    assert_eq!(j["result"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn writes_a_transaction_cannot_stage_are_refused_and_leave_nothing() {
    let (app, _tmp, token) = create_test_app();
    let tx = tx_test_setup(&app, &token).await;
    let uri = format!("/_api/database/txq/transaction/{}/query", tx);

    for query in [
        "CREATE MATERIALIZED VIEW big_orders AS FOR o IN orders FILTER o.amt > 4 RETURN o",
        r#"RETURN CREATE_VIEW("orders_v", {collection: "orders"})"#,
    ] {
        let (status, body) = call(&app, &token, "POST", &uri, json!({"query": query})).await;
        assert!(
            status.is_client_error() || status == StatusCode::NOT_IMPLEMENTED,
            "{} -> {} {}",
            query,
            status,
            body
        );
    }
    call(
        &app,
        &token,
        "POST",
        &format!("/_api/database/txq/transaction/{}/rollback", tx),
        json!({}),
    )
    .await;

    // Nothing was written behind the transaction's back.
    let (_, j) = call(
        &app,
        &token,
        "POST",
        "/_api/database/txq/cursor",
        json!({"query": "FOR v IN big_orders RETURN v"}),
    )
    .await;
    assert!(
        j["result"].as_array().is_none_or(|r| r.is_empty()),
        "the view was created: {}",
        j
    );
}
