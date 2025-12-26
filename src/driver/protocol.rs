//! Wire protocol definitions for the native driver
//!
//! Uses MessagePack for efficient binary serialization.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Magic header sent at the start of a driver connection
pub const DRIVER_MAGIC: &[u8] = b"solidb-drv-v1\0";

/// Maximum message size (16 MB)
pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

/// Driver protocol error types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DriverError {
    /// Connection or I/O error
    ConnectionError(String),
    /// Protocol violation
    ProtocolError(String),
    /// Database operation error
    DatabaseError(String),
    /// Authentication error
    AuthError(String),
    /// Transaction error
    TransactionError(String),
    /// Message too large
    MessageTooLarge,
    /// Invalid command
    InvalidCommand(String),
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DriverError::ConnectionError(msg) => write!(f, "Connection error: {}", msg),
            DriverError::ProtocolError(msg) => write!(f, "Protocol error: {}", msg),
            DriverError::DatabaseError(msg) => write!(f, "Database error: {}", msg),
            DriverError::AuthError(msg) => write!(f, "Auth error: {}", msg),
            DriverError::TransactionError(msg) => write!(f, "Transaction error: {}", msg),
            DriverError::MessageTooLarge => write!(f, "Message too large"),
            DriverError::InvalidCommand(msg) => write!(f, "Invalid command: {}", msg),
        }
    }
}

impl std::error::Error for DriverError {}

/// Commands that can be sent to the server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Authenticate with the server
    Auth {
        database: String,
        username: String,
        password: String,
    },

    /// Ping the server (keep-alive)
    Ping,

    // ==================== Database Operations ====================
    /// List all databases
    ListDatabases,

    /// Create a new database
    CreateDatabase { name: String },

    /// Delete a database
    DeleteDatabase { name: String },

    // ==================== Collection Operations ====================
    /// List collections in a database
    ListCollections { database: String },

    /// Create a new collection
    CreateCollection {
        database: String,
        name: String,
        #[serde(rename = "type")]
        collection_type: Option<String>,
    },

    /// Delete a collection
    DeleteCollection { database: String, name: String },

    /// Get collection statistics
    CollectionStats { database: String, name: String },

    // ==================== Document Operations ====================
    /// Get a document by key
    Get {
        database: String,
        collection: String,
        key: String,
    },

    /// Insert a new document
    Insert {
        database: String,
        collection: String,
        #[serde(default)]
        key: Option<String>,
        document: Value,
    },

    /// Update an existing document
    Update {
        database: String,
        collection: String,
        key: String,
        document: Value,
        #[serde(default)]
        merge: bool,
    },

    /// Delete a document
    Delete {
        database: String,
        collection: String,
        key: String,
    },

    /// List documents (with pagination)
    List {
        database: String,
        collection: String,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        offset: Option<usize>,
    },

    // ==================== Query Operations ====================
    /// Execute an SDBQL query
    Query {
        database: String,
        sdbql: String,
        #[serde(default)]
        bind_vars: HashMap<String, Value>,
    },

    /// Explain an SDBQL query (without executing)
    Explain {
        database: String,
        sdbql: String,
        #[serde(default)]
        bind_vars: HashMap<String, Value>,
    },

    // ==================== Index Operations ====================
    /// Create an index
    CreateIndex {
        database: String,
        collection: String,
        name: String,
        fields: Vec<String>,
        #[serde(default)]
        unique: bool,
        #[serde(default)]
        sparse: bool,
    },

    /// Delete an index
    DeleteIndex {
        database: String,
        collection: String,
        name: String,
    },

    /// List indexes on a collection
    ListIndexes { database: String, collection: String },

    // ==================== Transaction Operations ====================
    /// Begin a new transaction
    BeginTransaction {
        database: String,
        #[serde(default)]
        isolation_level: IsolationLevel,
    },

    /// Commit a transaction
    CommitTransaction { tx_id: String },

    /// Rollback a transaction
    RollbackTransaction { tx_id: String },

    /// Execute a command within a transaction
    TransactionCommand {
        tx_id: String,
        command: Box<Command>,
    },

    // ==================== Bulk Operations ====================
    /// Execute multiple commands in a batch
    Batch { commands: Vec<Command> },

    /// Bulk insert documents
    BulkInsert {
        database: String,
        collection: String,
        documents: Vec<Value>,
    },
}

