use super::error::DriverError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

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
