//! Regression test for pooled authentication over the native driver protocol.
//!
//! Authentication is **per-socket** state: the server records it against the
//! connection, not against a client identity or a token. `auth()` used to send
//! its `Auth` command through the round-robin `send_command`, which authenticated
//! exactly one connection and left the rest of the pool bare. The next command
//! landed on a bare one and failed with `Authentication required`, so every
//! `pool_size > 1` client was unusable — including this crate's own `benchmark`
//! binary, which is why the TCP transport looked broken rather than merely
//! unmeasured.
//!
//! The test issues more commands than there are connections, so the round-robin
//! is guaranteed to visit every one. Against the unfixed client it fails on the
//! second command.
//!
//! Needs a live SoliDB, so it is `#[ignore]`d by default:
//!
//! ```sh
//! SOLIDB_TEST_ADDR=127.0.0.1:6745 cargo test --test pool_auth -- --ignored
//! ```

use solidb_client::SoliDBClientBuilder;

fn addr() -> Option<String> {
    std::env::var("SOLIDB_TEST_ADDR").ok()
}

const POOL: usize = 4;

#[tokio::test]
#[ignore = "needs a live SoliDB; set SOLIDB_TEST_ADDR"]
async fn auth_reaches_every_pooled_connection() {
    let Some(addr) = addr() else {
        eprintln!("SOLIDB_TEST_ADDR unset — skipping");
        return;
    };
    let db = std::env::var("SOLIDB_TEST_DB").unwrap_or_else(|_| "_system".into());

    let mut client = SoliDBClientBuilder::new(&addr)
        .use_tcp()
        .pool_size(POOL)
        .auth(&db, "admin", "admin")
        .build()
        .await
        .expect("build + auth should succeed");

    // One more command than there are connections, so every socket in the pool
    // is exercised at least once and a single unauthenticated one cannot hide.
    //
    // It has to be a command the server *gates on auth*: `ping` is answered on
    // an unauthenticated socket, so a ping-based version of this test passes
    // even with the bug reintroduced (verified). `query` is gated.
    for i in 0..=POOL {
        let rows = client
            .query(&db, "RETURN 1", None)
            .await
            .unwrap_or_else(|e| panic!("command {i} on a pooled connection failed: {e}"));
        assert_eq!(rows.len(), 1, "query {i} returned {} rows", rows.len());
    }
}

/// `pool_size(1)` was the only working configuration before the fix. Keep it
/// covered so a future change cannot regress the single-connection case while
/// fixing the pooled one.
#[tokio::test]
#[ignore = "needs a live SoliDB; set SOLIDB_TEST_ADDR"]
async fn auth_works_with_a_single_connection() {
    let Some(addr) = addr() else {
        eprintln!("SOLIDB_TEST_ADDR unset — skipping");
        return;
    };
    let db = std::env::var("SOLIDB_TEST_DB").unwrap_or_else(|_| "_system".into());

    let mut client = SoliDBClientBuilder::new(&addr)
        .use_tcp()
        .pool_size(1)
        .auth(&db, "admin", "admin")
        .build()
        .await
        .expect("build + auth should succeed");

    for i in 0..3 {
        client
            .query(&db, "RETURN 1", None)
            .await
            .unwrap_or_else(|e| panic!("query {i} failed: {e}"));
    }
}
