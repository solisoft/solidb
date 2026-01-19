use super::wal::WalWriter;
use super::{IsolationLevel, Operation, Transaction, TransactionId};
use crate::error::{DbError, DbResult};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Transaction manager handles transaction lifecycle
pub struct TransactionManager {
    /// Active transactions
    active_transactions: Arc<RwLock<HashMap<TransactionId, Arc<RwLock<Transaction>>>>>,
    /// Write-ahead log
    wal: Arc<WalWriter>,
    /// Transaction timeout
    timeout: Duration,
}

impl TransactionManager {
    /// Create a new transaction manager
    pub fn new(wal_path: PathBuf) -> DbResult<Self> {
        let wal = WalWriter::new(&wal_path)?;

        Ok(Self {
            active_transactions: Arc::new(RwLock::new(HashMap::new())),
            wal: Arc::new(wal),
            timeout: Duration::from_secs(300), // 5 minutes default
        })
    }

    /// Set transaction timeout
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Begin a new transaction
    pub fn begin(&self, isolation_level: IsolationLevel) -> DbResult<TransactionId> {
        let tx = Transaction::new(isolation_level);
        let tx_id = tx.id;

        // Write to WAL
        self.wal.write_begin(tx_id)?;

        // Store in active transactions
        {
            let mut active = self.active_transactions.write().unwrap();
            active.insert(tx_id, Arc::new(RwLock::new(tx)));
        }

        tracing::debug!("Transaction {} started", tx_id);
        Ok(tx_id)
    }

    /// Get a transaction (returns a clone for thread safety)
    pub fn get(&self, tx_id: TransactionId) -> DbResult<Arc<RwLock<Transaction>>> {
        let active = self.active_transactions.read().unwrap();
        active
            .get(&tx_id)
            .cloned()
            .ok_or_else(|| DbError::TransactionNotFound(tx_id.to_string()))
    }

    /// Check if a transaction exists and is active
    pub fn is_active(&self, tx_id: TransactionId) -> bool {
        let active = self.active_transactions.read().unwrap();
        active
            .get(&tx_id)
            .map(|tx| tx.read().unwrap().is_active())
            .unwrap_or(false)
    }

    /// Validate transaction before commit (consistency checks)
    pub fn validate(&self, tx_id: TransactionId) -> DbResult<()> {
        let tx_arc = self.get(tx_id)?;

        // First, collect all errors without holding tx lock
        let errors = {
            let tx = tx_arc.read().unwrap();
            let mut validation_errors = Vec::new();

            // Check for conflicting operations within the transaction
            let mut seen_keys: std::collections::HashMap<String, Vec<Operation>> =
                std::collections::HashMap::new();

            for op in &tx.operations {
                let key = format!("{}:{}:{}", op.database(), op.collection(), op.key());
                seen_keys.entry(key.clone()).or_default().push(op.clone());
            }

            // Check for duplicate inserts within transaction
            for (key, ops) in seen_keys.iter() {
                let inserts: Vec<_> = ops
                    .iter()
                    .filter(|op| matches!(op, Operation::Insert { .. }))
                    .collect();

                if inserts.len() > 1 {
                    let error = format!("Duplicate insert for key {} within transaction", key);
                    validation_errors.push(error);
                }

                // Check for operations on deleted documents
                let deletes: Vec<_> = ops
                    .iter()
                    .filter(|op| matches!(op, Operation::Delete { .. }))
                    .collect();
                if !deletes.is_empty() {
                    let updates_after_delete: Vec<_> = ops
                        .iter()
                        .skip_while(|op| !matches!(op, Operation::Delete { .. }))
                        .filter(|op| matches!(op, Operation::Update { .. }))
                        .collect();

                    if !updates_after_delete.is_empty() {
                        let error =
                            format!("Cannot update deleted document {} within transaction", key);
                        validation_errors.push(error);
                    }
                }
            }

            validation_errors
        };

        // Now add errors to transaction
        {
            let mut tx = tx_arc.write().unwrap();
            tx.clear_validation_errors();
            for error in errors {
                tx.add_validation_error(error);
            }

            // If there are validation errors, return them
            if tx.has_validation_errors() {
                let error_msg = tx.get_validation_errors().join("; ");
                return Err(DbError::TransactionConflict(format!(
                    "Transaction validation failed: {}",
                    error_msg
                )));
            }
        }

        Ok(())
    }

