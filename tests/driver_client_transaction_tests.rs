//! Transactions through the Rust client, over a real driver connection
//! (card #793).
//!
//! The client used to keep the transaction's id and nothing else: its
//! writes went out as plain commands, so they applied at once and survived a
//! rollback, and on a pool the commit landed on a connection that had never
//! heard of the transaction. These pin down what an application now gets:
//! nothing visible before the commit, everything at the commit, nothing
//! after a rollback, on a pool as on one connection, and a refusal for what
//! cannot be staged.

use serde_json::json;
use solidb::driver::handlers::DriverHandler;
use solidb::storage::StorageEngine;
use solidb_client::{SoliDBClient, SoliDBClientBuilder};
use std::sync::Arc;
use tempfile::TempDir;

const DB: &str = "app";
const COLL: &str = "docs";
const PASSWORD: &str = "client-tx-password";

/// A server with `app.docs` holding `base` ({n: 1}) and `gone`, listening on
/// a local port, with the runtime its connections run on.
fn serve() -> (tokio::runtime::Runtime, String, Arc<StorageEngine>, TempDir) {
    std::env::set_var("SOLIDB_ADMIN_PASSWORD", PASSWORD);
    let tmp = TempDir::new().unwrap();
    let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    engine.create_database("_system".to_string()).unwrap();
    solidb::server::auth::AuthService::init(&engine, None, tmp.path().to_str().unwrap()).unwrap();
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection(COLL.to_string(), None).unwrap();
    let coll = db.get_collection(COLL).unwrap();
    coll.insert(json!({"_key": "base", "n": 1})).unwrap();
    coll.insert(json!({"_key": "gone", "n": 1})).unwrap();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let listener = rt
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let storage = engine.clone();
    rt.spawn(async move {
        use tokio::io::AsyncReadExt;
        loop {
            let (mut stream, peer) = listener.accept().await.unwrap();
            let storage = storage.clone();
            tokio::spawn(async move {
                let mut magic = [0u8; 14];
                stream.read_exact(&mut magic).await.unwrap();
                let mut handler = DriverHandler::new(storage, None);
                handler.handle_connection(stream, peer.to_string()).await;
            });
        }
    });
    (rt, addr, engine, tmp)
}

async fn client(addr: &str, pool: usize) -> SoliDBClient {
    SoliDBClientBuilder::new(addr)
        .auth(DB, "admin", PASSWORD)
        .pool_size(pool)
        .build()
        .await
        .unwrap()
}

/// `n` of `key` as another connection reads it, or None when it is absent.
async fn seen(observer: &mut SoliDBClient, key: &str) -> Option<i64> {
    observer
        .get(DB, COLL, key)
        .await
        .ok()
        .map(|d| d["n"].as_i64().unwrap_or(-1))
}

#[test]
fn a_rollback_leaves_nothing_and_nothing_shows_before_it() {
    let (rt, addr, _engine, _tmp) = serve();
    rt.block_on(async {
        let mut c = client(&addr, 1).await;
        let mut observer = client(&addr, 1).await;

        c.begin_transaction(DB, None).await.unwrap();
        assert!(c.in_transaction());
        c.insert(DB, COLL, Some("new"), json!({"n": 7}))
            .await
            .unwrap();
        c.update(DB, COLL, "base", json!({"n": 2}), true)
            .await
            .unwrap();
        c.delete(DB, COLL, "gone").await.unwrap();

        // Staged, not applied.
        assert_eq!(seen(&mut observer, "new").await, None);
        assert_eq!(seen(&mut observer, "base").await, Some(1));
        assert_eq!(seen(&mut observer, "gone").await, Some(1));

        c.rollback().await.unwrap();
        assert!(!c.in_transaction());
        assert_eq!(seen(&mut observer, "new").await, None);
        assert_eq!(seen(&mut observer, "base").await, Some(1));
        assert_eq!(seen(&mut observer, "gone").await, Some(1));
    });
}