/// Isolation level for transactions
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationLevel {
    #[default]
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl From<IsolationLevel> for crate::transaction::IsolationLevel {
    fn from(level: IsolationLevel) -> Self {
        match level {
            IsolationLevel::ReadCommitted => crate::transaction::IsolationLevel::ReadCommitted,
            IsolationLevel::RepeatableRead => crate::transaction::IsolationLevel::RepeatableRead,
            IsolationLevel::Serializable => crate::transaction::IsolationLevel::Serializable,
        }
    }
}

/// Response from the server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    /// Success with optional data
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        count: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tx_id: Option<String>,
    },

    /// Error response
    Error { error: DriverError },

    /// Pong response (for Ping)
    Pong { timestamp: i64 },

    /// Batch response (for Batch command)
    Batch { responses: Vec<Response> },
}

impl Response {
    /// Create a success response with data
    pub fn ok(data: Value) -> Self {
        Response::Ok {
            data: Some(data),
            count: None,
            tx_id: None,
        }
    }

    /// Create a success response with count
    pub fn ok_count(count: usize) -> Self {
        Response::Ok {
            data: None,
            count: Some(count),
            tx_id: None,
        }
    }

    /// Create a success response with no data
    pub fn ok_empty() -> Self {
        Response::Ok {
            data: None,
            count: None,
            tx_id: None,
        }
    }

    /// Create a success response with transaction ID
    pub fn ok_tx(tx_id: String) -> Self {
        Response::Ok {
            data: None,
            count: None,
            tx_id: Some(tx_id),
        }
    }

    /// Create an error response
    pub fn error(err: DriverError) -> Self {
        Response::Error { error: err }
    }

    /// Create a pong response
    pub fn pong() -> Self {
        Response::Pong {
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }
}

/// Helper to encode a command with length prefix (uses compact/fast serialization)
/// Commands are sent from client to server
pub fn encode_command(cmd: &Command) -> Result<Vec<u8>, DriverError> {
    // Use named serialization for commands (required for tagged enums)
    let payload = rmp_serde::to_vec_named(cmd)
        .map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(DriverError::MessageTooLarge);
    }

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Helper to encode a response with length prefix (uses named serialization for compatibility)
/// Responses are sent from server to client
pub fn encode_response(resp: &Response) -> Result<Vec<u8>, DriverError> {
    // Use named serialization for responses (required for tagged enums + external clients)
    let payload = rmp_serde::to_vec_named(resp)
        .map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(DriverError::MessageTooLarge);
    }

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Helper to encode a generic message with length prefix
pub fn encode_message<T: Serialize>(msg: &T) -> Result<Vec<u8>, DriverError> {
    // Use named serialization to ensure maps are serialized with string keys
    let payload = rmp_serde::to_vec_named(msg)
        .map_err(|e| DriverError::ProtocolError(format!("Serialization failed: {}", e)))?;

    if payload.len() > MAX_MESSAGE_SIZE {
        return Err(DriverError::MessageTooLarge);
    }

    let mut buf = Vec::with_capacity(4 + payload.len());
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Helper to decode a message from bytes
pub fn decode_message<T: for<'de> Deserialize<'de>>(data: &[u8]) -> Result<T, DriverError> {
    rmp_serde::from_slice(data)
        .map_err(|e| DriverError::ProtocolError(format!("Deserialization failed: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_serialization() {
        let cmd = Command::Get {
            database: "test".to_string(),
            collection: "users".to_string(),
            key: "user1".to_string(),
        };

        let encoded = encode_message(&cmd).unwrap();
        assert!(encoded.len() > 4);

        // Decode (skip length prefix)
        let decoded: Command = decode_message(&encoded[4..]).unwrap();
        match decoded {
            Command::Get { database, collection, key } => {
                assert_eq!(database, "test");
                assert_eq!(collection, "users");
                assert_eq!(key, "user1");
            }
            _ => panic!("Wrong command type"),
        }
    }

    #[test]
    fn test_response_serialization() {
        let resp = Response::ok(serde_json::json!({"name": "Alice"}));
        let encoded = encode_message(&resp).unwrap();
        let decoded: Response = decode_message(&encoded[4..]).unwrap();

        match decoded {
            Response::Ok { data, .. } => {
                assert_eq!(data.unwrap()["name"], "Alice");
            }
            _ => panic!("Wrong response type"),
        }
    }
}