    /// Commit a transaction
    pub fn commit(&self, tx_id: TransactionId) -> DbResult<()> {
        // Validate transaction first (consistency checks)
        self.validate(tx_id)?;

        // Get transaction
        let tx_arc = self.get(tx_id)?;

        // Prepare transaction
        {
            let mut tx = tx_arc.write().unwrap();
            if !tx.is_active() {
                return Err(DbError::TransactionConflict(format!(
                    "Transaction {} is not active (state: {:?})",
                    tx_id, tx.state
                )));
            }
            tx.prepare();
        }

        // Write commit to WAL (ensures durability)
        self.wal.write_commit(tx_id)?;

        // Mark as committed
        {
            let mut tx = tx_arc.write().unwrap();
            tx.commit();
        }

        // Remove from active transactions
        {
            let mut active = self.active_transactions.write().unwrap();
            active.remove(&tx_id);
        }

        tracing::debug!("Transaction {} committed", tx_id);
        Ok(())
    }

    /// Rollback/abort a transaction
    pub fn rollback(&self, tx_id: TransactionId) -> DbResult<()> {
        // Get transaction
        let tx_arc = self.get(tx_id)?;

        // Write abort to WAL
        self.wal.write_abort(tx_id)?;

        // Mark as aborted
        {
            let mut tx = tx_arc.write().unwrap();
            tx.abort();
        }

        // Remove from active transactions
        {
            let mut active = self.active_transactions.write().unwrap();
            active.remove(&tx_id);
        }

        tracing::debug!("Transaction {} rolled back", tx_id);
        Ok(())
    }

    /// Get all active transaction IDs
    pub fn active_transaction_ids(&self) -> Vec<TransactionId> {
        let active = self.active_transactions.read().unwrap();
        active.keys().copied().collect()
    }

    /// Get transaction count
    pub fn transaction_count(&self) -> usize {
        let active = self.active_transactions.read().unwrap();
        active.len()
    }

    /// Clean up expired transactions
    pub fn cleanup_expired(&self) -> usize {
        let now = chrono::Utc::now();
        let mut expired = Vec::new();

        {
            let active = self.active_transactions.read().unwrap();
            for (tx_id, tx_arc) in active.iter() {
                let tx = tx_arc.read().unwrap();
                if now
                    .signed_duration_since(tx.created_at)
                    .to_std()
                    .unwrap_or(Duration::ZERO)
                    > self.timeout
                {
                    expired.push(*tx_id);
                }
            }
        }

        let count = expired.len();
        for tx_id in expired {
            tracing::warn!("Aborting expired transaction {}", tx_id);
            let _ = self.rollback(tx_id);
        }

        count
    }

    /// Get WAL writer for writing operations
    pub fn wal(&self) -> &Arc<WalWriter> {
        &self.wal
    }

    /// Checkpoint - create a WAL checkpoint marker
    pub fn checkpoint(&self) -> DbResult<()> {
        self.wal.write_checkpoint()
    }
}

impl std::fmt::Debug for TransactionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TransactionManager")
            .field("active_count", &self.transaction_count())
            .field("timeout", &self.timeout)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_begin_transaction() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");
        let manager = TransactionManager::new(wal_path).unwrap();

        let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
        assert!(manager.is_active(tx_id));
        assert_eq!(manager.transaction_count(), 1);
    }

    #[test]
    fn test_commit_transaction() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");
        let manager = TransactionManager::new(wal_path).unwrap();

        let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
        manager.commit(tx_id).unwrap();

        assert!(!manager.is_active(tx_id));
        assert_eq!(manager.transaction_count(), 0);
    }

    #[test]
    fn test_rollback_transaction() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");
        let manager = TransactionManager::new(wal_path).unwrap();

        let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
        manager.rollback(tx_id).unwrap();

        assert!(!manager.is_active(tx_id));
        assert_eq!(manager.transaction_count(), 0);
    }

    #[test]
    fn test_multiple_transactions() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");
        let manager = TransactionManager::new(wal_path).unwrap();

        let tx1 = manager.begin(IsolationLevel::ReadCommitted).unwrap();
        let tx2 = manager.begin(IsolationLevel::Serializable).unwrap();

        assert_eq!(manager.transaction_count(), 2);
        assert!(manager.is_active(tx1));
        assert!(manager.is_active(tx2));

        manager.commit(tx1).unwrap();
        assert_eq!(manager.transaction_count(), 1);

        manager.rollback(tx2).unwrap();
        assert_eq!(manager.transaction_count(), 0);
    }

    #[test]
    fn test_transaction_not_found() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");
        let manager = TransactionManager::new(wal_path).unwrap();

        let fake_id = TransactionId::new();
        assert!(manager.get(fake_id).is_err());
    }

    #[test]
    fn test_double_commit() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");
        let manager = TransactionManager::new(wal_path).unwrap();

        let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
        manager.commit(tx_id).unwrap();

        // Second commit should fail (transaction not found)
        assert!(manager.commit(tx_id).is_err());
    }
}
