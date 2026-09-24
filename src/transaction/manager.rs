use super::lock_manager::LockManager;
use super::wal::WalWriter;
use super::{IsolationLevel, Operation, Transaction, TransactionId};
use crate::error::{DbError, DbResult};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

#[allow(dead_code)]
pub struct TransactionManager {
    active_transactions: Arc<RwLock<HashMap<TransactionId, Arc<RwLock<Transaction>>>>>,
    wal: Arc<WalWriter>,
    lock_manager: Arc<LockManager>,
    timeout: Duration,
    wal_batch_size: usize,
}

impl TransactionManager {
    pub fn new(wal_path: PathBuf) -> DbResult<Self> {
        Self::with_wal_batch_size(wal_path, 100)
    }

    pub fn with_wal_batch_size(wal_path: PathBuf, batch_size: usize) -> DbResult<Self> {
        let wal = WalWriter::with_batch_size(&wal_path, batch_size)?;

        Ok(Self {
            active_transactions: Arc::new(RwLock::new(HashMap::new())),
            wal: Arc::new(wal),
            lock_manager: Arc::new(LockManager::new()),
            timeout: Duration::from_secs(300),
            wal_batch_size: batch_size,
        })
    }

    /// Set transaction timeout
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Begin a new transaction
    ///
    /// Audit M7: nothing is written to the transaction WAL any more. It used
    /// to get a `Begin` line here and a fsynced `Commit` line at commit, but
    /// no operation was ever logged, so replay recovered nothing while the
    /// file grew without bound. Atomicity and durability now come from the
    /// commit being a single RocksDB `WriteBatch` (synced for isolation
    /// levels that `requires_wal`), which RocksDB's own WAL covers.
    pub fn begin(&self, isolation_level: IsolationLevel) -> DbResult<TransactionId> {
        let tx = Transaction::new(isolation_level);
        let tx_id = tx.id;

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

    /// First half of a commit: freeze the transaction and validate it.
    ///
    /// Moves the transaction from `Active` to `Preparing` — so no further
    /// operations can be added and a concurrent commit, rollback or the
    /// expiry reaper cannot act on it — then runs [`Self::validate`]. On a
    /// validation failure the transaction is aborted (removed, locks
    /// released) before the error is returned; nothing has been written.
    ///
    /// Returns the operations to apply and whether the write must be synced.
    pub fn prepare_commit(&self, tx_id: TransactionId) -> DbResult<(Vec<Operation>, bool)> {
        let tx_arc = self.get(tx_id)?;

        let requires_sync = {
            let mut tx = tx_arc.write().unwrap();
            if !tx.is_active() {
                return Err(DbError::TransactionConflict(format!(
                    "Transaction {} is not active (state: {:?})",
                    tx_id, tx.state
                )));
            }
            tx.prepare();
            tx.isolation_level.requires_wal()
        };

        // Audit D2: validate *before* anything is written.
        if let Err(e) = self.validate(tx_id) {
            self.abort(tx_id);
            return Err(e);
        }

        let operations = tx_arc.read().unwrap().operations.clone();
        Ok((operations, requires_sync))
    }

    /// Second half of a commit, once its writes are durable: mark it
    /// committed, forget it, release its locks.
    pub fn finish_commit(&self, tx_id: TransactionId) -> DbResult<()> {
        let tx_arc = self.get(tx_id)?;
        tx_arc.write().unwrap().commit();

        {
            let mut active = self.active_transactions.write().unwrap();
            active.remove(&tx_id);
        }
        self.lock_manager.release_locks(tx_id);

        tracing::debug!("Transaction {} committed", tx_id);
        Ok(())
    }

    /// Commit a transaction that has no storage effects to apply (the
    /// storage engine drives [`Self::prepare_commit`] / [`Self::finish_commit`]
    /// itself so it can write in between).
    pub fn commit(&self, tx_id: TransactionId) -> DbResult<()> {
        self.prepare_commit(tx_id)?;
        self.finish_commit(tx_id)
    }

    /// Abort regardless of state: used when a commit fails part-way, after
    /// the transaction has already left `Active`. Nothing was written, so
    /// dropping it and its locks is the whole rollback.
    pub fn abort(&self, tx_id: TransactionId) {
        let removed = {
            let mut active = self.active_transactions.write().unwrap();
            active.remove(&tx_id)
        };
        if let Some(tx_arc) = removed {
            tx_arc.write().unwrap().abort();
        }
        self.lock_manager.release_locks(tx_id);
        tracing::debug!("Transaction {} aborted", tx_id);
    }

    pub fn rollback(&self, tx_id: TransactionId) -> DbResult<()> {
        let tx_arc = self.get(tx_id)?;

        {
            let mut tx = tx_arc.write().unwrap();
            if tx.state == super::TransactionState::Preparing {
                // A commit is writing this transaction right now; yanking its
                // locks mid-write would let another writer in before the
                // commit finishes. The committer cleans up on either outcome.
                return Err(DbError::TransactionConflict(format!(
                    "Transaction {} is being committed",
                    tx_id
                )));
            }
            tx.abort();
        }

        {
            let mut active = self.active_transactions.write().unwrap();
            active.remove(&tx_id);
        }

        // Release locks
        self.lock_manager.release_locks(tx_id);

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
                // A transaction mid-commit is not abandoned.
                if tx.is_active()
                    && now
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

    pub fn wal(&self) -> &Arc<WalWriter> {
        &self.wal
    }

    pub fn lock_manager(&self) -> &Arc<LockManager> {
        &self.lock_manager
    }

    /// Nothing in the transaction WAL is needed once its contents are
    /// applied (see [`Self::begin`]), so a checkpoint empties it rather than
    /// appending a marker to a file that would otherwise only grow.
    pub fn checkpoint(&self) -> DbResult<()> {
        self.wal.truncate()
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

    #[test]
    fn test_failed_validation_aborts_and_releases_locks() {
        let dir = tempdir().unwrap();
        let manager = TransactionManager::new(dir.path().join("test.wal")).unwrap();

        let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
        manager
            .lock_manager()
            .acquire_exclusive(tx_id, "db", "c", "k")
            .unwrap();
        {
            let tx_arc = manager.get(tx_id).unwrap();
            let mut tx = tx_arc.write().unwrap();
            for _ in 0..2 {
                tx.add_operation(Operation::Insert {
                    database: "db".into(),
                    collection: "c".into(),
                    key: "k".into(),
                    data: serde_json::json!({}),
                });
            }
        }

        assert!(matches!(
            manager.prepare_commit(tx_id),
            Err(DbError::TransactionConflict(_))
        ));
        // Gone, and its lock is free for the next transaction.
        assert_eq!(manager.transaction_count(), 0);
        let other = manager.begin(IsolationLevel::ReadCommitted).unwrap();
        assert!(manager
            .lock_manager()
            .acquire_exclusive(other, "db", "c", "k")
            .is_ok());
    }

    #[test]
    fn test_rollback_refused_while_preparing() {
        let dir = tempdir().unwrap();
        let manager = TransactionManager::new(dir.path().join("test.wal")).unwrap();

        let tx_id = manager.begin(IsolationLevel::ReadCommitted).unwrap();
        manager.prepare_commit(tx_id).unwrap();
        assert!(manager.rollback(tx_id).is_err());
        assert_eq!(manager.cleanup_expired(), 0);
        manager.finish_commit(tx_id).unwrap();
        assert_eq!(manager.transaction_count(), 0);
    }

    #[test]
    fn test_begin_and_commit_do_not_grow_wal() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");
        let manager = TransactionManager::new(wal_path.clone()).unwrap();

        for _ in 0..10 {
            let tx = manager.begin(IsolationLevel::Serializable).unwrap();
            manager.commit(tx).unwrap();
        }
        assert_eq!(std::fs::metadata(&wal_path).unwrap().len(), 0);
    }
}
