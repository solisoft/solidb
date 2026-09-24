use super::SoliDBClient;
use crate::protocol::response::{RowsResponse, WireStatus};
use crate::protocol::{decode_message, Command, DriverError};
use serde_json::Value;
use std::collections::HashMap;

impl SoliDBClient {
    /// Run an SDBQL query, using SoliDB's read-result cache when eligible.
    pub async fn query(
        &mut self,
        database: &str,
        sdbql: &str,
        bind_vars: Option<HashMap<String, Value>>,
    ) -> Result<Vec<Value>, DriverError> {
        self.query_with_cache(database, sdbql, bind_vars, true)
            .await
    }

    /// Like [`query`](Self::query), but controls result memoization.
    ///
    /// `cache: false` mirrors HTTP `/cursor` with `"cache": false` — the query
    /// always executes for real. Used by Soli's `SOLI_DB_NO_QUERY_CACHE=1`
    /// diagnostic so the driver path and the cursor path measure the same thing.
    pub async fn query_with_cache(
        &mut self,
        database: &str,
        sdbql: &str,
        bind_vars: Option<HashMap<String, Value>>,
        cache: bool,
    ) -> Result<Vec<Value>, DriverError> {
        let response = self
            .send_command(Command::Query {
                database: database.to_string(),
                sdbql: sdbql.to_string(),
                bind_vars,
                cache,
            })
            .await?;

        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;

        // Rows arrive as an array already: take its vector rather than running
        // every row back through `serde_json::from_value`, which deserializes
        // (and so copies) the whole result set a second time.
        match data {
            Value::Array(rows) => Ok(rows),
            other => serde_json::from_value(other)
                .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e))),
        }
    }

    /// Like [`query_with_cache`](Self::query_with_cache), but each row is
    /// deserialized straight from the wire into `T` — no intermediate
    /// `serde_json::Value`. For callers with their own value representation.
    pub async fn query_as<T: serde::de::DeserializeOwned>(
        &mut self,
        database: &str,
        sdbql: &str,
        bind_vars: Option<HashMap<String, Value>>,
        cache: bool,
    ) -> Result<Vec<T>, DriverError> {
        let payload = self
            .send_command_raw(Command::Query {
                database: database.to_string(),
                sdbql: sdbql.to_string(),
                bind_vars,
                cache,
            })
            .await?;
        rows_from_payload(&payload)
    }

    pub async fn explain(
        &mut self,
        database: &str,
        sdbql: &str,
        bind_vars: Option<HashMap<String, Value>>,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::Explain {
                database: database.to_string(),
                sdbql: sdbql.to_string(),
                bind_vars,
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }
}

/// Decode a query response payload into rows of `T`.
fn rows_from_payload<T: serde::de::DeserializeOwned>(
    payload: &[u8],
) -> Result<Vec<T>, DriverError> {
    let response: RowsResponse<T> = decode_message(payload)?;
    match response.status {
        WireStatus::Ok => response
            .data
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string())),
        WireStatus::Error => Err(response.error.unwrap_or_else(|| {
            DriverError::ProtocolError("Error response without an error".to_string())
        })),
        WireStatus::Pong | WireStatus::Batch => Err(DriverError::ProtocolError(
            "Unexpected response to a query".to_string(),
        )),
    }
}

#[cfg(test)]
mod rows_tests {
    use super::*;
    use crate::protocol::Response;

    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Row {
        id: i64,
        title: String,
    }

    #[test]
    fn rows_decode_straight_into_the_callers_type() {
        let payload = rmp_serde::to_vec_named(&Response::ok(serde_json::json!([
            {"id": 1, "title": "a"},
            {"id": 2, "title": "b"}
        ])))
        .unwrap();
        let rows: Vec<Row> = rows_from_payload(&payload).unwrap();
        assert_eq!(
            rows,
            vec![
                Row {
                    id: 1,
                    title: "a".into()
                },
                Row {
                    id: 2,
                    title: "b".into()
                }
            ]
        );
    }

    #[test]
    fn an_error_response_is_an_error() {
        let payload =
            rmp_serde::to_vec_named(&Response::error(DriverError::ProtocolError("boom".into())))
                .unwrap();
        let err = rows_from_payload::<Row>(&payload).unwrap_err();
        assert!(format!("{err:?}").contains("boom"));
    }
}
