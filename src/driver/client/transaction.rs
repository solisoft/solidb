use super::SoliDBClient;
use crate::driver::protocol::{Command, DriverError, IsolationLevel};

impl SoliDBClient {
    /// Begin a new transaction
    pub async fn begin_transaction(
        &mut self,
        database: &str,
        isolation_level: Option<IsolationLevel>,
    ) -> Result<String, DriverError> {
        let response = self
            .send_command(Command::BeginTransaction {
                database: database.to_string(),
                isolation_level: isolation_level.unwrap_or_default(),
            })
            .await?;

        let tx_id = Self::extract_tx_id(response)?;
        self.current_tx = Some(tx_id.clone());
        Ok(tx_id)
    }

    /// Commit the current transaction
    pub async fn commit(&mut self) -> Result<(), DriverError> {
        let tx_id = self
            .current_tx
            .take()
            .ok_or_else(|| DriverError::TransactionError("No active transaction".to_string()))?;

        let response = self
            .send_command(Command::CommitTransaction { tx_id })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Rollback the current transaction
    pub async fn rollback(&mut self) -> Result<(), DriverError> {
        let tx_id = self
            .current_tx
            .take()
            .ok_or_else(|| DriverError::TransactionError("No active transaction".to_string()))?;

        let response = self
            .send_command(Command::RollbackTransaction { tx_id })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Check if there's an active transaction
    pub fn in_transaction(&self) -> bool {
        self.current_tx.is_some()
    }

    /// Get the current transaction ID
    pub fn transaction_id(&self) -> Option<&str> {
        self.current_tx.as_deref()
    }
}
