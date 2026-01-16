use super::SoliDBClient;
use crate::driver::protocol::{Command, DriverError, Response};
use serde_json::Value;

impl SoliDBClient {
    /// Execute multiple commands in a batch
    pub async fn batch(&mut self, commands: Vec<Command>) -> Result<Vec<Response>, DriverError> {
        let response = self.send_command(Command::Batch { commands }).await?;
        match response {
            Response::Batch { responses } => Ok(responses),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError(
                "Expected batch response".to_string(),
            )),
        }
    }

    /// Bulk insert documents
    pub async fn bulk_insert(
        &mut self,
        database: &str,
        collection: &str,
        documents: Vec<Value>,
    ) -> Result<usize, DriverError> {
        let response = self
            .send_command(Command::BulkInsert {
                database: database.to_string(),
                collection: collection.to_string(),
                documents,
            })
            .await?;

        match response {
            Response::Ok { count, .. } => Ok(count.unwrap_or(0)),
            Response::Error { error } => Err(error),
            _ => Err(DriverError::ProtocolError(
                "Unexpected response".to_string(),
            )),
        }
    }
}
