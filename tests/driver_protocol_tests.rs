//! Driver Protocol Tests
//!
//! Tests for the MessagePack-based client protocol including:
//! - Command encoding/decoding
//! - Response handling
//! - Error handling

use serde_json::json;
use solidb::driver::protocol::{
    decode_message, encode_command, encode_response, Command, DriverError, IsolationLevel, Response,
};
use std::collections::HashMap;

// ============================================================================
// Command Serialization Tests
// ============================================================================

#[test]
fn test_command_query() {
    let cmd = Command::Query {
        database: "_system".to_string(),
        sdbql: "FOR doc IN users RETURN doc".to_string(),
        bind_vars: Some(HashMap::new()),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
    assert!(!encoded.unwrap().is_empty());
}

#[test]
fn test_command_query_with_bind_vars() {
    let mut bind_vars = HashMap::new();
    bind_vars.insert("name".to_string(), json!("Alice"));
    bind_vars.insert("age".to_string(), json!(30));

    let cmd = Command::Query {
        database: "mydb".to_string(),
        sdbql: "FOR doc IN users FILTER doc.name == @name RETURN doc".to_string(),
        bind_vars: Some(bind_vars),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_insert() {
    let cmd = Command::Insert {
        database: "_system".to_string(),
        collection: "users".to_string(),
        key: None,
        document: json!({"name": "Bob", "email": "bob@example.com"}),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_insert_with_key() {
    let cmd = Command::Insert {
        database: "_system".to_string(),
        collection: "users".to_string(),
        key: Some("user123".to_string()),
        document: json!({"name": "Bob"}),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_get() {
    let cmd = Command::Get {
        database: "_system".to_string(),
        collection: "users".to_string(),
        key: "user123".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_update() {
    let cmd = Command::Update {
        database: "_system".to_string(),
        collection: "users".to_string(),
        key: "user123".to_string(),
        document: json!({"name": "Updated Name"}),
        merge: true,
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_delete() {
    let cmd = Command::Delete {
        database: "_system".to_string(),
        collection: "users".to_string(),
        key: "user123".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_list() {
    let cmd = Command::List {
        database: "_system".to_string(),
        collection: "users".to_string(),
        limit: Some(100),
        offset: Some(0),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

// ============================================================================
// Response Tests
// ============================================================================

#[test]
fn test_response_ok() {
    let response = Response::ok(json!({"_key": "abc123", "name": "Test"}));

    let encoded = encode_response(&response);
    assert!(encoded.is_ok());

    // Decode
    let bytes = encoded.unwrap();
    let decoded: Response = decode_message(&bytes[4..]).unwrap();

    match decoded {
        Response::Ok { data, .. } => {
            assert!(data.is_some());
            assert_eq!(data.unwrap()["name"], "Test");
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_empty() {
    let response = Response::ok_empty();

    let encoded = encode_response(&response);
    assert!(encoded.is_ok());

    let bytes = encoded.unwrap();
    let decoded: Response = decode_message(&bytes[4..]).unwrap();

    match decoded {
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

    let encoded = encode_response(&response);
    assert!(encoded.is_ok());

    let bytes = encoded.unwrap();
    let decoded: Response = decode_message(&bytes[4..]).unwrap();

    match decoded {
        Response::Ok { count, .. } => {
            assert_eq!(count, Some(42));
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_ok_tx() {
    let response = Response::ok_tx("tx:12345".to_string());

    let encoded = encode_response(&response);
    assert!(encoded.is_ok());

    let bytes = encoded.unwrap();
    let decoded: Response = decode_message(&bytes[4..]).unwrap();

    match decoded {
        Response::Ok { tx_id, .. } => {
            assert_eq!(tx_id, Some("tx:12345".to_string()));
        }
        _ => panic!("Expected Ok response"),
    }
}

#[test]
fn test_response_error() {
    let response = Response::error(DriverError::DatabaseError("Document not found".to_string()));

    let encoded = encode_response(&response);
    assert!(encoded.is_ok());

    let bytes = encoded.unwrap();
    let decoded: Response = decode_message(&bytes[4..]).unwrap();

    match decoded {
        Response::Error { error } => match error {
            DriverError::DatabaseError(msg) => {
                assert!(msg.contains("not found"));
            }
            _ => panic!("Wrong error type"),
        },
        _ => panic!("Expected Error response"),
    }
}

#[test]
fn test_response_pong() {
    let response = Response::pong();

    let encoded = encode_response(&response);
    assert!(encoded.is_ok());

    let bytes = encoded.unwrap();
    let decoded: Response = decode_message(&bytes[4..]).unwrap();

    match decoded {
        Response::Pong { timestamp } => {
            assert!(timestamp > 0);
        }
        _ => panic!("Expected Pong response"),
    }
}

// ============================================================================
// Collection Command Tests
// ============================================================================

#[test]
fn test_command_create_collection() {
    let cmd = Command::CreateCollection {
        database: "_system".to_string(),
        name: "new_collection".to_string(),
        collection_type: None,
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_create_edge_collection() {
    let cmd = Command::CreateCollection {
        database: "_system".to_string(),
        name: "edges".to_string(),
        collection_type: Some("edge".to_string()),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_list_collections() {
    let cmd = Command::ListCollections {
        database: "_system".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_delete_collection() {
    let cmd = Command::DeleteCollection {
        database: "_system".to_string(),
        name: "to_delete".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_collection_stats() {
    let cmd = Command::CollectionStats {
        database: "_system".to_string(),
        name: "users".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

// ============================================================================
// Database Command Tests
// ============================================================================

#[test]
fn test_command_list_databases() {
    let cmd = Command::ListDatabases;

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_create_database() {
    let cmd = Command::CreateDatabase {
        name: "mydb".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_delete_database() {
    let cmd = Command::DeleteDatabase {
        name: "mydb".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

// ============================================================================
// Transaction Command Tests
// ============================================================================

#[test]
fn test_command_begin_transaction() {
    let cmd = Command::BeginTransaction {
        database: "_system".to_string(),
        isolation_level: IsolationLevel::ReadCommitted,
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_begin_transaction_serializable() {
    let cmd = Command::BeginTransaction {
        database: "_system".to_string(),
        isolation_level: IsolationLevel::Serializable,
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_commit_transaction() {
    let cmd = Command::CommitTransaction {
        tx_id: "tx:12345".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_rollback_transaction() {
    let cmd = Command::RollbackTransaction {
        tx_id: "tx:12345".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_transaction_command() {
    let inner_cmd = Command::Insert {
        database: "_system".to_string(),
        collection: "users".to_string(),
        key: None,
        document: json!({"name": "Test"}),
    };

    let cmd = Command::TransactionCommand {
        tx_id: "tx:12345".to_string(),
        command: Box::new(inner_cmd),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

// ============================================================================
// Index Command Tests
// ============================================================================

#[test]
fn test_command_create_index() {
    let cmd = Command::CreateIndex {
        database: "_system".to_string(),
        collection: "users".to_string(),
        name: "email_idx".to_string(),
        fields: vec!["email".to_string()],
        unique: true,
        sparse: false,
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_delete_index() {
    let cmd = Command::DeleteIndex {
        database: "_system".to_string(),
        collection: "users".to_string(),
        name: "email_idx".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_list_indexes() {
    let cmd = Command::ListIndexes {
        database: "_system".to_string(),
        collection: "users".to_string(),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

// ============================================================================
// Bulk Command Tests
// ============================================================================

#[test]
fn test_command_bulk_insert() {
    let cmd = Command::BulkInsert {
        database: "_system".to_string(),
        collection: "users".to_string(),
        documents: vec![
            json!({"name": "Alice"}),
            json!({"name": "Bob"}),
            json!({"name": "Charlie"}),
        ],
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_batch() {
    let cmd = Command::Batch {
        commands: vec![
            Command::Get {
                database: "_system".to_string(),
                collection: "users".to_string(),
                key: "user1".to_string(),
            },
            Command::Get {
                database: "_system".to_string(),
                collection: "users".to_string(),
                key: "user2".to_string(),
            },
        ],
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

// ============================================================================
// Auth/Ping Command Tests
// ============================================================================

#[test]
fn test_command_auth() {
    let cmd = Command::Auth {
        database: "_system".to_string(),
        username: "admin".to_string(),
        password: "secret".to_string(),
        api_key: None,
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_ping() {
    let cmd = Command::Ping;

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

// ============================================================================
// Round-trip Tests
// ============================================================================

#[test]
fn test_roundtrip_query_command() {
    let original = Command::Query {
        database: "testdb".to_string(),
        sdbql: "RETURN 1+1".to_string(),
        bind_vars: Some(HashMap::new()),
    };

    let encoded = encode_command(&original).unwrap();

    // Decode (skip 4-byte length prefix)
    let decoded: Command = decode_message(&encoded[4..]).unwrap();

    match decoded {
        Command::Query {
            database,
            sdbql,
            bind_vars,
        } => {
            assert_eq!(database, "testdb");
            assert_eq!(sdbql, "RETURN 1+1");
            assert!(bind_vars.unwrap().is_empty());
        }
        _ => panic!("Wrong command type"),
    }
}

#[test]
fn test_roundtrip_insert_command() {
    let original = Command::Insert {
        database: "db".to_string(),
        collection: "col".to_string(),
        key: Some("mykey".to_string()),
        document: json!({"key": "value", "number": 42}),
    };

    let encoded = encode_command(&original).unwrap();
    let decoded: Command = decode_message(&encoded[4..]).unwrap();

    match decoded {
        Command::Insert {
            database,
            collection,
            key,
            document,
        } => {
            assert_eq!(database, "db");
            assert_eq!(collection, "col");
            assert_eq!(key, Some("mykey".to_string()));
            assert_eq!(document.get("key"), Some(&json!("value")));
            assert_eq!(document.get("number"), Some(&json!(42)));
        }
        _ => panic!("Wrong command type"),
    }
}

// ============================================================================
// Edge Cases
// ============================================================================

#[test]
fn test_command_with_special_characters() {
    let cmd = Command::Query {
        database: "db-with-dashes".to_string(),
        sdbql: "FOR doc IN `collection with spaces` FILTER doc.name == 'O\\'Brien' RETURN doc"
            .to_string(),
        bind_vars: Some(HashMap::new()),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
}

#[test]
fn test_command_with_unicode() {
    let cmd = Command::Insert {
        database: "_system".to_string(),
        collection: "users".to_string(),
        key: None,
        document: json!({
            "name": "Müller",
            "city": "東京",
            "emoji": "🎉"
        }),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());

    // Verify roundtrip preserves unicode
    let bytes = encoded.unwrap();
    let decoded: Command = decode_message(&bytes[4..]).unwrap();

    match decoded {
        Command::Insert { document, .. } => {
            assert_eq!(document["name"], "Müller");
            assert_eq!(document["city"], "東京");
            assert_eq!(document["emoji"], "🎉");
        }
        _ => panic!("Wrong command type"),
    }
}

#[test]
fn test_command_with_large_document() {
    let large_array: Vec<i32> = (0..1000).collect();

    let cmd = Command::Insert {
        database: "_system".to_string(),
        collection: "large".to_string(),
        key: None,
        document: json!({"data": large_array}),
    };

    let encoded = encode_command(&cmd);
    assert!(encoded.is_ok());
    assert!(encoded.unwrap().len() > 1000); // Should be reasonably large
}

// ============================================================================
// Error Type Tests
// ============================================================================

#[test]
fn test_driver_error_connection() {
    let err = DriverError::ConnectionError("Connection refused".to_string());
    assert!(err.to_string().contains("Connection"));
}

#[test]
fn test_driver_error_protocol() {
    let err = DriverError::ProtocolError("Invalid message format".to_string());
    assert!(err.to_string().contains("Protocol"));
}

#[test]
fn test_driver_error_database() {
    let err = DriverError::DatabaseError("Collection not found".to_string());
    assert!(err.to_string().contains("Database"));
}

#[test]
fn test_driver_error_auth() {
    let err = DriverError::AuthError("Invalid credentials".to_string());
    assert!(err.to_string().contains("Auth"));
}

#[test]
fn test_driver_error_transaction() {
    let err = DriverError::TransactionError("Transaction aborted".to_string());
    assert!(err.to_string().contains("Transaction"));
}

#[test]
fn test_driver_error_message_too_large() {
    let err = DriverError::MessageTooLarge;
    assert!(err.to_string().contains("too large"));
}

#[test]
fn test_driver_error_invalid_command() {
    let err = DriverError::InvalidCommand("Unknown command".to_string());
    assert!(err.to_string().contains("Invalid command"));
}