#[test]
fn a_commit_applies_every_write_at_once() {
    let (rt, addr, engine, _tmp) = serve();
    rt.block_on(async {
        let mut c = client(&addr, 1).await;
        let mut observer = client(&addr, 1).await;

        c.begin_transaction(DB, None).await.unwrap();
        let staged = c
            .insert(DB, COLL, Some("a"), json!({"n": 3}))
            .await
            .unwrap();
        assert_eq!(staged["_key"], "a");
        c.insert(DB, COLL, Some("b"), json!({"n": 4}))
            .await
            .unwrap();
        c.update(DB, COLL, "base", json!({"n": 2}), true)
            .await
            .unwrap();
        c.delete(DB, COLL, "gone").await.unwrap();
        assert_eq!(seen(&mut observer, "a").await, None);
        assert_eq!(seen(&mut observer, "base").await, Some(1));

        c.commit().await.unwrap();
        assert_eq!(seen(&mut observer, "a").await, Some(3));
        assert_eq!(seen(&mut observer, "b").await, Some(4));
        assert_eq!(seen(&mut observer, "base").await, Some(2));
        assert_eq!(seen(&mut observer, "gone").await, None);
        let coll = engine
            .get_database(DB)
            .unwrap()
            .get_collection(COLL)
            .unwrap();
        assert_eq!(coll.count(), 3);
    });
}

/// The server keeps a driver transaction per connection; a client with a
/// pool must keep every command of it, its commit and its rollback on the
/// connection that began it, whatever else goes out in between.
#[test]
fn a_pooled_client_keeps_its_transaction_on_one_connection() {
    let (rt, addr, _engine, _tmp) = serve();
    rt.block_on(async {
        let mut c = client(&addr, 4).await;
        let mut observer = client(&addr, 1).await;
        for i in 0..6 {
            let key = format!("p{i}");
            c.begin_transaction(DB, None).await.unwrap();
            // Reads outside the transaction move the pool's round robin.
            assert_eq!(c.get(DB, COLL, "base").await.unwrap()["n"], 1);
            c.insert(DB, COLL, Some(&key), json!({"n": i}))
                .await
                .unwrap();
            assert_eq!(c.get(DB, COLL, "base").await.unwrap()["n"], 1);
            assert_eq!(seen(&mut observer, &key).await, None);
            if i % 2 == 0 {
                c.commit().await.unwrap();
                assert_eq!(seen(&mut observer, &key).await, Some(i));
            } else {
                c.rollback().await.unwrap();
                assert_eq!(seen(&mut observer, &key).await, None);
            }
        }
    });
}

#[test]
fn what_cannot_be_staged_is_refused() {
    let (rt, addr, engine, _tmp) = serve();
    rt.block_on(async {
        let mut c = client(&addr, 2).await;
        c.begin_transaction(DB, None).await.unwrap();

        // Cannot be staged: refused by the client, nothing written.
        let err = c
            .bulk_insert(DB, COLL, vec![json!({"_key": "bulk", "n": 1})])
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot run inside a transaction"), "{err}");

        // A query that writes: refused by the server, nothing written.
        let err = c
            .query(DB, "INSERT {_key: 'q', n: 1} INTO docs", None)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot run inside a transaction"), "{err}");

        // A read-only query runs, against committed data.
        let rows = c
            .query(DB, "FOR d IN docs FILTER d._key == 'base' RETURN d.n", None)
            .await
            .unwrap();
        assert_eq!(rows, vec![json!(1)]);

        // One transaction at a time.
        let err = c.begin_transaction(DB, None).await.unwrap_err().to_string();
        assert!(err.contains("already open"), "{err}");

        c.rollback().await.unwrap();
        let coll = engine
            .get_database(DB)
            .unwrap()
            .get_collection(COLL)
            .unwrap();
        assert!(coll.get("bulk").is_err() && coll.get("q").is_err());
        assert_eq!(coll.count(), 2);

        // Outside a transaction, the same calls apply at once.
        assert_eq!(
            c.bulk_insert(DB, COLL, vec![json!({"_key": "bulk", "n": 1})])
                .await
                .unwrap(),
            1
        );
        let err = c.commit().await.unwrap_err().to_string();
        assert!(err.contains("No active transaction"), "{err}");
    });
}
