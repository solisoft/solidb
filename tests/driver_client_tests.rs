//! Driver Client Tests
//!
//! Tests for the SoliDB native driver client, including:
//! - SoliDBClientBuilder configuration
//! - Response extraction helpers
//! - Transaction state management
//! - Error handling

use serde_json::json;
use solidb::driver::protocol::{DriverError, Response};
use solidb_client::SoliDBClientBuilder;

// ============================================================================
// SoliDBClientBuilder Tests
// ============================================================================

#[test]
fn test_builder_new() {
    let _builder = SoliDBClientBuilder::new("localhost:6745");
}

#[test]
fn test_builder_with_auth() {
    let builder = SoliDBClientBuilder::new("localhost:6745").auth("mydb", "admin", "password");
    let _ = builder;
}

#[test]
fn test_builder_with_timeout() {
    let builder = SoliDBClientBuilder::new("localhost:6745").timeout_ms(5000);
    let _ = builder;
}

#[test]
fn test_builder_chained() {
    let builder = SoliDBClientBuilder::new("127.0.0.1:6745")
        .auth("production", "root", "secret123")
        .timeout_ms(10000);
    let _ = builder;
}

// ============================================================================
// Response Helper Function Tests
// ============================================================================

#[test]
fn test_response_ok_with_data() {
    let response = Response::ok(json!({"_key": "abc", "name": "Test"}));

    match response {
        Response::Ok { data, count, tx_id } => {
            assert!(data.is_some());
            let d = data.unwrap();
            assert_eq!(d["_key"], "abc");
            assert_eq!(d["name"], "Test");
            assert!(count.is_none());
            assert!(tx_id.is_none());
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_empty() {
    let response = Response::ok_empty();

    match response {
        Response::Ok { data, count, tx_id } => {
            assert!(data.is_none());
            assert!(count.is_none());
            assert!(tx_id.is_none());
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_count() {
    let response = Response::ok_count(42);

    match response {
        Response::Ok { data, count, tx_id } => {
            assert!(data.is_none());
            assert_eq!(count, Some(42));
            assert!(tx_id.is_none());
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_tx() {
    let response = Response::ok_tx("tx:12345".to_string());

    match response {
        Response::Ok { data, count, tx_id } => {
            assert!(data.is_none());
            assert!(count.is_none());
            assert_eq!(tx_id, Some("tx:12345".to_string()));
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_pong() {
    let response = Response::pong();

    match response {
        Response::Pong { timestamp } => {
            assert!(timestamp > 0);
        }
        _ => panic!("Expected Pong response"),
    }
}

#[test]
fn test_response_error() {
    let error = DriverError::DatabaseError("Collection not found".to_string());
    let response = Response::error(error);

    match response {
        Response::Error { error } => match error {
            DriverError::DatabaseError(msg) => {
                assert!(msg.contains("Collection not found"));
            }
            _ => panic!("Wrong error type"),
        },
        _ => panic!("Expected Error response"),
    }
}

#[test]
fn test_response_batch() {
    let responses = vec![
        Response::ok(json!({"id": 1})),
        Response::ok(json!({"id": 2})),
        Response::error(DriverError::DatabaseError("Not found".to_string())),
    ];

    let batch = Response::Batch { responses };

    match batch {
        Response::Batch { responses } => {
            assert_eq!(responses.len(), 3);
        }
        _ => panic!("Expected Batch response"),
    }
}

// ============================================================================
// DriverError Tests
// ============================================================================

#[test]
fn test_driver_error_display_connection() {
    let err = DriverError::ConnectionError("Connection timeout".to_string());
    let display = format!("{}", err);
    assert!(display.contains("Connection"));
    assert!(display.contains("timeout"));
}

#[test]
fn test_driver_error_display_protocol() {
    let err = DriverError::ProtocolError("Invalid magic header".to_string());
    let display = format!("{}", err);
    assert!(display.contains("Protocol"));
}

#[test]
fn test_driver_error_display_database() {
    let err = DriverError::DatabaseError("Document already exists".to_string());
    let display = format!("{}", err);
    assert!(display.contains("Database"));
}

#[test]
fn test_driver_error_display_auth() {
    let err = DriverError::AuthError("Invalid password".to_string());
    let display = format!("{}", err);
    assert!(display.contains("Auth"));
}

#[test]
fn test_driver_error_display_transaction() {
    let err = DriverError::TransactionError("Deadlock detected".to_string());
    let display = format!("{}", err);
    assert!(display.contains("Transaction"));
}

#[test]
fn test_driver_error_display_message_too_large() {
    let err = DriverError::MessageTooLarge;
    let display = format!("{}", err);
    assert!(display.contains("too large"));
}

#[test]
fn test_driver_error_display_invalid_command() {
    let err = DriverError::InvalidCommand("Unknown command type".to_string());
    let display = format!("{}", err);
    assert!(display.contains("Invalid command"));
}

// ============================================================================
// Connection Error Handling Tests (using async)
// ============================================================================

#[tokio::test]
async fn test_connect_invalid_address() {
    use solidb_client::SoliDBClient;

    // Try to connect to an invalid address
    let result = SoliDBClient::connect("invalid-host:99999").await;
    assert!(result.is_err());

    if let Err(err) = result {
        match err {
            solidb_client::DriverError::ConnectionError(msg) => {
                assert!(msg.contains("Failed to connect"));
            }
            _ => panic!("Expected ConnectionError"),
        }
    }
}

#[tokio::test]
async fn test_connect_refused() {
    use solidb_client::SoliDBClient;

    // Try to connect to a port that's likely not listening
    let result = SoliDBClient::connect("127.0.0.1:59999").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_builder_connect_invalid() {
    let result = SoliDBClientBuilder::new("invalid:99999").build().await;

    assert!(result.is_err());
}

// ============================================================================
// Response Data Extraction Edge Cases
// ============================================================================

#[test]
fn test_response_ok_with_nested_data() {
    let nested = json!({
        "user": {
            "profile": {
                "name": "Alice",
                "settings": {
                    "theme": "dark"
                }
            }
        }
    });

    let response = Response::ok(nested.clone());

    match response {
        Response::Ok { data, .. } => {
            let d = data.unwrap();
            assert_eq!(d["user"]["profile"]["name"], "Alice");
            assert_eq!(d["user"]["profile"]["settings"]["theme"], "dark");
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_with_array_data() {
    let array = json!([
        {"id": 1, "name": "First"},
        {"id": 2, "name": "Second"},
        {"id": 3, "name": "Third"}
    ]);

    let response = Response::ok(array);

    match response {
        Response::Ok { data, .. } => {
            let arr = data.unwrap();
            assert!(arr.is_array());
            assert_eq!(arr.as_array().unwrap().len(), 3);
            assert_eq!(arr[0]["id"], 1);
            assert_eq!(arr[2]["name"], "Third");
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_with_null_data() {
    let response = Response::ok(json!(null));

    match response {
        Response::Ok { data, .. } => {
            assert!(data.unwrap().is_null());
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_with_numeric_data() {
    let response = Response::ok(json!(42));

    match response {
        Response::Ok { data, .. } => {
            assert_eq!(data.unwrap(), 42);
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_with_string_data() {
    let response = Response::ok(json!("Hello, World!"));

    match response {
        Response::Ok { data, .. } => {
            assert_eq!(data.unwrap(), "Hello, World!");
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_with_boolean_data() {
    let response = Response::ok(json!(true));

    match response {
        Response::Ok { data, .. } => {
            assert_eq!(data.unwrap(), true);
        }
        _ => panic!("Expected Ok response"),
    }
}

// ============================================================================
// Large Data Tests
// ============================================================================

#[test]
fn test_response_ok_with_large_array() {
    let large_array: Vec<serde_json::Value> = (0..1000)
        .map(|i| json!({"id": i, "data": format!("item_{}", i)}))
        .collect();

    let response = Response::ok(json!(large_array));

    match response {
        Response::Ok { data, .. } => {
            let arr = data.unwrap();
            assert_eq!(arr.as_array().unwrap().len(), 1000);
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_with_large_string() {
    let large_string = "a".repeat(100_000);
    let response = Response::ok(json!({"content": large_string.clone()}));

    match response {
        Response::Ok { data, .. } => {
            let d = data.unwrap();
            assert_eq!(d["content"].as_str().unwrap().len(), 100_000);
        }
        _ => panic!("Expected Ok response"),
    }
}

// ============================================================================
// Unicode and Special Character Tests
// ============================================================================

#[test]
fn test_response_ok_with_unicode() {
    let response = Response::ok(json!({
        "japanese": "日本語",
        "chinese": "中文",
        "korean": "한국어",
        "emoji": "🎉🚀💻",
        "mixed": "Hello 世界 🌍"
    }));

    match response {
        Response::Ok { data, .. } => {
            let d = data.unwrap();
            assert_eq!(d["japanese"], "日本語");
            assert_eq!(d["chinese"], "中文");
            assert_eq!(d["korean"], "한국어");
            assert_eq!(d["emoji"], "🎉🚀💻");
            assert_eq!(d["mixed"], "Hello 世界 🌍");
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_with_special_characters() {
    let response = Response::ok(json!({
        "quotes": "He said \"Hello\"",
        "backslash": "C:\\Users\\test",
        "newlines": "Line1\nLine2\nLine3",
        "tabs": "Col1\tCol2\tCol3"
    }));

    match response {
        Response::Ok { data, .. } => {
            let d = data.unwrap();
            assert!(d["quotes"].as_str().unwrap().contains("\"Hello\""));
            assert!(d["backslash"].as_str().unwrap().contains("\\"));
            assert!(d["newlines"].as_str().unwrap().contains("\n"));
            assert!(d["tabs"].as_str().unwrap().contains("\t"));
        }
        _ => panic!("Expected Ok response"),
    }
}

// ============================================================================
// Error Chaining Tests
// ============================================================================

#[test]
fn test_all_error_types() {
    let errors = vec![
        DriverError::ConnectionError("conn err".to_string()),
        DriverError::ProtocolError("proto err".to_string()),
        DriverError::DatabaseError("db err".to_string()),
        DriverError::AuthError("auth err".to_string()),
        DriverError::TransactionError("tx err".to_string()),
        DriverError::MessageTooLarge,
        DriverError::InvalidCommand("cmd err".to_string()),
    ];

    for err in errors {
        // All errors should implement Display
        let _ = format!("{}", err);
        // All errors should implement Debug
        let _ = format!("{:?}", err);
    }
}

// ============================================================================
// Response Batch Edge Cases
// ============================================================================

#[test]
fn test_response_batch_empty() {
    let batch = Response::Batch { responses: vec![] };

    match batch {
        Response::Batch { responses } => {
            assert_eq!(responses.len(), 0);
        }
        _ => panic!("Expected Batch response"),
    }
}

#[test]
fn test_response_batch_mixed() {
    let responses = vec![
        Response::ok(json!({"success": true})),
        Response::error(DriverError::DatabaseError("Not found".to_string())),
        Response::pong(),
        Response::ok_count(100),
        Response::ok_tx("tx:abc".to_string()),
    ];

    let batch = Response::Batch { responses };

    match batch {
        Response::Batch { responses } => {
            assert_eq!(responses.len(), 5);

            // Verify first is ok with data
            match &responses[0] {
                Response::Ok { data, .. } => assert!(data.is_some()),
                _ => panic!("Expected Ok"),
            }

            // Verify second is error
            match &responses[1] {
                Response::Error { .. } => {}
                _ => panic!("Expected Error"),
            }

            // Verify third is pong
            match &responses[2] {
                Response::Pong { .. } => {}
                _ => panic!("Expected Pong"),
            }
        }
        _ => panic!("Expected Batch response"),
    }
}

// ============================================================================
// Builder Address Formats
// ============================================================================

#[test]
fn test_builder_with_localhost() {
    let _ = SoliDBClientBuilder::new("localhost:6745");
}

#[test]
fn test_builder_with_ipv4() {
    let _ = SoliDBClientBuilder::new("192.168.1.100:6745");
}

#[test]
fn test_builder_with_loopback() {
    let _ = SoliDBClientBuilder::new("127.0.0.1:6745");
}

#[test]
fn test_builder_with_different_port() {
    let _ = SoliDBClientBuilder::new("localhost:9999");
}

// ============================================================================
// Count Response Tests
// ============================================================================

#[test]
fn test_response_ok_count_zero() {
    let response = Response::ok_count(0);

    match response {
        Response::Ok { count, .. } => {
            assert_eq!(count, Some(0));
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_count_large() {
    let response = Response::ok_count(1_000_000);

    match response {
        Response::Ok { count, .. } => {
            assert_eq!(count, Some(1_000_000));
        }
        _ => panic!("Expected Ok response"),
    }
}

// ============================================================================
// Reads over a real connection
// ============================================================================

/// What the client returns for reads, against an in-process server over
/// loopback TCP: the client decodes responses by hand and reads them into a
/// buffer it keeps, so each answer is checked against what the server
/// computes for itself — through the raw-rows path (stored bytes copied
/// into the response), the ordinary executor, a one-document get, an error,
/// and responses larger than the client's buffer. Values are compared
/// through `Debug`, which tells `1` from `1.0`.
#[test]
fn reads_over_a_real_connection_match_the_server() {
    use serde_json::Value;
    use solidb::sdbql::QueryExecutor;
    use solidb::storage::{IndexType, StorageEngine};
    use std::collections::HashMap;

    const DB: &str = "reads";
    std::env::set_var("SOLIDB_ADMIN_PASSWORD", "reads-password");
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = std::sync::Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    engine.create_database("_system".to_string()).unwrap();
    solidb::server::auth::AuthService::init(&engine, None, tmp.path().to_str().unwrap()).unwrap();
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection("orders".to_string(), None).unwrap();
    let orders = db.get_collection("orders").unwrap();
    orders
        .create_index(
            "idx_customer".into(),
            vec!["customer_id".into()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
    let docs: Vec<Value> = (0..300u64)
        .map(|i| {
            json!({
                "_key": format!("o{i:04}"),
                "customer_id": format!("c{}", i % 7),
                "amount": i as f64 / 4.0,
                "qty": i,
                "delta": -(i as i64),
                "big": u64::MAX - i,
                "name": format!("Zoë ✓ {i}"),
                "tags": ["a", "b", i.to_string()],
                "items": [{"sku": format!("S{i}"), "price": 1.5, "n": null, "ok": i % 2 == 0}],
                "address": {"city": "Lyon", "zip": "69007", "geo": [45.75, 4.85]},
            })
        })
        .collect();
    orders.insert_batch(docs).unwrap();

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
                let mut handler = solidb::driver::DriverHandler::new(storage, None);
                handler.handle_connection(stream, peer.to_string()).await;
            });
        }
    });

    let server_side = |q: &str, vars: &HashMap<String, Value>| -> Vec<Value> {
        let ast = solidb::sdbql::parse(q).unwrap();
        QueryExecutor::with_database_and_bind_vars(&engine, DB.to_string(), vars.clone())
            .execute(&ast)
            .unwrap()
    };
    let by_key = |mut rows: Vec<Value>| {
        rows.sort_by(|a, b| a["_key"].as_str().cmp(&b["_key"].as_str()));
        format!("{rows:?}")
    };

    rt.block_on(async {
        // Two connections, so the round robin reaches both buffers.
        let mut c = SoliDBClientBuilder::new(&addr)
            .auth(DB, "admin", "reads-password")
            .pool_size(2)
            .build()
            .await
            .unwrap();

        // Whole documents through the index: the raw-rows path.
        let q = "FOR o IN orders FILTER o.customer_id == @c RETURN o";
        for customer in 0..7 {
            let vars: HashMap<String, Value> =
                [("c".to_string(), json!(format!("c{customer}")))].into();
            let got = c
                .query_with_cache(DB, q, Some(vars.clone()), false)
                .await
                .unwrap();
            assert!(got.len() > 40);
            assert_eq!(by_key(got), by_key(server_side(q, &vars)));
        }

        // Every document, whole, and projections, through the executor.
        for q in [
            "FOR o IN orders RETURN o",
            "FOR o IN orders SORT o.qty DESC LIMIT 25 RETURN {k: o._key, a: o.amount, d: o.delta, b: o.big, t: o.tags}",
            "RETURN [1, -1, 1.5, 0.0, -0.5, 4294967296, -4294967296, \"é\", null, true, {z: 1, a: [2]}]",
            "FOR i IN 1..2000 RETURN {i, s: CONCAT(\"row \", i), f: i / 8}",
        ] {
            let none = HashMap::new();
            let got = c.query_with_cache(DB, q, None, false).await.unwrap();
            assert_eq!(format!("{got:?}"), format!("{:?}", server_side(q, &none)), "{q}");
        }

        // A single document.
        let got = c.get(DB, "orders", "o0042").await.unwrap();
        assert_eq!(
            format!("{got:?}"),
            format!("{:?}", orders.get("o0042").unwrap().to_value())
        );

        // An error, then the connection carries on.
        let err = c
            .query_with_cache(DB, "FOR o IN", None, false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("error"), "{err}");

        // Past the buffer a connection keeps (1 MiB), then small again.
        let q = format!(
            "FOR i IN 1..3000 RETURN {{i, pad: CONCAT(\"{}\", i)}}",
            "0123456789abcdef".repeat(25)
        );
        let q = q.as_str();
        for _ in 0..2 {
            let got = c.query_with_cache(DB, q, None, false).await.unwrap();
            assert_eq!(got.len(), 3000);
            assert_eq!(got[2999]["i"], json!(3000));
        }
        let big = c
            .query_with_cache(
                DB,
                "FOR o IN orders FOR p IN orders FILTER p.qty < 20 RETURN o",
                None,
                false,
            )
            .await
            .unwrap();
        assert_eq!(big.len(), 300 * 20);
        assert!(c.ping().await.is_ok());
        let got = c.get(DB, "orders", "o0001").await.unwrap();
        assert_eq!(got["delta"], json!(-1));

        // Straight into the caller's type.
        #[derive(serde::Deserialize)]
        struct Row {
            qty: u64,
            big: u64,
            name: String,
        }
        let rows: Vec<Row> = c
            .query_as(DB, "FOR o IN orders FILTER o.qty == 7 RETURN o", None, false)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].qty, rows[0].big, rows[0].name.as_str()),
            (7, u64::MAX - 7, "Zoë ✓ 7")
        );
    });
}

/// Foreign keys through the driver and the Rust client: declared, listed,
/// checked and dropped by commands; enforced on the driver's own writes.
#[test]
fn foreign_keys_over_a_real_connection() {
    use solidb::storage::StorageEngine;

    const DB: &str = "keys";
    std::env::set_var("SOLIDB_ADMIN_PASSWORD", "reads-password");
    let tmp = tempfile::TempDir::new().unwrap();
    let engine = std::sync::Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
    engine.create_database("_system".to_string()).unwrap();
    solidb::server::auth::AuthService::init(&engine, None, tmp.path().to_str().unwrap()).unwrap();
    engine.create_database(DB.to_string()).unwrap();
    let db = engine.get_database(DB).unwrap();
    db.create_collection("customers".to_string(), None).unwrap();
    db.create_collection("orders".to_string(), None).unwrap();

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
                let mut handler = solidb::driver::DriverHandler::new(storage, None);
                handler.handle_connection(stream, peer.to_string()).await;
            });
        }
    });

    rt.block_on(async {
        let mut c = SoliDBClientBuilder::new(&addr)
            .auth(DB, "admin", "reads-password")
            .build()
            .await
            .unwrap();
        let violates =
            |e: &solidb_client::DriverError| e.to_string().contains("Foreign key violation");

        c.insert(DB, "customers", Some("ann"), json!({}))
            .await
            .unwrap();
        c.insert(DB, "orders", Some("o0"), json!({"customer": "zed"}))
            .await
            .unwrap();

        // What the key would be refused for, then the refusal.
        let orphans = c
            .check_foreign_key(DB, "orders", "customer", "customers")
            .await
            .unwrap();
        assert_eq!(orphans["count"], json!(1));
        assert_eq!(orphans["sample"][0]["_key"], json!("o0"));
        let refused = c
            .create_foreign_key(DB, "orders", "customer", "customers", Some("cascade"))
            .await
            .unwrap_err();
        assert!(violates(&refused), "{refused}");
        let unknown = c
            .create_foreign_key(DB, "orders", "customer", "customers", Some("explode"))
            .await;
        assert!(unknown.is_err());

        c.delete(DB, "orders", "o0").await.unwrap();
        let key = c
            .create_foreign_key(DB, "orders", "customer", "customers", Some("cascade"))
            .await
            .unwrap();
        assert_eq!(key["name"], json!("fk_customer"));
        assert_eq!(key["on_delete"], json!("cascade"));

        let listed = c.list_foreign_keys(DB, "orders").await.unwrap();
        assert_eq!(listed["foreign_keys"][0]["field"], json!("customer"));
        let parent = c.list_foreign_keys(DB, "customers").await.unwrap();
        assert_eq!(parent["referenced_by"][0]["collection"], json!("orders"));

        // The driver's writes meet it, and its deletes cascade.
        let orphan = c
            .insert(DB, "orders", Some("o1"), json!({"customer": "zed"}))
            .await
            .unwrap_err();
        assert!(violates(&orphan), "{orphan}");
        c.insert(DB, "orders", Some("o2"), json!({"customer": "ann"}))
            .await
            .unwrap();
        let held = c
            .delete_index(DB, "orders", "fk_customer")
            .await
            .unwrap_err();
        assert!(
            held.to_string().contains("serves the foreign key"),
            "{held}"
        );
        c.delete(DB, "customers", "ann").await.unwrap();
        assert!(c.get(DB, "orders", "o2").await.is_err());

        // The key's index, compared with the documents.
        let report = c.check_indexes(DB, "orders").await.unwrap();
        assert_eq!(report["consistent"], json!(true), "{report}");
        assert_eq!(report["indexes"][0]["name"], json!("fk_customer"));

        c.delete_foreign_key(DB, "orders", "fk_customer")
            .await
            .unwrap();
        assert!(c
            .delete_foreign_key(DB, "orders", "fk_customer")
            .await
            .is_err());
        let listed = c.list_foreign_keys(DB, "orders").await.unwrap();
        assert_eq!(listed["foreign_keys"], json!([]));
    });
}
