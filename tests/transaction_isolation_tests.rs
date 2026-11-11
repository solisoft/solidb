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

/// Run `workers` tasks, each trying `rounds` transactions at `level`:
/// `query(worker)` then commit. Returns how many committed.
async fn hammer(
    app: &Arc<App>,
    level: &'static str,
    workers: usize,
    rounds: usize,
    query: fn(usize) -> &'static str,
) -> usize {
    let mut tasks = Vec::new();
    for worker in 0..workers {
        let app = Arc::clone(app);
        tasks.push(tokio::spawn(async move {
            let mut committed = 0;
            for _ in 0..rounds {
                let tx = app.begin(level).await;
                let (status, _) = app.query(&tx, query(worker)).await;
                if status == StatusCode::OK && app.commit(&tx).await == StatusCode::OK {
                    committed += 1;
                }
            }
            committed
        }));
    }
    let mut committed = 0;
    for task in tasks {
        committed += task.await.unwrap();
    }
    committed
}

/// Concurrent transactions that each count a collection (a range read) and
/// insert the count into it. Run one after another, every committed one
/// sees a different count; two that commit the same count both read the
/// collection before either wrote it.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_counts_of_a_collection_are_all_different_at_serializable() {
    let app = Arc::new(App::new().await);
    let committed = hammer(&app, "serializable", 8, 60, |_| {
        "LET n = LENGTH(FOR l IN log RETURN 1) INSERT {seq: n} INTO log RETURN n"
    })
    .await;
    assert!(committed > 0, "nothing committed");

    let (_, out) = app.cursor("FOR l IN log SORT l.seq RETURN l.seq").await;
    let seqs: Vec<u64> = out["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap())
        .collect();
    let expected: Vec<u64> = (0..committed as u64).collect();
    assert_eq!(seqs, expected, "{committed} committed");
}

/// Two kinds of transactions across two collections: one counts `log` and
/// inserts into `doctors`, the other counts `doctors` and inserts into
/// `log`. In any serial order, a row's count is the number of rows of the
/// other kind before it; replaying the rows must never find one of each
/// kind that both saw the state before the other (write skew).
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_cross_collection_counts_have_a_serial_order_at_serializable() {
    let app = Arc::new(App::new().await);
    let (status, _) = app.cursor("FOR d IN doctors REMOVE d IN doctors").await;
    assert!(status.is_success());
    let committed = hammer(&app, "serializable", 8, 60, |worker| {
        if worker % 2 == 0 {
            "LET n = LENGTH(FOR l IN log RETURN 1) INSERT {saw: n} INTO doctors RETURN n"
        } else {
            "LET n = LENGTH(FOR d IN doctors RETURN 1) INSERT {saw: n} INTO log RETURN n"
        }
    })
    .await;
    assert!(committed > 0, "nothing committed");

    let saw = |collection: &'static str| {
        let app = Arc::clone(&app);
        async move {
            let (_, out) = app
                .cursor(&format!("FOR x IN {collection} SORT x.saw RETURN x.saw"))
                .await;
            out["result"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as usize)
                .collect::<Vec<_>>()
        }
    };
    // A row of `doctors` saw a number of `log` rows, and the reverse.
    let (doctors, log) = (saw("doctors").await, saw("log").await);
    assert_eq!(doctors.len() + log.len(), committed);
    let (mut d, mut l) = (0, 0);
    while d < doctors.len() || l < log.len() {
        let doctor_next = d < doctors.len() && doctors[d] == l;
        let log_next = l < log.len() && log[l] == d;
        assert!(
            doctor_next != log_next,
            "no serial order after {d} doctors and {l} log rows: doctors saw {doctors:?}, log saw {log:?}"
        );
        if doctor_next {
            d += 1;
        } else {
            l += 1;
        }
    }
}

/// At `read_committed`, an `UPDATE` in a transaction waits for the lock
/// another transaction holds on its document, then is evaluated again on
/// what that transaction committed: no refusal, and no lost update.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_update_waits_for_the_lock_and_builds_on_what_was_committed() {
    let app = Arc::new(App::new().await);
    let bump = "FOR d IN doctors FILTER d._key == 'alice' \
                UPDATE d WITH {shifts: d.shifts + 1} IN doctors RETURN NEW.shifts";
    let first = app.begin("read_committed").await;
    assert_eq!(app.rows(&first, bump).await, json!([1]));

    // The second transaction's statement blocks on alice's lock.
    let second = app.begin("read_committed").await;
    let waiting = {
        let (app, second) = (Arc::clone(&app), second.clone());
        tokio::spawn(async move { app.query(&second, bump).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!waiting.is_finished(), "it did not wait for the lock");

    assert_eq!(app.commit(&first).await, StatusCode::OK);
    let (status, out) = waiting.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["result"], json!([2]), "computed from the committed 1");
    assert_eq!(app.commit(&second).await, StatusCode::OK);
    let (_, out) = app.cursor("RETURN DOCUMENT('doctors/alice').shifts").await;
    assert_eq!(out["result"], json!([2]));
}

/// At `serializable` the same update does not wait: once the first
/// transaction commits, what the second read is stale, and it would be
/// refused anyway. It is refused at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn at_serializable_the_update_is_refused_without_waiting() {
    let app = Arc::new(App::new().await);
    let bump = "FOR d IN doctors FILTER d._key == 'alice' \
                UPDATE d WITH {shifts: d.shifts + 1} IN doctors RETURN NEW.shifts";
    let first = app.begin("serializable").await;
    assert_eq!(app.rows(&first, bump).await, json!([1]));
    let second = app.begin("serializable").await;
    let started = std::time::Instant::now();
    let (status, out) = app.query(&second, bump).await;
    assert_eq!(status, StatusCode::CONFLICT, "{out}");
    assert!(started.elapsed() < std::time::Duration::from_millis(250));
    assert_eq!(app.commit(&first).await, StatusCode::OK);
}

/// A filter that no longer holds on the committed document leaves the row
/// alone: two transactions cannot both take the last shift.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_row_that_no_longer_passes_its_filter_is_not_written() {
    let app = Arc::new(App::new().await);
    let take = "FOR d IN doctors FILTER d._key == 'alice' FILTER d.on_call == true \
                UPDATE d WITH {on_call: false} IN doctors RETURN 1";
    let first = app.begin("read_committed").await;
    assert_eq!(app.rows(&first, take).await, json!([1]));
    let second = app.begin("read_committed").await;
    let waiting = {
        let (app, second) = (Arc::clone(&app), second.clone());
        tokio::spawn(async move { app.query(&second, take).await })
    };
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(app.commit(&first).await, StatusCode::OK);
    let (status, out) = waiting.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{out}");
    assert_eq!(out["result"], json!([]), "alice was already off call");
    assert_eq!(app.commit(&second).await, StatusCode::OK);
}

/// Many transactions incrementing one document: every one commits (none
/// is refused for a lock, they wait), and none of the increments is lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_increments_all_commit_and_none_is_lost() {
    let app = Arc::new(App::new().await);
    let committed = hammer(&app, "read_committed", 8, 40, |_| {
        "FOR d IN doctors FILTER d._key == 'bob' \
         UPDATE d WITH {shifts: d.shifts + 1} IN doctors RETURN 1"
    })
    .await;
    assert_eq!(committed, 8 * 40, "a transaction was refused");
    let (_, out) = app.cursor("RETURN DOCUMENT('doctors/bob').shifts").await;
    assert_eq!(out["result"], json!([committed]));
}
