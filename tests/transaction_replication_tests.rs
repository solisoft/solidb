//! What a committed transaction puts in the replication log.
//!
//! The log is fed by the handlers that take a write, and a transaction's
//! writes are stored by its commit, below them: they were stored and never
//! logged, so a peer never received them. The commit now reports every
//! write it stored (`group_commit::prepare`, `whole`).

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use serde_json::{json, Value};
use solidb::scripting::ScriptStats;
use solidb::server::routes::create_router;
use solidb::storage::collection::counters::CounterSpec;
use solidb::storage::StorageEngine;
use solidb::sync::log::SyncLog;
use solidb::sync::protocol::Operation;
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

mod common;

struct App {
    router: axum::Router,
    token: String,
    engine: StorageEngine,
    log: Arc<SyncLog>,
    _tmp: TempDir,
}

const BASE: &str = "/_api/database/_system";

impl App {
    fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let engine = StorageEngine::new(tmp.path().join("data").to_str().unwrap()).unwrap();
        engine.initialize().unwrap();
        let log = Arc::new(
            SyncLog::new(
                "n1".to_string(),
                tmp.path().join("log").to_str().unwrap(),
                64,
            )
            .unwrap(),
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
        let token = common::seed_user_token(&engine, "tx_admin", &["admin"]);
        App {
            router,
            token,
            engine,
            log,
            _tmp: tmp,
        }
    }

