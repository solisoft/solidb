//! `faults probe-client-tx`: what the Rust client's transaction calls do.
//!
//! `SoliDBClient::begin_transaction` keeps the transaction's id, and
//! `commit` / `rollback` end it, but `insert`, `update` and `delete` never
//! say which transaction they belong to. The server keeps a driver
//! transaction per connection, and the client spreads commands over a pool.
//! The probe states what follows, against a real server.

use serde_json::{json, Value};
use solidb_client::SoliDBClient;

use crate::server::{Server, PASSWORD};
use crate::Res;

const DB: &str = "probe";

async fn client(server: &Server, pool: usize) -> Res<SoliDBClient> {
    let mut c = SoliDBClient::connect_with_pool(&server.addr(), pool).await?;
    c.auth("_system", "admin", PASSWORD).await?;
    Ok(c)
}

async fn exists(c: &mut SoliDBClient, key: &str) -> bool {
    c.get(DB, "docs", key).await.is_ok()
}

pub async fn run(server: &mut Server) -> Res<Value> {
    let http = reqwest::Client::new();
    server.start(&http).await?;
    let mut setup = client(server, 1).await?;
    setup.create_database(DB).await?;
    setup.create_collection(DB, "docs", None).await?;
    let mut other = client(server, 1).await?;

    // 1. A write between begin and rollback, on a single connection.
    let mut c = client(server, 1).await?;
    c.begin_transaction(DB, None).await?;
    c.insert(DB, "docs", Some("rolled-back"), json!({"n": 1}))
        .await?;
    let visible_before_end = exists(&mut other, "rolled-back").await;
    let rollback = c.rollback().await.map_err(|e| e.to_string());
    let present_after_rollback = exists(&mut other, "rolled-back").await;

    // 2. begin and commit on a pool of four connections.
    let mut pooled = client(server, 4).await?;
    let mut commits = Vec::new();
    for _ in 0..4 {
        pooled.begin_transaction(DB, None).await?;
        commits.push(pooled.commit().await.map_err(|e| e.to_string()));
    }

    server.kill().await?;
    Ok(json!({
        "write_between_begin_and_rollback": {
            "visible_to_another_connection_before_rollback": visible_before_end,
            "rollback": rollback,
            "present_after_rollback": present_after_rollback,
            "expected": "invisible before the end, absent after the rollback",
        },
        "begin_then_commit_on_a_pool_of_4": {
            "commits": commits,
            "expected": "every commit Ok",
        },
    }))
}
