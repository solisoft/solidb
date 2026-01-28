use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DriverError {
    ConnectionError(String),
    ProtocolError(String),
    DatabaseError(String),
    AuthError(String),
    TransactionError(String),
    MessageTooLarge,
    InvalidCommand(String),
    ServerError(String),
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
            DriverError::ServerError(msg) => write!(f, "Server error: {}", msg),
        }
    }
}

impl std::error::Error for DriverError {}
