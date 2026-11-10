//! What `repeatable_read` and `serializable` add to a transaction's queries
//! (card #802): what they read is recorded and checked at commit.
//!
//! - A document read by key must still be at the revision read (or still be
//!   absent) when the transaction commits.
//! - A collection read any other way (a scan, an index) must not have been
//!   written at all since the transaction first read it.
//! - At every level, a query reads by key what its own transaction staged.

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

const DB: &str = "iso";

struct App {
    router: axum::Router,
    token: String,
    _tmp: TempDir,
}

impl App {
    /// A server with database `iso` and collection `doctors`: alice and bob,
    /// both on call.
    async fn new() -> Self {
        let tmp = TempDir::new().expect("temp dir");
        let engine = StorageEngine::new(tmp.path().to_str().unwrap()).expect("storage engine");
        engine.initialize().expect("initialize");
        let router = create_router(
            engine.clone(),
            None,
            None,
            None,
            None,
            Arc::new(ScriptStats::default()),
            None,
            None,
            0,
        );
        let token = common::seed_user_token(&engine, "test_admin", &["admin"]);
        let app = App {
            router,
            token,
            _tmp: tmp,
        };
        app.call("POST", "/_api/database", json!({"name": DB}))
            .await;
        for collection in ["doctors", "log"] {
            app.call(
                "POST",
                &format!("/_api/database/{DB}/collection"),
                json!({"name": collection}),
            )
            .await;
        }
        let (status, out) = app
            .cursor(
                "FOR k IN ['alice', 'bob'] INSERT {_key: k, on_call: true, shifts: 0} INTO doctors",
            )
            .await;
        assert!(status.is_success(), "seeding failed: {out}");
        app
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
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    /// A query outside any transaction.
    async fn cursor(&self, query: &str) -> (StatusCode, Value) {
        self.call(
            "POST",
            &format!("/_api/database/{DB}/cursor"),
            json!({"query": query}),
        )
        .await
    }

    async fn begin(&self, level: &str) -> String {
        let (status, out) = self
            .call(
                "POST",
                &format!("/_api/database/{DB}/transaction/begin"),
                json!({"isolationLevel": level}),
            )
            .await;
        assert!(status.is_success(), "begin failed: {out}");
        out["id"].as_str().unwrap().to_string()
    }

    async fn query(&self, tx: &str, query: &str) -> (StatusCode, Value) {
        self.call(
            "POST",
            &format!("/_api/database/{DB}/transaction/{tx}/query"),
            json!({"query": query}),
        )
        .await
    }

    /// A query in `tx` that has to succeed; its rows.
    async fn rows(&self, tx: &str, query: &str) -> Value {
        let (status, out) = self.query(tx, query).await;
        assert_eq!(status, StatusCode::OK, "{query}: {out}");
        out["result"].clone()
    }

    async fn commit(&self, tx: &str) -> StatusCode {
        self.call(
            "POST",
            &format!("/_api/database/{DB}/transaction/{tx}/commit"),
            json!({}),
        )
        .await
        .0
    }

    async fn on_call(&self) -> Vec<String> {
        let (_, out) = self
            .cursor("FOR d IN doctors FILTER d.on_call == true SORT d._key RETURN d._key")
            .await;
        out["result"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect()
    }
}

/// "Go off call if someone else stays on call", decided from a scan.
fn leave_if_covered(me: &str) -> String {
    format!(
        "FOR d IN doctors FILTER d._key == '{me}' \
         LET n = LENGTH(FOR x IN doctors FILTER x.on_call == true RETURN 1) \
         FILTER n >= 2 UPDATE d WITH {{on_call: false}} IN doctors RETURN 1"
    )
}

/// The same decision, reading the other doctor by key.
fn leave_if_other_on_call(me: &str, other: &str) -> String {
    format!(
        "FOR d IN doctors FILTER d._key == '{me}' \
         LET o = DOCUMENT('doctors/{other}') \
         FILTER o.on_call == true UPDATE d WITH {{on_call: false}} IN doctors RETURN 1"
    )
}

#[tokio::test]
async fn write_skew_through_a_scan_is_refused_at_serializable() {
    let app = App::new().await;
    let t1 = app.begin("serializable").await;
    let t2 = app.begin("serializable").await;
    assert_eq!(app.rows(&t1, &leave_if_covered("alice")).await, json!([1]));
    assert_eq!(app.rows(&t2, &leave_if_covered("bob")).await, json!([1]));

    assert_eq!(app.commit(&t1).await, StatusCode::OK);
    assert_eq!(app.commit(&t2).await, StatusCode::CONFLICT);
    assert_eq!(app.on_call().await, vec!["bob"]);
}

#[tokio::test]
async fn write_skew_through_reads_by_key_is_refused_at_repeatable_read() {
    let app = App::new().await;
    let t1 = app.begin("repeatable_read").await;
    let t2 = app.begin("repeatable_read").await;
    let leave_alice = leave_if_other_on_call("alice", "bob");
    let leave_bob = leave_if_other_on_call("bob", "alice");
    assert_eq!(app.rows(&t1, &leave_alice).await, json!([1]));
    assert_eq!(app.rows(&t2, &leave_bob).await, json!([1]));

    assert_eq!(app.commit(&t1).await, StatusCode::OK);
    assert_eq!(app.commit(&t2).await, StatusCode::CONFLICT);
    assert_eq!(app.on_call().await, vec!["bob"]);
}

/// `read_committed` is what it was: both commit, nobody is on call.
#[tokio::test]
async fn write_skew_goes_through_at_read_committed() {
    let app = App::new().await;
    let t1 = app.begin("read_committed").await;
    let t2 = app.begin("read_committed").await;
    app.rows(&t1, &leave_if_covered("alice")).await;
    app.rows(&t2, &leave_if_covered("bob")).await;

    assert_eq!(app.commit(&t1).await, StatusCode::OK);
    assert_eq!(app.commit(&t2).await, StatusCode::OK);
    assert!(app.on_call().await.is_empty());
}

/// Reading a document by key does not tie the transaction to the rest of
/// the collection: another document may be written meanwhile.
#[tokio::test]
async fn a_read_by_key_does_not_conflict_with_a_write_to_another_document() {
    let app = App::new().await;
    let tx = app.begin("serializable").await;
    let read = "FOR d IN doctors FILTER d._key == 'alice' RETURN d.shifts";
    assert_eq!(app.rows(&tx, read).await, json!([0]));
    app.rows(
        &tx,
        "FOR d IN doctors FILTER d._key == 'alice' UPDATE d WITH {shifts: d.shifts + 1} IN doctors RETURN NEW.shifts",
    )
    .await;

    let (status, _) = app.cursor("UPDATE 'bob' WITH {shifts: 7} IN doctors").await;
    assert!(status.is_success());
    let (status, _) = app
        .cursor("INSERT {_key: 'carol', on_call: true} INTO doctors")
        .await;
    assert!(status.is_success());

    assert_eq!(app.commit(&tx).await, StatusCode::OK);
}

#[tokio::test]
async fn a_document_read_by_key_that_changed_fails_the_commit() {
    let app = App::new().await;
    let tx = app.begin("serializable").await;
    let read = "FOR d IN doctors FILTER d._key == 'alice' RETURN d.shifts";
    assert_eq!(app.rows(&tx, read).await, json!([0]));
    app.rows(&tx, "INSERT {seen: 0} INTO log RETURN 1").await;

    let (status, _) = app
        .cursor("UPDATE 'alice' WITH {shifts: 5} IN doctors")
        .await;
    assert!(status.is_success());

    assert_eq!(app.commit(&tx).await, StatusCode::CONFLICT);
    let (_, log) = app.cursor("FOR l IN log RETURN l").await;
    assert_eq!(log["result"], json!([]), "nothing of it was written");
}

/// Reading it again is refused at once: the transaction cannot commit any
/// more, and must not go on with two versions of one document.
#[tokio::test]
async fn a_document_read_twice_is_the_same_or_a_conflict() {
    let app = App::new().await;
    let tx = app.begin("repeatable_read").await;
    let read = "FOR d IN doctors FILTER d._key == 'alice' RETURN d.shifts";
    assert_eq!(app.rows(&tx, read).await, json!([0]));
    assert_eq!(app.rows(&tx, read).await, json!([0]));

    app.cursor("UPDATE 'alice' WITH {shifts: 5} IN doctors")
        .await;

    let (status, out) = app.query(&tx, read).await;
    assert_eq!(status, StatusCode::CONFLICT, "{out}");
    assert_eq!(app.commit(&tx).await, StatusCode::CONFLICT);
}

/// Read in one statement, update in the next: the update is computed by the
/// client from the first read, so a write in between must not be overwritten.
#[tokio::test]
async fn read_then_update_in_two_statements_is_not_a_lost_update() {
    let app = App::new().await;
    let tx = app.begin("serializable").await;
    let read = "FOR d IN doctors FILTER d._key == 'alice' RETURN d.shifts";
    assert_eq!(app.rows(&tx, read).await, json!([0]));

    app.cursor("UPDATE 'alice' WITH {shifts: 5} IN doctors")
        .await;

    // 0 + 1, from the stale read.
    let (status, _) = app
        .query(&tx, "UPDATE 'alice' WITH {shifts: 1} IN doctors")
        .await;
    let committed = status == StatusCode::OK && app.commit(&tx).await == StatusCode::OK;
    assert!(!committed, "the stale update went through");
    let (_, out) = app.cursor("RETURN DOCUMENT('doctors/alice').shifts").await;
    assert_eq!(out["result"], json!([5]));
}

/// A key read as absent is a read too.
#[tokio::test]
async fn a_document_read_as_absent_that_appeared_fails_the_commit() {
    let app = App::new().await;
    let tx = app.begin("serializable").await;
    let read = "FOR d IN doctors FILTER d._key == 'dave' RETURN d";
    assert_eq!(app.rows(&tx, read).await, json!([]));
    app.rows(&tx, "INSERT {no: 'dave'} INTO log RETURN 1").await;

    let (status, _) = app.cursor("INSERT {_key: 'dave'} INTO doctors").await;
    assert!(status.is_success());

    assert_eq!(app.commit(&tx).await, StatusCode::CONFLICT);
}

/// A document that appears in a range the transaction read (a phantom).
#[tokio::test]
async fn a_phantom_fails_the_commit_at_serializable() {
    let app = App::new().await;
    let tx = app.begin("serializable").await;
    let rows = app
        .rows(
            &tx,
            "LET n = LENGTH(FOR d IN doctors FILTER d.on_call == true RETURN 1) \
             INSERT {on_call: n} INTO log RETURN n",
        )
        .await;
    assert_eq!(rows, json!([2]));

    let (status, _) = app
        .cursor("INSERT {_key: 'carol', on_call: true} INTO doctors")
        .await;
    assert!(status.is_success());

    assert_eq!(app.commit(&tx).await, StatusCode::CONFLICT);
}

/// Nothing changed under it: a transaction that read by range commits, and
/// so does one that only read.
#[tokio::test]
async fn an_undisturbed_transaction_commits() {
    let app = App::new().await;
    let tx = app.begin("serializable").await;
    assert_eq!(app.rows(&tx, &leave_if_covered("alice")).await, json!([1]));
    assert_eq!(app.commit(&tx).await, StatusCode::OK);
    assert_eq!(app.on_call().await, vec!["bob"]);

    let reader = app.begin("serializable").await;
    app.rows(&reader, "FOR d IN doctors RETURN d._key").await;
    assert_eq!(app.commit(&reader).await, StatusCode::OK);
}

/// A transaction's own writes do not count against its reads.
#[tokio::test]
async fn a_transaction_does_not_conflict_with_itself() {
    let app = App::new().await;
    let tx = app.begin("serializable").await;
    app.rows(&tx, "FOR d IN doctors RETURN d._key").await;
    app.rows(
        &tx,
        "FOR d IN doctors UPDATE d WITH {shifts: d.shifts + 1} IN doctors RETURN 1",
    )
    .await;
    app.rows(
        &tx,
        "INSERT {_key: 'erin', on_call: false} INTO doctors RETURN 1",
    )
    .await;
    assert_eq!(app.commit(&tx).await, StatusCode::OK);
}

/// At every level, a query reads by key what its transaction staged: an
/// update, an insert, and a delete (which reads as missing).
#[tokio::test]
async fn a_query_reads_its_transactions_own_writes_by_key() {
    for level in ["read_committed", "serializable"] {
        let app = App::new().await;
        let tx = app.begin(level).await;
        let bump =
            "FOR d IN doctors FILTER d._key == 'alice' UPDATE d WITH {shifts: d.shifts + 1} IN doctors RETURN NEW.shifts";
        assert_eq!(app.rows(&tx, bump).await, json!([1]), "{level}");
        assert_eq!(
            app.rows(
                &tx,
                "FOR d IN doctors FILTER d._key == 'alice' RETURN d.shifts"
            )
            .await,
            json!([1]),
            "{level}"
        );
        // Computed from the staged value, not from the stored one.
        assert_eq!(app.rows(&tx, bump).await, json!([2]), "{level}");

        app.rows(
            &tx,
            "INSERT {_key: 'frank', shifts: 9} INTO doctors RETURN 1",
        )
        .await;
        assert_eq!(
            app.rows(
                &tx,
                "FOR d IN doctors FILTER d._key == 'frank' RETURN d.shifts"
            )
            .await,
            json!([9]),
            "{level}"
        );

        app.rows(&tx, "REMOVE 'bob' IN doctors RETURN 1").await;
        assert_eq!(
            app.rows(&tx, "FOR d IN doctors FILTER d._key == 'bob' RETURN d._key")
                .await,
            json!([]),
            "{level}"
        );

        assert_eq!(app.commit(&tx).await, StatusCode::OK, "{level}");
        let (_, out) = app
            .cursor("FOR d IN doctors SORT d._key RETURN [d._key, d.shifts]")
            .await;
        assert_eq!(
            out["result"],
            json!([["alice", 2], ["frank", 9]]),
            "{level}"
        );
    }
}
