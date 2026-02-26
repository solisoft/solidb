use super::TransactionId;
use crate::error::{DbError, DbResult};
use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockType {
    Shared,
    Exclusive,
}

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

pub struct LockManager {
    exclusive_locks: RwLock<HashMap<LockKey, TransactionId>>,
    shared_locks: RwLock<HashMap<LockKey, HashSet<TransactionId>>>,
    tx_locks: RwLock<HashMap<TransactionId, HashSet<LockKey>>>,
}

impl LockManager {
    pub fn new() -> Self {
        Self {
            exclusive_locks: RwLock::new(HashMap::new()),
            shared_locks: RwLock::new(HashMap::new()),
            tx_locks: RwLock::new(HashMap::new()),
        }
    }

    pub fn acquire_shared(
        &self,
        tx_id: TransactionId,
        database: &str,
        collection: &str,
        key: &str,
    ) -> DbResult<()> {
        let lock_key = LockKey::new(database, collection, key);

        {
            let excl_locks = self.exclusive_locks.read().unwrap();
            if let Some(owner) = excl_locks.get(&lock_key) {
                if *owner == tx_id {
                    return Ok(());
                }
                return Err(DbError::TransactionConflict(format!(
                    "Write conflict: Key {}/{}/{} is locked by transaction {}",
                    database, collection, key, owner
                )));
            }
        }

        {
            let mut shared = self.shared_locks.write().unwrap();
            shared.entry(lock_key.clone()).or_default().insert(tx_id);
        }

        {
            let mut tx_locks = self.tx_locks.write().unwrap();
            tx_locks.entry(tx_id).or_default().insert(lock_key);
        }

        tracing::debug!(
            "Transaction {} acquired SHARED lock on {}/{}/{}",
            tx_id,
            database,
            collection,
            key
        );

        Ok(())
    }

    pub fn acquire_exclusive(
        &self,
        tx_id: TransactionId,
        database: &str,
        collection: &str,
        key: &str,
    ) -> DbResult<()> {
        let lock_key = LockKey::new(database, collection, key);

        {
            let excl_locks = self.exclusive_locks.read().unwrap();
            if let Some(owner) = excl_locks.get(&lock_key) {
                if *owner == tx_id {
                    return Ok(());
                }
                return Err(DbError::TransactionConflict(format!(
                    "Write conflict: Key {}/{}/{} is locked by transaction {}",
                    database, collection, key, owner
                )));
            }
        }

        {
            let mut shared = self.shared_locks.write().unwrap();
            if let Some(readers) = shared.get(&lock_key) {
                if !readers.is_empty() && !readers.contains(&tx_id) {
                    return Err(DbError::TransactionConflict(format!(
                        "Read conflict: Key {}/{}/{} is locked by {} reader(s)",
                        database,
                        collection,
                        key,
                        readers.len()
                    )));
                }
                shared.remove(&lock_key);
            }
        }

        {
            let mut excl_locks = self.exclusive_locks.write().unwrap();
            excl_locks.insert(lock_key.clone(), tx_id);
        }

        {
            let mut tx_locks = self.tx_locks.write().unwrap();
            tx_locks.entry(tx_id).or_default().insert(lock_key);
        }

        tracing::debug!(
            "Transaction {} acquired EXCLUSIVE lock on {}/{}/{}",
            tx_id,
            database,
            collection,
            key
        );

        Ok(())
    }

    pub fn upgrade_to_exclusive(
        &self,
        tx_id: TransactionId,
        database: &str,
        collection: &str,
        key: &str,
    ) -> DbResult<()> {
        let lock_key = LockKey::new(database, collection, key);

        {
            let mut shared = self.shared_locks.write().unwrap();
            if let Some(readers) = shared.get(&lock_key) {
                if readers.len() > 1 || (readers.len() == 1 && !readers.contains(&tx_id)) {
                    return Err(DbError::TransactionConflict(format!(
                        "Cannot upgrade: Key {}/{}/{} has other readers",
                        database, collection, key
                    )));
                }
                shared.remove(&lock_key);
            }
        }

        {
            let mut excl_locks = self.exclusive_locks.write().unwrap();
            excl_locks.insert(lock_key, tx_id);
        }

        tracing::debug!(
            "Transaction {} upgraded to EXCLUSIVE lock on {}/{}/{}",
            tx_id,
            database,
            collection,
            key
        );

        Ok(())
    }

    pub fn release_locks(&self, tx_id: TransactionId) {
        let locks_to_release = {
            let mut tx_locks = self.tx_locks.write().unwrap();
            tx_locks.remove(&tx_id)
        };

        if let Some(keys) = locks_to_release {
            for key in keys {
                {
                    let mut excl_locks = self.exclusive_locks.write().unwrap();
                    excl_locks.remove(&key);
                }
                {
                    let mut shared = self.shared_locks.write().unwrap();
                    shared.remove(&key);
                }
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

    pub fn get_locked_keys(&self, tx_id: TransactionId) -> Vec<(String, String, String)> {
        let tx_locks = self.tx_locks.read().unwrap();
        tx_locks
            .get(&tx_id)
            .map(|keys| {
                keys.iter()
                    .map(|k| (k.database.clone(), k.collection.clone(), k.key.clone()))
                    .collect()
            })
            .unwrap_or_default()
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
