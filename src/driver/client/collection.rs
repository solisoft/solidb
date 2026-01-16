use super::SoliDBClient;
use crate::driver::protocol::{Command, DriverError};
use serde_json::Value;

impl SoliDBClient {
    /// List collections in a database
    pub async fn list_collections(&mut self, database: &str) -> Result<Vec<String>, DriverError> {
        let response = self
            .send_command(Command::ListCollections {
                database: database.to_string(),
            })
            .await?;
        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;

        serde_json::from_value(data)
            .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e)))
    }

    /// Create a new collection
    pub async fn create_collection(
        &mut self,
        database: &str,
        name: &str,
        collection_type: Option<&str>,
    ) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::CreateCollection {
                database: database.to_string(),
                name: name.to_string(),
                collection_type: collection_type.map(|s| s.to_string()),
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Delete a collection
    pub async fn delete_collection(
        &mut self,
        database: &str,
        name: &str,
    ) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::DeleteCollection {
                database: database.to_string(),
                name: name.to_string(),
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Get collection statistics
    pub async fn collection_stats(
        &mut self,
        database: &str,
        collection: &str,
    ) -> Result<Value, DriverError> {
        let response = self
            .send_command(Command::CollectionStats {
                database: database.to_string(),
                name: collection.to_string(),
            })
            .await?;
        Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))
    }
}
