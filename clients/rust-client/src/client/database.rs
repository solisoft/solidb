use super::SoliDBClient;
use crate::protocol::{Command, DriverError};

impl SoliDBClient {
    pub async fn list_databases(&mut self) -> Result<Vec<String>, DriverError> {
        let response = self.send_command(Command::ListDatabases).await?;
        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;

        serde_json::from_value(data)
            .map_err(|e| DriverError::ProtocolError(format!("Invalid response: {}", e)))
    }

    pub async fn create_database(&mut self, name: &str) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::CreateDatabase {
                name: name.to_string(),
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    pub async fn delete_database(&mut self, name: &str) -> Result<(), DriverError> {
        let response = self
            .send_command(Command::DeleteDatabase {
                name: name.to_string(),
            })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }
}
