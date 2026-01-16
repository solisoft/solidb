use serde::{Deserialize, Serialize};

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
