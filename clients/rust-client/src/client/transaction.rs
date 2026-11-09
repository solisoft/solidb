use super::{OpenTransaction, SoliDBClient};
use crate::protocol::{Command, DriverError, IsolationLevel};

impl SoliDBClient {
    /// Open a transaction on one of the pool's connections.
    ///
    /// Until [`commit`](Self::commit) or [`rollback`](Self::rollback),
    /// [`insert`](Self::insert), [`update`](Self::update) and
    /// [`delete`](Self::delete) are staged in it and applied at commit, and
    /// a read-only query runs against committed data (it does not see the
    /// transaction's own writes); a query that writes, `bulk_insert` and
    /// `batch` are refused. A [`get`](Self::get) is not part of the
    /// transaction either: it reads committed data. One transaction at a
    /// time per client.
    pub async fn begin_transaction(
        &mut self,
        database: &str,
        isolation_level: Option<IsolationLevel>,
    ) -> Result<String, DriverError> {
        if self.current_tx.is_some() {
            return Err(DriverError::TransactionError(
                "a transaction is already open on this client: commit or roll it back first"
                    .to_string(),
            ));
        }
        let connection = self.next_connection_index();
        let response = self
            .send_command_on(
                connection,
                Command::BeginTransaction {
                    database: database.to_string(),
                    isolation_level: isolation_level.unwrap_or_default(),
                },
            )
            .await?;

        let tx_id = Self::extract_tx_id(response)?;
        self.current_tx = Some(OpenTransaction {
            id: tx_id.clone(),
            connection,
        });
        Ok(tx_id)
    }

    /// Commit the open transaction, on the connection that began it. The
    /// client has no open transaction afterwards, whatever the outcome.
    pub async fn commit(&mut self) -> Result<(), DriverError> {
        let tx = self.take_transaction()?;
        let response = self
            .send_command_on(tx.connection, Command::CommitTransaction { tx_id: tx.id })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    /// Roll back the open transaction: nothing it staged is applied.
    pub async fn rollback(&mut self) -> Result<(), DriverError> {
        let tx = self.take_transaction()?;
        let response = self
            .send_command_on(tx.connection, Command::RollbackTransaction { tx_id: tx.id })
            .await?;
        Self::extract_data(response)?;
        Ok(())
    }

    fn take_transaction(&mut self) -> Result<OpenTransaction, DriverError> {
        self.current_tx
            .take()
            .ok_or_else(|| DriverError::TransactionError("No active transaction".to_string()))
    }

    pub fn in_transaction(&self) -> bool {
        self.current_tx.is_some()
    }

    pub fn transaction_id(&self) -> Option<&str> {
        self.current_tx.as_ref().map(|tx| tx.id.as_str())
    }
}