    async fn call(&self, method: &str, uri: &str, body: Value) -> (StatusCode, Value) {
        let response = self
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {}", self.token))
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn ok(&self, method: &str, uri: &str, body: Value) -> Value {
        let (status, out) = self.call(method, uri, body).await;
        assert!(status.is_success(), "{method} {uri}: {status} {out}");
        out
    }

    async fn begin(&self) -> String {
        let out = self
            .ok("POST", &format!("{BASE}/transaction/begin"), json!({}))
            .await;
        out["id"].as_str().unwrap().to_string()
    }

    async fn in_tx(&self, tx: &str, query: &str) {
        self.ok(
            "POST",
            &format!("{BASE}/transaction/{tx}/query"),
            json!({ "query": query }),
        )
        .await;
    }

    /// The document writes logged after `from`, as (collection, operation,
    /// key, document).
    fn logged(&self, from: u64) -> Vec<(String, Operation, String, Option<Value>)> {
        self.log
            .get_entries_after(from, 1000)
            .into_iter()
            .filter(|e| {
                matches!(
                    e.operation,
                    Operation::Insert | Operation::Update | Operation::Delete
                )
            })
            .map(|e| {
                let document = e
                    .data
                    .as_deref()
                    .and_then(|d| serde_json::from_slice(d).ok());
                (e.collection, e.operation, e.key, document)
            })
            .collect()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_committed_transaction_is_logged_for_the_peers() {
    let app = App::new();
    for name in ["accounts", "ledger"] {
        app.ok(
            "POST",
            &format!("{BASE}/collection"),
            json!({ "name": name }),
        )
        .await;
    }
    for key in ["a", "b", "gone"] {
        app.ok(
            "POST",
            &format!("{BASE}/document/accounts"),
            json!({"_key": key, "balance": 100}),
        )
        .await;
    }
    let before = app.log.current_sequence();

    // A transaction that is rolled back logs nothing.
    let tx = app.begin().await;
    app.in_tx(&tx, "INSERT {_key: 'never'} INTO ledger").await;
    app.ok(
        "POST",
        &format!("{BASE}/transaction/{tx}/rollback"),
        json!({}),
    )
    .await;
    assert_eq!(app.logged(before), vec![]);

    // Inserts by query and by the document endpoint, updates, a delete.
    let tx = app.begin().await;
    app.in_tx(&tx, "UPDATE 'a' WITH {balance: 70} IN accounts")
        .await;
    app.in_tx(&tx, "UPDATE 'b' WITH {balance: 130} IN accounts")
        .await;
    app.in_tx(
        &tx,
        "INSERT {_key: 't1', from: 'a', to: 'b', amount: 30} INTO ledger",
    )
    .await;
    app.ok(
        "POST",
        &format!("{BASE}/transaction/{tx}/document/ledger"),
        json!({"_key": "t2", "note": "by the document endpoint"}),
    )
    .await;
    app.in_tx(&tx, "REMOVE 'gone' IN accounts").await;
    // Nothing is logged before the commit.
    assert_eq!(app.logged(before), vec![]);
    app.ok(
        "POST",
        &format!("{BASE}/transaction/{tx}/commit"),
        json!({}),
    )
    .await;

    let logged = app.logged(before);
    let summary: Vec<(String, String, bool)> = logged
        .iter()
        .map(|(collection, operation, key, document)| {
            (
                format!("{collection}/{key}"),
                format!("{:?}", operation == &Operation::Delete),
                document.is_some(),
            )
        })
        .collect();
    let mut expected = vec![
        ("accounts/a".to_string(), "false".to_string(), true),
        ("accounts/b".to_string(), "false".to_string(), true),
        ("accounts/gone".to_string(), "true".to_string(), false),
        ("ledger/t1".to_string(), "false".to_string(), true),
        ("ledger/t2".to_string(), "false".to_string(), true),
    ];
    let mut got = summary.clone();
    got.sort();
    expected.sort();
    assert_eq!(got, expected, "{logged:?}");
    // The documents as they are stored.
    let stored = |collection: &str, key: &str| {
        app.engine
            .get_database("_system")
            .unwrap()
            .get_collection(collection)
            .unwrap()
            .get(key)
            .unwrap()
            .to_value()
    };
    for (collection, _, key, document) in &logged {
        if let Some(document) = document {
            assert_eq!(document, &stored(collection, key), "{collection}/{key}");
        }
    }
    assert_eq!(stored("accounts", "a")["balance"], json!(70));
}

/// A peer that applies what the commit logged holds what the origin holds,
/// counter numbers included: they are given at commit.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_applies_the_log_holds_the_transactions_writes() {
    let app = App::new();
    app.ok(
        "POST",
        &format!("{BASE}/collection"),
        json!({"name": "invoices"}),
    )
    .await;
    let origin = app
        .engine
        .get_database("_system")
        .unwrap()
        .get_collection("invoices")
        .unwrap();
    let spec: CounterSpec =
        serde_json::from_value(json!({"field": "number", "prefix": "INV-"})).unwrap();
    origin.add_counter(spec).unwrap();
    let before = app.log.current_sequence();

    let tx = app.begin().await;
    app.in_tx(&tx, "INSERT {_key: 'i1', total: 10} INTO invoices")
        .await;
    app.in_tx(&tx, "INSERT {_key: 'i2', total: 20} INTO invoices")
        .await;
    app.in_tx(&tx, "UPDATE 'i1' WITH {total: 15} IN invoices")
        .await;
    app.ok(
        "POST",
        &format!("{BASE}/transaction/{tx}/commit"),
        json!({}),
    )
    .await;

    let tmp = TempDir::new().unwrap();
    let peer = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
    peer.initialize().unwrap();
    let db = peer.get_database("_system").unwrap();
    db.create_collection("invoices".to_string(), None).unwrap();
    let copy = db.get_collection("invoices").unwrap();
    for (_, operation, key, document) in app.logged(before) {
        solidb::storage::foreign_keys::unchecked(|| match (operation, document) {
            (Operation::Delete, _) => copy.delete(&key).map(|_| ()),
            (_, Some(document)) => copy.upsert_batch(vec![(key, document)]).map(|_| ()),
            _ => Ok(()),
        })
        .unwrap();
    }
    for key in ["i1", "i2"] {
        assert_eq!(
            copy.get(key).unwrap().data,
            origin.get(key).unwrap().data,
            "{key}"
        );
    }
    assert_eq!(origin.get("i1").unwrap().data["number"], json!("INV-1"));
    assert_eq!(copy.get("i1").unwrap().data["total"], json!(15));
    assert_eq!(copy.get("i2").unwrap().data["number"], json!("INV-2"));
}

/// A delete that cascades inside a transaction: the parent and the
/// children are each logged once.
#[tokio::test(flavor = "multi_thread")]
async fn a_cascade_inside_a_transaction_is_logged_once() {
    let app = App::new();
    for name in ["customers", "orders"] {
        app.ok(
            "POST",
            &format!("{BASE}/collection"),
            json!({ "name": name }),
        )
        .await;
    }
    app.ok(
        "POST",
        &format!("{BASE}/collection/orders/foreign-keys"),
        json!({"field": "customer", "references": "customers", "on_delete": "cascade"}),
    )
    .await;
    app.ok(
        "POST",
        &format!("{BASE}/document/customers"),
        json!({"_key": "ann"}),
    )
    .await;
    for key in ["o1", "o2"] {
        app.ok(
            "POST",
            &format!("{BASE}/document/orders"),
            json!({"_key": key, "customer": "ann"}),
        )
        .await;
    }
    let before = app.log.current_sequence();
    let tx = app.begin().await;
    app.in_tx(&tx, "REMOVE 'ann' IN customers").await;
    app.ok(
        "POST",
        &format!("{BASE}/transaction/{tx}/commit"),
        json!({}),
    )
    .await;
    let mut deleted: Vec<String> = app
        .logged(before)
        .into_iter()
        .map(|(collection, operation, key, _)| {
            assert_eq!(operation, Operation::Delete);
            format!("{collection}/{key}")
        })
        .collect();
    deleted.sort();
    assert_eq!(deleted, ["customers/ann", "orders/o1", "orders/o2"]);
}

/// A node on its own keeps no log (it is deferred until something reads
/// it): its commits report nothing. Once the log is active they do.
#[tokio::test(flavor = "multi_thread")]
async fn a_deferred_log_takes_nothing_and_an_active_one_takes_the_commit() {
    let app = App::new();
    app.ok(
        "POST",
        &format!("{BASE}/collection"),
        json!({"name": "notes"}),
    )
    .await;
    app.log.defer_until_needed();
    assert!(!app.log.is_active());
    let before = app.log.current_sequence();
    let tx = app.begin().await;
    app.in_tx(&tx, "INSERT {_key: 'quiet'} INTO notes").await;
    app.ok(
        "POST",
        &format!("{BASE}/transaction/{tx}/commit"),
        json!({}),
    )
    .await;
    assert_eq!(app.logged(before), vec![]);

    app.log.activate("a peer connected");
    let tx = app.begin().await;
    app.in_tx(&tx, "INSERT {_key: 'heard'} INTO notes").await;
    app.ok(
        "POST",
        &format!("{BASE}/transaction/{tx}/commit"),
        json!({}),
    )
    .await;
    let keys: Vec<String> = app
        .logged(before)
        .into_iter()
        .map(|(_, _, key, _)| key)
        .collect();
    assert_eq!(keys, ["heard"]);
}
