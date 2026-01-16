use super::SoliDBClient;
use crate::driver::protocol::{Command, DriverError, Response};
use serde_json::Value;

impl SoliDBClient {
    /// Get a document by key
    pub async fn get(
        &mut self,
        database: &str,
        collection: &str,
        key: &str,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::Get {
                database: database.to_string(),
                collection: collection.to_string(),
                key: key.to_string(),
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Insert a new document
    pub async fn insert(
        &mut self,
        database: &str,
        collection: &str,
        key: Option<&str>,
        document: Value,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::Insert {
                database: database.to_string(),
                collection: collection.to_string(),
                key: key.map(|s| s.to_string()),
                document,
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Update an existing document
    pub async fn update(
        &mut self,
        database: &str,
        collection: &str,
        key: &str,
        document: Value,
        merge: bool,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::Update {
                database: database.to_string(),
                collection: collection.to_string(),
                key: key.to_string(),
                document,
                merge,
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }

    /// Delete a document
    pub async fn delete(
        &mut self,
        database: &str,
        collection: &str,
        key: &str,
    ) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::Delete {
                database: database.to_string(),
                collection: collection.to_string(),
                key: key.to_string(),
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// List documents in a collection with pagination
    pub async fn list(
        &mut self,
        database: &str,
        collection: &str,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<(Vec<Value>, usize), DriverError> {
        let response = self
            .send_command(Command::List {
                database: database.to_string(),
                collection: collection.to_string(),
                limit,
                offset,
            })
            .await?;

        match response {
            Response::Ok { data, count, .. } => {
                let docs: Vec<Value> = data
                    .and_then(|d| serde_json::from_value(d).ok())
                    .unwrap_or_default();
                let len = docs.len();
                Ok((docs, count.unwrap_or(len)))
            }
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError(
                "Unexpected response".to_string(),
            )),
        }
    }
}
