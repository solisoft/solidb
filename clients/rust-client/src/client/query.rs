use super::SoliDBClient;
use crate::protocol::{Command, DriverError};
use serde_json::Value;
use std::collections::HashMap;

impl SoliDBClient {
    pub async fn query(
        &mut self,
        database: &str,
        sdbql: &str,
        bind_vars: Option<HashMap<String, Value>>,
    ) -> Result<Vec<Value>, DriverError> {
        let response = self
            .send_command(Command::Query {
                database: database.to_string(),
                sdbql: sdbql.to_string(),
                bind_vars,
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
