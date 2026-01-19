use super::SoliDBClient;
use crate::protocol::{Command, DriverError};
use serde_json::Value;

impl SoliDBClient {
    pub async fn create_index(
        &mut self,
        database: &str,
        collection: &str,
        name: &str,
        fields: Vec<String>,
        unique: bool,
        sparse: bool,
    ) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::CreateIndex {
                database: database.to_string(),
                collection: collection.to_string(),
                name: name.to_string(),
                fields,
                unique,
                sparse,
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    pub async fn delete_index(
        &mut self,
        database: &str,
        collection: &str,
        name: &str,
    ) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::DeleteIndex {
                database: database.to_string(),
                collection: collection.to_string(),
                name: name.to_string(),
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    pub async fn list_indexes(
        &mut self,
        database: &str,
        collection: &str,
    ) -> Result<Vec<Value>, DriverError> {
        let response = self
            .send_command(Command::ListIndexes {
                database: database.to_string(),
                collection: collection.to_string(),
            })
            .await?;
        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;

        serde_json::from_value(data)
            .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e)))
    }
}
