use super::error::DriverError;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// `Deserialize` is written by hand below; `Serialize` keeps the derive, so the
// encoding (an internally tagged map: `{"status": "ok", "data": …}`) is unchanged.
#[derive(Debug, Clone, Serialize)]
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
}

impl Response {
    pub fn ok(data: Value) -> Self {
        Response::Ok {
            data: Some(data),
            count: None,
            tx_id: None,
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

/// The response as it is on the wire: one flat map whose `status` says which
/// variant it is.
///
/// A derived `Deserialize` for an internally tagged enum cannot know the
/// variant before it has seen `status`, and `status` may come after the
/// payload, so serde first buffers the *whole* message into its private
/// `Content` tree and deserializes the variant from that copy. For a query
/// result that is every row copied once more before it reaches the caller —
/// the largest single cost in decoding a read. A flat struct needs no
/// lookahead: rmp_serde fills each field straight from the bytes, in whatever
/// order they arrive, and the variant is picked afterwards.
#[derive(Deserialize)]
struct WireResponse {
    status: WireStatus,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    count: Option<usize>,
    #[serde(default)]
    tx_id: Option<String>,
    #[serde(default)]
    error: Option<DriverError>,
    #[serde(default)]
    timestamp: Option<i64>,
    #[serde(default)]
    responses: Option<Vec<Response>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WireStatus {
    Ok,
    Error,
    Pong,
    Batch,
}

/// A query response whose rows deserialize straight into the caller's `T`.
///
/// Used by [`SoliDBClient::query_as`](crate::SoliDBClient::query_as): the rows
/// never become `serde_json::Value`, so a caller with its own value model (an
/// interpreter, say) pays one decode instead of two.
#[derive(Deserialize)]
pub(crate) struct RowsResponse<T> {
    pub(crate) status: WireStatus,
    #[serde(default = "none")]
    pub(crate) data: Option<Vec<T>>,
    #[serde(default)]
    pub(crate) error: Option<DriverError>,
}

fn none<T>() -> Option<T> {
    None
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let wire = WireResponse::deserialize(deserializer)?;
        Ok(match wire.status {
            WireStatus::Ok => Response::Ok {
                data: wire.data,
                count: wire.count,
                tx_id: wire.tx_id,
            },
            WireStatus::Error => Response::Error {
                error: wire.error.ok_or_else(|| D::Error::missing_field("error"))?,
            },
            WireStatus::Pong => Response::Pong {
                timestamp: wire
                    .timestamp
                    .ok_or_else(|| D::Error::missing_field("timestamp"))?,
            },
            WireStatus::Batch => Response::Batch {
                responses: wire
                    .responses
                    .ok_or_else(|| D::Error::missing_field("responses"))?,
            },
        })
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;

    /// Encode the way the server does (`rmp_serde::to_vec_named`) and decode
    /// with the hand-written `Deserialize`.
    fn round_trip(resp: &Response) -> Response {
        let bytes = rmp_serde::to_vec_named(resp).expect("encode");
        rmp_serde::from_slice(&bytes).expect("decode")
    }

    #[test]
    fn every_variant_survives_the_wire() {
        let rows = serde_json::json!([{"id": 1, "title": "a"}, {"id": 2, "title": "b"}]);
        match round_trip(&Response::ok(rows.clone())) {
            Response::Ok { data, count, tx_id } => {
                assert_eq!(data, Some(rows));
                assert_eq!((count, tx_id), (None, None));
            }
            other => panic!("expected ok, got {other:?}"),
        }
        match round_trip(&Response::ok_count(7)) {
            Response::Ok { count, data, .. } => assert_eq!((count, data), (Some(7), None)),
            other => panic!("expected ok, got {other:?}"),
        }
        match round_trip(&Response::ok_tx("tx-1".into())) {
            Response::Ok { tx_id, .. } => assert_eq!(tx_id.as_deref(), Some("tx-1")),
            other => panic!("expected ok, got {other:?}"),
        }
        match round_trip(&Response::Pong { timestamp: 42 }) {
            Response::Pong { timestamp } => assert_eq!(timestamp, 42),
            other => panic!("expected pong, got {other:?}"),
        }
        let err = DriverError::ProtocolError("nope".into());
        match round_trip(&Response::error(err)) {
            Response::Error { error } => assert!(format!("{error:?}").contains("nope")),
            other => panic!("expected error, got {other:?}"),
        }
        let batch = Response::Batch {
            responses: vec![Response::ok_count(1), Response::Pong { timestamp: 3 }],
        };
        match round_trip(&batch) {
            Response::Batch { responses } => {
                assert_eq!(responses.len(), 2);
                assert!(matches!(responses[1], Response::Pong { timestamp: 3 }));
            }
            other => panic!("expected batch, got {other:?}"),
        }
    }

    /// `status` after the payload: the case that forced the buffered decode.
    #[test]
    fn status_may_come_last() {
        #[derive(Serialize)]
        struct Reordered {
            data: Value,
            status: &'static str,
        }
        let bytes = rmp_serde::to_vec_named(&Reordered {
            data: serde_json::json!([1, 2]),
            status: "ok",
        })
        .unwrap();
        let resp: Response = rmp_serde::from_slice(&bytes).unwrap();
        assert!(matches!(resp, Response::Ok { data: Some(_), .. }));
    }

    #[test]
    fn an_unknown_status_or_a_missing_error_is_refused() {
        #[derive(Serialize)]
        struct Only {
            status: &'static str,
        }
        let unknown = rmp_serde::to_vec_named(&Only { status: "weird" }).unwrap();
        assert!(rmp_serde::from_slice::<Response>(&unknown).is_err());
        let bare_error = rmp_serde::to_vec_named(&Only { status: "error" }).unwrap();
        assert!(rmp_serde::from_slice::<Response>(&bare_error).is_err());
    }
}
