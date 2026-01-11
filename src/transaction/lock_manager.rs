use super::TransactionId;
use crate::error::{DbError, DbResult};
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

/// Type of lock
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockType {
    /// Shared lock (for reading)
    Shared,
    /// Exclusive lock (for writing)
    Exclusive,
}

/// A unique key identifying a resource to lock
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LockKey {
    pub database: String,
    pub collection: String,
    pub key: String,
}

impl LockKey {
    pub fn new(database: &str, collection: &str, key: &str) -> Self {
        Self {
            database: database.to_string(),
            collection: collection.to_string(),
            key: key.to_string(),
        }
    }
}

/// Manages locks for transactions
pub struct LockManager {
    /// Maps a resource key to the transaction holding the exclusive lock
    /// For now, we simplify to only supporting exclusive locks for robust OLTP writes
    exclusive_locks: RwLock<HashMap<LockKey, TransactionId>>,

    /// Maps a transaction ID to the set of keys it holds (for fast release)
    tx_locks: RwLock<HashMap<TransactionId, HashSet<LockKey>>>,
}

impl LockManager {
    pub fn new() -> Self {
        Self {
            exclusive_locks: RwLock::new(HashMap::new()),
            tx_locks: RwLock::new(HashMap::new()),
        }
    }

    /// Try to acquire an exclusive lock on a key
    pub fn acquire_exclusive(
        &self,
        tx_id: TransactionId,
        database: &str,
        collection: &str,
        key: &str,
    ) -> DbResult<()> {
        let lock_key = LockKey::new(database, collection, key);

        // Check availability
        {
            let mut locks = self.exclusive_locks.write().unwrap();

            if let Some(owner) = locks.get(&lock_key) {
                if *owner == tx_id {
                    // Already locked by this transaction, re-entrant
                    return Ok(());
                }
                // Locked by someone else
                return Err(DbError::TransactionConflict(format!(
                    "Write conflict: Key {}/{}/{} is locked by transaction {}",
                    database, collection, key, owner
                )));
            }

            // Acquire lock
            locks.insert(lock_key.clone(), tx_id);
        }

        // Record in transaction's lock set
        {
            let mut tx_locks = self.tx_locks.write().unwrap();
            tx_locks.entry(tx_id).or_default().insert(lock_key);
        }

        tracing::debug!(
            "Transaction {} acquired lock on {}/{}/{}",
            tx_id,
            database,
            collection,
            key
        );

        Ok(())
    }

    /// Release all locks held by a transaction
    pub fn release_locks(&self, tx_id: TransactionId) {
        let locks_to_release = {
            let mut tx_locks = self.tx_locks.write().unwrap();
            tx_locks.remove(&tx_id)
        };

        if let Some(keys) = locks_to_release {
            let mut locks = self.exclusive_locks.write().unwrap();
            for key in keys {
                locks.remove(&key);
                tracing::debug!(
                    "Transaction {} released lock on {}/{}/{}",
                    tx_id,
                    key.database,
                    key.collection,
                    key.key
                );
            }
        }
    }
}

impl Default for LockManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lock_acquire_and_release() {
        let manager = LockManager::new();
        let tx1 = TransactionId::from_u64(1);

        // Acquire
        assert!(manager.acquire_exclusive(tx1, "db", "col", "key1").is_ok());

        // Re-acquire (re-entrant)
        assert!(manager.acquire_exclusive(tx1, "db", "col", "key1").is_ok());

        // Acquire another
        assert!(manager.acquire_exclusive(tx1, "db", "col", "key2").is_ok());

        // Verify recorded
        {
            let tx_locks = manager.tx_locks.read().unwrap();
            let keys = tx_locks.get(&tx1).unwrap();
            assert_eq!(keys.len(), 2);
        }

        // Release
        manager.release_locks(tx1);

        // Verify released
        {
            let locks = manager.exclusive_locks.read().unwrap();
            assert!(locks.is_empty());
        }
    }

    #[test]
    fn test_lock_conflict() {
        let manager = LockManager::new();
        let tx1 = TransactionId::from_u64(1);
        let tx2 = TransactionId::from_u64(2);

        manager.acquire_exclusive(tx1, "db", "col", "key1").unwrap();

        // Conflict
        let res = manager.acquire_exclusive(tx2, "db", "col", "key1");
        assert!(matches!(res, Err(DbError::TransactionConflict(_))));

        // No conflict on different key
        assert!(manager.acquire_exclusive(tx2, "db", "col", "key2").is_ok());
    }
}
