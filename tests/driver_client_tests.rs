//! Driver Client Tests
//!
//! Tests for the SoliDB native driver client, including:
//! - SoliDBClientBuilder configuration
//! - Response extraction helpers
//! - Transaction state management
//! - Error handling

use serde_json::json;
use solidb::driver::client::SoliDBClientBuilder;
use solidb::driver::protocol::{DriverError, Response};

// ============================================================================
// SoliDBClientBuilder Tests
// ============================================================================

#[test]
fn test_builder_new() {
    let builder = SoliDBClientBuilder::new("localhost:6745");
    // Builder should be created successfully
    assert!(true);
    let _ = builder; // Ensure builder is used
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
    use solidb::driver::SoliDBClient;

    // Try to connect to an invalid address
    let result = SoliDBClient::connect("invalid-host:99999").await;
    assert!(result.is_err());

    if let Err(err) = result {
        match err {
            DriverError::ConnectionError(msg) => {
                assert!(msg.contains("Failed to connect"));
            }
            _ => panic!("Expected ConnectionError"),
        }
    }
}

#[tokio::test]
async fn test_connect_refused() {
    use solidb::driver::SoliDBClient;

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
