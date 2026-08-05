use super::SoliDBClient;
use crate::protocol::{Command, DriverError};
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

        serde_json::from_value(data)
            .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e)))
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
