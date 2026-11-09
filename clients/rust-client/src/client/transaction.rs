use super::{OpenTransaction, SoliDBClient};
use crate::protocol::{Command, DriverError, IsolationLevel, TransactionStep};
use serde_json::Value;

/// How a transaction sent in one request ended.
#[derive(Debug, Clone, PartialEq)]
pub enum TransactionOutcome {
    /// Every step ran and the transaction committed: each step's result, as
    /// the same call inside a transaction would have returned it.
    Committed(Vec<Value>),
    /// Step `step` had [`TransactionStep::require_rows`] and its query
    /// returned no row: the transaction was rolled back, nothing landed.
    Stopped { step: usize },
}

impl SoliDBClient {
    /// Open a transaction on one of the pool's connections.
    ///
    /// Until [`commit`](Self::commit) or [`rollback`](Self::rollback),
    /// [`insert`](Self::insert), [`update`](Self::update) and
    /// [`delete`](Self::delete) are staged in it and applied at commit. A
    /// query reads committed data (it does not see the transaction's own
    /// writes), and the writes of a query that writes are staged too (servers
    /// after 2.9.1; 2.9.1 and earlier refuse it); an `UPDATE` it computes from
    /// a document that changed since it read it fails the transaction with a
    /// conflict. `bulk_insert` and `batch` are refused. A [`get`](Self::get) is not part of the
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

    /// A whole transaction in one round trip (servers after 2.9.2): the
    /// server begins it, runs `steps` in order, and commits. A step that
    /// fails rolls it back and its error names the step (a conflict reads
    /// "… conflict …": run the transaction again); a step with
    /// [`TransactionStep::require_rows`] whose query returns no row rolls it
    /// back and returns [`TransactionOutcome::Stopped`]. Each step needs the
    /// permission it would need on its own. Saves the round trips of
    /// `begin_transaction`, one per step, and `commit`: a money transfer
    /// written this way is one request instead of five.
    ///
    /// ```no_run
    /// # async fn f(c: &mut solidb_client::SoliDBClient) -> Result<(), solidb_client::DriverError> {
    /// use solidb_client::{TransactionOutcome, TransactionStep};
    /// use std::collections::HashMap;
    /// use serde_json::json;
    /// let vars = |k: &str| Some(HashMap::from([("k".into(), json!(k)), ("x".into(), json!(30))]));
    /// let outcome = c.run_transaction("bank", vec![
    ///     TransactionStep::query("bank",
    ///         "FOR a IN accounts FILTER a._key == @k FILTER a.balance >= @x \
    ///          UPDATE a WITH {balance: a.balance - @x} IN accounts RETURN 1", vars("alice"))
    ///         .require_rows(),
    ///     TransactionStep::query("bank",
    ///         "FOR a IN accounts FILTER a._key == @k \
    ///          UPDATE a WITH {balance: a.balance + @x} IN accounts RETURN 1", vars("bob"))
    ///         .require_rows(),
    ///     TransactionStep::insert("bank", "transfers", None, json!({"from": "alice", "to": "bob", "x": 30})),
    /// ]).await?;
    /// if let TransactionOutcome::Stopped { step: 0 } = outcome { /* not enough money */ }
    /// # Ok(()) }
    /// ```
    pub async fn run_transaction(
        &mut self,
        database: &str,
        steps: Vec<TransactionStep>,
    ) -> Result<TransactionOutcome, DriverError> {
        let response = self
            .send_command(Command::RunTransaction {
                database: database.to_string(),
                isolation_level: IsolationLevel::default(),
                steps,
            })
            .await?;
        let data = Self::extract_data(response)?
            .ok_or_else(|| DriverError::ProtocolError("Expected data".to_string()))?;
        match data.get("committed").and_then(Value::as_bool) {
            Some(true) => Ok(TransactionOutcome::Committed(
                data.get("results")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            )),
            Some(false) => Ok(TransactionOutcome::Stopped {
                step: data
                    .get("stopped_at")
                    .and_then(Value::as_u64)
                    .unwrap_or_default() as usize,
            }),
            None => Err(DriverError::ProtocolError(format!(
                "unexpected answer to run_transaction: {data}"
            ))),
        }
    }

    pub fn in_transaction(&self) -> bool {
        self.current_tx.is_some()
    }

    pub fn transaction_id(&self) -> Option<&str> {
        self.current_tx.as_ref().map(|tx| tx.id.as_str())
    }
}
