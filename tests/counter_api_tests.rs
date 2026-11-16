//! Counter fields over HTTP and across nodes: the endpoints, the number
//! reached through the document API and SDBQL, and what a peer keeps.

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::StorageEngine;
use solidb::sync::log::SyncLog;
use solidb::sync::{counters, LogEntry, Operation};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

struct Node {
    app: axum::Router,
    engine: StorageEngine,
    log: Arc<SyncLog>,
    token: String,
    _tmp: TempDir,
}

/// A node named `id`, with a database `shop`, a collection `invoices` and
/// an active replication log.
fn node(id: Option<&str>, peers: bool) -> Node {
    let tmp = TempDir::new().unwrap();
    let engine = StorageEngine::new(tmp.path().join("data").to_str().unwrap()).unwrap();
    engine.initialize().unwrap();
    engine.set_local_node(id.map(str::to_string), peers);
    engine.create_database("shop".to_string()).unwrap();
    engine
        .get_database("shop")
        .unwrap()
        .create_collection("invoices".to_string(), None)
        .unwrap();
    let dir = tmp.path().join("log");
    let log = Arc::new(SyncLog::new("n".to_string(), dir.to_str().unwrap(), 64).unwrap());
    let app = create_router(
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
    let token = common::seed_user_token(&engine, "test_admin", &["admin"]);
    Node {
        app,
        engine,
        log,
        token,
        _tmp: tmp,
    }
}

async fn call(node: &Node, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", node.token));
    let body = body.map_or_else(Body::empty, |b| Body::from(b.to_string()));
    let response = node
        .app
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

async fn query(node: &Node, text: &str) -> (StatusCode, Value) {
    call(
        node,
        "POST",
        "/_api/database/shop/cursor",
        Some(json!({ "query": text })),
    )
    .await
}

const COUNTERS: &str = "/_api/database/shop/collection/invoices/counters";
const DOCS: &str = "/_api/database/shop/document/invoices";

#[tokio::test(flavor = "multi_thread")]
async fn counters_over_http() {
    let n = node(None, false);

    let (status, body) = call(
        &n,
        "POST",
        COUNTERS,
        Some(json!({"field": "number", "start": 7, "prefix": "INV-", "padding": 3})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["field"], "number");
    assert_eq!(body["state"]["next"], 7);
    assert_eq!(body["index"], "counter_number");
    assert_eq!(body["node"], Value::Null);

    // The document API and SDBQL are numbered from the same count.
    let (status, doc) = call(&n, "POST", DOCS, Some(json!({"total": 10}))).await;
    assert!(status.is_success(), "{status} {doc}");
    assert_eq!(doc["number"], "INV-007");
    let (status, body) = query(
        &n,
        "FOR i IN 1..3 INSERT {total: i} INTO invoices RETURN NEW.number",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["result"], json!(["INV-008", "INV-009", "INV-010"]));
    let (_, body) = query(&n, "INSERT {total: 1} INTO invoices RETURN NEW.number").await;
    assert_eq!(body["result"], json!(["INV-011"]));

    // The number is not the client's.
    let (status, body) = call(&n, "POST", DOCS, Some(json!({"number": "INV-001"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = query(
        &n,
        "FOR d IN invoices FILTER d.number == 'INV-007' UPDATE d WITH {number: 'X'} IN invoices",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");

    // It is found by its number, through the unique index.
    let (_, body) = query(
        &n,
        "FOR d IN invoices FILTER d.number == 'INV-009' RETURN d.total",
    )
    .await;
    assert_eq!(body["result"], json!([2]));

    // Listing, the next number, and the drop.
    let (status, body) = call(&n, "GET", COUNTERS, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["counters"][0]["state"]["next"], 12);
    let next = format!("{COUNTERS}/number/next");
    let (status, body) = call(&n, "PUT", &next, Some(json!({"next": 9}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "a held number: {body}");
    let (status, body) = call(&n, "PUT", &next, Some(json!({"next": 500}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (_, doc) = call(&n, "POST", DOCS, Some(json!({}))).await;
    assert_eq!(doc["number"], "INV-500");

    let (status, _) = call(&n, "DELETE", &format!("{COUNTERS}/number"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (_, body) = call(&n, "GET", COUNTERS, None).await;
    assert_eq!(body["counters"], json!([]));
    let (status, _) = call(&n, "DELETE", &format!("{COUNTERS}/number"), None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_declaration_refuses() {
    let n = node(None, false);
    for (body, why) in [
        (json!({"field": "_key"}), "a system field"),
        (json!({"field": "a.b"}), "a nested field"),
        (
            json!({"field": "n", "reset": "month", "prefix": "{YYYY}-"}),
            "a period the value does not name",
        ),
        (
            json!({"field": "n", "timezone": "Nowhere/City"}),
            "a timezone",
        ),
        (json!({"field": "n", "start": 0}), "a start of zero"),
    ] {
        let (status, answer) = call(&n, "POST", COUNTERS, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {answer}");
    }
    // In a cluster the node that numbers must have a name.
    let clustered = node(None, true);
    let (status, body) = call(&clustered, "POST", COUNTERS, Some(json!({"field": "n"}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_transaction_over_http_is_numbered_at_commit() {
    let n = node(None, false);
    call(&n, "POST", COUNTERS, Some(json!({"field": "number"}))).await;
    let (status, body) = call(
        &n,
        "POST",
        "/_api/database/shop/transaction/begin",
        Some(json!({})),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    let tx = body["id"].as_str().unwrap().to_string();
    let (status, body) = call(
        &n,
        "POST",
        &format!("/_api/database/shop/transaction/{tx}/query"),
        Some(json!({"query": "INSERT {_key: 'a'} INTO invoices"})),
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    let (_, doc) = call(&n, "POST", DOCS, Some(json!({"_key": "before"}))).await;
    assert_eq!(doc["number"], 1);
    let (status, body) = call(
        &n,
        "POST",
        &format!("/_api/database/shop/transaction/{tx}/commit"),
        None,
    )
    .await;
    assert!(status.is_success(), "{status} {body}");
    let (_, body) = query(
        &n,
        "FOR d IN invoices SORT d.number RETURN [d._key, d.number]",
    )
    .await;
    assert_eq!(body["result"], json!([["before", 1], ["a", 2]]));
}

/// What the log of `node` holds about `invoices`.
fn entries(node: &Node) -> Vec<LogEntry> {
    node.log
        .get_entries_after(0, 100_000)
        .into_iter()
        .filter(|e| e.collection == "invoices")
        .collect()
}

/// What the replication worker does with each entry, on `peer`.
fn apply(peer: &Node, entries: &[LogEntry]) {
    let collection = peer
        .engine
        .get_database("shop")
        .unwrap()
        .get_collection("invoices")
        .unwrap();
    for e in entries {
        match e.operation {
            Operation::PutCounter | Operation::DropCounter => {
                counters::apply(&collection, e.operation, &e.key, e.data.as_deref()).unwrap()
            }
            Operation::Insert | Operation::Update => {
                let document: Value = serde_json::from_slice(e.data.as_ref().unwrap()).unwrap();
                solidb::storage::foreign_keys::unchecked(|| {
                    collection.upsert_batch(vec![(e.key.clone(), document)])
                })
                .unwrap();
            }
            _ => {}
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn one_node_numbers_and_hands_over() {
    let n1 = node(Some("n1"), true);
    let n2 = node(Some("n2"), true);

    let (status, body) = call(&n1, "POST", COUNTERS, Some(json!({"field": "number"}))).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(body["node"], "n1");
    for _ in 0..3 {
        call(&n1, "POST", DOCS, Some(json!({}))).await;
    }
    apply(&n2, &entries(&n1));

    // The peer has the counter, its index and the numbered documents, and
    // refuses to number: n1 does.
    let (_, body) = call(&n2, "GET", COUNTERS, None).await;
    assert_eq!(body["counters"][0]["node"], "n1");
    let (_, body) = query(&n2, "FOR d IN invoices SORT d.number RETURN d.number").await;
    assert_eq!(body["result"], json!([1, 2, 3]));
    let (status, body) = call(&n2, "POST", DOCS, Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.to_string().contains("n1"), "{body}");
    // Nor may it hand over what it does not hold.
    let node_url = format!("{COUNTERS}/number/node");
    let (status, _) = call(&n2, "PUT", &node_url, Some(json!({"node": "n2"}))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // n1 hands the counter to n2 and stops numbering in the same write.
    let (status, body) = call(&n1, "PUT", &node_url, Some(json!({"node": "n2"}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["state"]["next"], 4);
    let (status, _) = call(&n1, "POST", DOCS, Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    // Replayed whole, as a joining node replays a log: the same result.
    apply(&n2, &entries(&n1));
    let (status, doc) = call(&n2, "POST", DOCS, Some(json!({}))).await;
    assert!(status.is_success(), "{status} {doc}");
    assert_eq!(doc["number"], 4);

    // A drop reaches the peer, and an older record does not bring the
    // counter back.
    let before_drop = entries(&n2);
    let (status, _) = call(&n2, "DELETE", &format!("{COUNTERS}/number"), None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    apply(&n1, &entries(&n2));
    let (_, body) = call(&n1, "GET", COUNTERS, None).await;
    assert_eq!(body["counters"], json!([]));
    apply(&n1, &before_drop);
    apply(&n1, &entries(&n1));
    let (_, body) = call(&n1, "GET", COUNTERS, None).await;
    assert_eq!(
        body["counters"],
        json!([]),
        "an old record revived the counter"
    );
}

/// `POST …/collection/{name}/import` with `lines` as a JSONL file, which
/// does not end with a newline: the last line is a document like the
/// others.
async fn import(node: &Node, query: &str, lines: &str) -> (StatusCode, Value) {
    const BOUNDARY: &str = "counter-import-boundary";
    let body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"docs.jsonl\"\r\nContent-Type: application/x-ndjson\r\n\r\n{lines}\r\n--{BOUNDARY}--\r\n"
    );
    let request = Request::builder()
        .method("POST")
        .uri(format!(
            "/_api/database/shop/collection/invoices/import{query}"
        ))
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("Authorization", format!("Bearer {}", node.token))
        .body(Body::from(body))
        .unwrap();
    let response = node.app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_numbers_what_it_creates_and_says_why_it_fails() {
    let n = node(None, false);
    call(
        &n,
        "POST",
        COUNTERS,
        Some(json!({"field": "number", "prefix": "N-", "padding": 3})),
    )
    .await;
    call(&n, "POST", DOCS, Some(json!({"_key": "a"}))).await;

    // Upsert mode: the stored document keeps its number, the new ones get
    // the next.
    let (status, body) = import(
        &n,
        "?mode=upsert",
        "{\"_key\":\"a\",\"paid\":true}\n{\"_key\":\"b\"}\n{\"_key\":\"c\"}",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["count"], 3);
    assert_eq!(body["failed"], 0);
    assert_eq!(body.get("error"), None);
    let (_, rows) = query(
        &n,
        "FOR d IN invoices SORT d._key RETURN [d._key, d.number, d.paid]",
    )
    .await;
    assert_eq!(
        rows["result"],
        json!([
            ["a", "N-001", true],
            ["b", "N-002", null],
            ["c", "N-003", null]
        ])
    );

    // A document that carries a number is refused, and the answer says why.
    let (status, body) = import(&n, "?mode=upsert", "{\"_key\":\"d\",\"number\":\"N-900\"}").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["failed"], 1);
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("the database assigns it")),
        "{body}"
    );

    // Insert mode numbers too.
    let (_, body) = import(&n, "", "{\"label\":\"x\"}\n{\"label\":\"y\"}").await;
    assert_eq!(body["count"], 2);
    let (_, rows) = query(&n, "FOR d IN invoices SORT d.number RETURN d.number").await;
    assert_eq!(
        rows["result"],
        json!(["N-001", "N-002", "N-003", "N-004", "N-005"])
    );
}

/// The last line of an imported file is a document whether or not the file
/// ends with a newline: it used to be dropped, counted neither as imported
/// nor as failed. And a line that is not JSON is reported with its reason.
#[tokio::test(flavor = "multi_thread")]
async fn an_import_takes_a_last_line_without_a_newline() {
    let n = node(None, false);
    let (status, body) = import(&n, "", "{\"_key\":\"a\"}\n{\"_key\":\"b\"}").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((&body["count"], &body["failed"]), (&json!(2), &json!(0)));
    let (_, rows) = query(&n, "FOR d IN invoices SORT d._key RETURN d._key").await;
    assert_eq!(rows["result"], json!(["a", "b"]));

    let (_, body) = import(&n, "", "{\"_key\":\"c\"}\n{\"_key\": oops").await;
    assert_eq!((&body["count"], &body["failed"]), (&json!(1), &json!(1)));
    assert!(body["error"].is_string(), "{body}");
}

/// The node that numbers is lost: another takes the counter over and goes
/// on after the highest number it holds, which is all it knows.
#[tokio::test(flavor = "multi_thread")]
async fn a_counter_is_taken_over_from_a_lost_node() {
    let n1 = node(Some("n1"), true);
    let n2 = node(Some("n2"), true);
    let body = json!({"field": "number", "reset": "year", "prefix": "{YYYY}/", "padding": 3});
    call(&n1, "POST", COUNTERS, Some(body)).await;
    for _ in 0..3 {
        call(&n1, "POST", DOCS, Some(json!({}))).await;
    }
    // What reached n2 before n1 was lost: the counter and three documents.
    let reached = entries(&n1);
    apply(&n2, &reached);
    // n1 gives two more numbers that never leave it.
    call(&n1, "POST", DOCS, Some(json!({"_key": "late-a"}))).await;
    call(&n1, "POST", DOCS, Some(json!({"_key": "late-b"}))).await;

    let node_url = format!("{COUNTERS}/number/node");
    // Without `force` only the node that numbers hands the counter over.
    let (status, _) = call(&n2, "PUT", &node_url, Some(json!({"node": "n2"}))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, body) = call(&n2, "PUT", &node_url, Some(json!({"force": true}))).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["node"], "n2");
    // n2 was last told "next is 1": the three documents it holds say 4.
    assert_eq!(body["state"]["next"], 4);
    let (status, doc) = call(&n2, "POST", DOCS, Some(json!({}))).await;
    assert!(status.is_success(), "{status} {doc}");
    let number = doc["number"].as_str().unwrap().to_string();
    assert!(number.ends_with("/004"), "{number}");

    // Taking it twice changes nothing more.
    let (status, _) = call(&n2, "PUT", &node_url, Some(json!({"force": true}))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // n1 comes back and hears of it: it no longer numbers.
    apply(&n1, &entries(&n2));
    let (status, body) = call(&n1, "POST", DOCS, Some(json!({}))).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.to_string().contains("n2"), "{body}");

    // What the takeover cannot know: the number n1 gave to a document that
    // had not reached n2 was given again. Both documents exist once n1's
    // log arrives, with the same number.
    apply(&n2, &entries(&n1));
    let (_, rows) = query(
        &n2,
        &format!("FOR d IN invoices FILTER d.number == '{number}' RETURN d._key"),
    )
    .await;
    assert_eq!(rows["result"].as_array().map(Vec::len), Some(2), "{rows}");
}
