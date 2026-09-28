use super::error::DriverError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        count: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tx_id: Option<String>,
    },
    Error {
        error: DriverError,
    },
    Pong {
        timestamp: i64,
    },
    Batch {
        responses: Vec<Response>,
    },
    /// An `Ok` whose rows are shared with the query cache. It encodes exactly
    /// like `Ok { data: [...] }`; it exists so a cache hit is written straight
    /// from the cached rows. Built as `json!(rows.clone())` it was two deep
    /// copies of every row per request — the clone, then a round trip through
    /// `serde_json::Value`'s serializer.
    #[serde(rename = "ok", skip_deserializing)]
    Rows {
        data: SharedRows,
    },
}

/// Query rows shared with the query cache; serializes as an array.
#[derive(Debug, Clone)]
pub struct SharedRows(pub std::sync::Arc<Vec<Value>>);

impl Serialize for SharedRows {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_slice().serialize(serializer)
    }
}

impl Response {
    pub fn ok(data: Value) -> Self {
        Response::Ok {
            data: Some(data),
            count: None,
            tx_id: None,
        }
    }

    /// Rows shared with the query cache, sent without copying them.
    pub fn ok_shared_rows(rows: std::sync::Arc<Vec<Value>>) -> Self {
        Response::Rows {
            data: SharedRows(rows),
        }
    }

    pub fn ok_count(count: usize) -> Self {
        Response::Ok {
            data: None,
            count: Some(count),
            tx_id: None,
        }
    }

    pub fn ok_empty() -> Self {
        Response::Ok {
            data: None,
            count: None,
            tx_id: None,
        }
    }

    pub fn ok_tx(tx_id: String) -> Self {
        Response::Ok {
            data: None,
            count: None,
            tx_id: Some(tx_id),
        }
    }

    pub fn error(err: DriverError) -> Self {
        Response::Error { error: err }
    }

    pub fn pong() -> Self {
        Response::Pong {
            timestamp: chrono::Utc::now().timestamp_millis(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::protocol::codec::encode_response;

    /// A cache hit is sent as `Rows`; a client must not be able to tell it
    /// from the `Ok` a cache miss sends.
    #[test]
    fn shared_rows_encode_like_ok_data() {
        let rows = vec![
            serde_json::json!({"id": 1, "title": "Post title 1", "views": 7}),
            serde_json::json!({"id": 2, "title": "Post title 2", "views": 14}),
        ];
        let owned = encode_response(&Response::ok(Value::Array(rows.clone()))).unwrap();
        let shared = encode_response(&Response::ok_shared_rows(std::sync::Arc::new(rows))).unwrap();
        assert_eq!(owned, shared);
    }
}
