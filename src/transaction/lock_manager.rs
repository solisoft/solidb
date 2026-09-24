use super::TransactionId;
use crate::error::{DbError, DbResult};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard};

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

/// All lock state lives behind one mutex. Audit D3: the three maps used to
/// sit behind separate `RwLock`s, so `acquire_exclusive` checked under one
/// guard and inserted under another — two transactions could both pass the
/// check and both believe they held the key.
#[derive(Default)]
struct LockState {
    exclusive_locks: HashMap<LockKey, TransactionId>,
    shared_locks: HashMap<LockKey, HashSet<TransactionId>>,
    tx_locks: HashMap<TransactionId, HashSet<LockKey>>,
}

pub struct LockManager {
    state: Mutex<LockState>,
}

impl LockManager {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(LockState::default()),
        }
    }

    fn state(&self) -> MutexGuard<'_, LockState> {
        // A panic while holding the guard leaves the maps consistent (every
        // mutation is a single insert/remove), so recover from poisoning
        // rather than wedging every transaction for the life of the process.
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn acquire_shared(
        &self,
        tx_id: TransactionId,
        database: &str,
        collection: &str,
        key: &str,
    ) -> DbResult<()> {
        let lock_key = LockKey::new(database, collection, key);
        let mut state = self.state();

        if let Some(owner) = state.exclusive_locks.get(&lock_key) {
            if *owner == tx_id {
                return Ok(());
            }
            return Err(DbError::TransactionConflict(format!(
                "Write conflict: Key {}/{}/{} is locked by transaction {}",
                database, collection, key, owner
            )));
        }

        state
            .shared_locks
            .entry(lock_key.clone())
            .or_default()
            .insert(tx_id);
        state.tx_locks.entry(tx_id).or_default().insert(lock_key);
        drop(state);

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
        let mut state = self.state();
        Self::take_exclusive(&mut state, tx_id, &lock_key, "Read conflict")?;
        state.tx_locks.entry(tx_id).or_default().insert(lock_key);
        drop(state);

        tracing::debug!(
            "Transaction {} acquired EXCLUSIVE lock on {}/{}/{}",
            tx_id,
            database,
            collection,
            key
        );

        Ok(())
    }

    /// Grant `tx_id` the exclusive lock on `lock_key`, or explain why not.
    /// Caller holds the state mutex for the whole check-and-insert.
    fn take_exclusive(
        state: &mut LockState,
        tx_id: TransactionId,
        lock_key: &LockKey,
        reader_conflict: &str,
    ) -> DbResult<()> {
        if let Some(owner) = state.exclusive_locks.get(lock_key) {
            if *owner == tx_id {
                return Ok(());
            }
            return Err(DbError::TransactionConflict(format!(
                "Write conflict: Key {}/{}/{} is locked by transaction {}",
                lock_key.database, lock_key.collection, lock_key.key, owner
            )));
        }

        if let Some(readers) = state.shared_locks.get(lock_key) {
            let others = readers.iter().filter(|r| **r != tx_id).count();
            if others > 0 {
                return Err(DbError::TransactionConflict(format!(
                    "{}: Key {}/{}/{} is locked by {} reader(s)",
                    reader_conflict, lock_key.database, lock_key.collection, lock_key.key, others
                )));
            }
            // Only our own shared lock remains; the exclusive one subsumes it.
            state.shared_locks.remove(lock_key);
        }

        state.exclusive_locks.insert(lock_key.clone(), tx_id);
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
        let mut state = self.state();
        Self::take_exclusive(&mut state, tx_id, &lock_key, "Cannot upgrade")?;
        state.tx_locks.entry(tx_id).or_default().insert(lock_key);
        drop(state);

        tracing::debug!(
            "Transaction {} upgraded to EXCLUSIVE lock on {}/{}/{}",
            tx_id,
            database,
            collection,
            key
        );

        Ok(())
    }

    /// Release every lock `tx_id` holds — and only those. Audit D3: this used
    /// to drop the whole reader set and the exclusive entry for each key
    /// regardless of owner, freeing other transactions' locks.
    pub fn release_locks(&self, tx_id: TransactionId) {
        let mut state = self.state();
        let Some(keys) = state.tx_locks.remove(&tx_id) else {
            return;
        };

        for key in &keys {
            if state.exclusive_locks.get(key) == Some(&tx_id) {
                state.exclusive_locks.remove(key);
            }
            if let Some(readers) = state.shared_locks.get_mut(key) {
                readers.remove(&tx_id);
                if readers.is_empty() {
                    state.shared_locks.remove(key);
                }
            }
        }
        drop(state);

        for key in keys {
            tracing::debug!(
                "Transaction {} released lock on {}/{}/{}",
                tx_id,
                key.database,
                key.collection,
                key.key
            );
        }
    }

    pub fn get_locked_keys(&self, tx_id: TransactionId) -> Vec<(String, String, String)> {
        let state = self.state();
        state
            .tx_locks
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
            let state = manager.state();
            let keys = state.tx_locks.get(&tx1).unwrap();
            assert_eq!(keys.len(), 2);
        }

        // Release
        manager.release_locks(tx1);

        // Verify released
        {
            let state = manager.state();
            assert!(state.exclusive_locks.is_empty());
            assert!(state.tx_locks.is_empty());
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

    #[test]
    fn test_concurrent_exclusive_single_winner() {
        use std::sync::{Arc, Barrier};

        // Audit D3: with check and insert under separate guards, several
        // threads could all be granted the same key.
        for round in 0..200u64 {
            let manager = Arc::new(LockManager::new());
            let threads = 8;
            let barrier = Arc::new(Barrier::new(threads));
            let handles: Vec<_> = (0..threads as u64)
                .map(|i| {
                    let manager = manager.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        let tx = TransactionId::from_u64(round * 100 + i + 1);
                        barrier.wait();
                        manager.acquire_exclusive(tx, "db", "col", "hot").is_ok()
                    })
                })
                .collect();
            let winners = handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(|ok| *ok)
                .count();
            assert_eq!(winners, 1, "round {}: {} winners", round, winners);
        }
    }

    #[test]
    fn test_release_keeps_other_readers() {
        let manager = LockManager::new();
        let tx1 = TransactionId::from_u64(1);
        let tx2 = TransactionId::from_u64(2);
        let tx3 = TransactionId::from_u64(3);

        manager.acquire_shared(tx1, "db", "col", "k").unwrap();
        manager.acquire_shared(tx2, "db", "col", "k").unwrap();
        manager.release_locks(tx1);

        // tx2 still reads `k`, so a writer must still be refused.
        let res = manager.acquire_exclusive(tx3, "db", "col", "k");
        assert!(matches!(res, Err(DbError::TransactionConflict(_))));

        manager.release_locks(tx2);
        assert!(manager.acquire_exclusive(tx3, "db", "col", "k").is_ok());
    }

    #[test]
    fn test_release_never_frees_foreign_exclusive() {
        let manager = LockManager::new();
        let tx1 = TransactionId::from_u64(1);
        let tx2 = TransactionId::from_u64(2);

        manager.acquire_exclusive(tx1, "db", "col", "k").unwrap();
        // Simulate tx2 having recorded the key (as the old check-then-insert
        // race could leave it) without owning the exclusive lock.
        manager
            .state()
            .tx_locks
            .entry(tx2)
            .or_default()
            .insert(LockKey::new("db", "col", "k"));

        manager.release_locks(tx2);

        assert_eq!(
            manager
                .state()
                .exclusive_locks
                .get(&LockKey::new("db", "col", "k")),
            Some(&tx1)
        );
        let res = manager.acquire_exclusive(tx2, "db", "col", "k");
        assert!(matches!(res, Err(DbError::TransactionConflict(_))));
    }

    #[test]
    fn test_upgrade_refuses_foreign_exclusive() {
        let manager = LockManager::new();
        let tx1 = TransactionId::from_u64(1);
        let tx2 = TransactionId::from_u64(2);

        manager.acquire_exclusive(tx1, "db", "col", "k").unwrap();
        let res = manager.upgrade_to_exclusive(tx2, "db", "col", "k");
        assert!(matches!(res, Err(DbError::TransactionConflict(_))));

        manager.acquire_shared(tx2, "db", "col", "j").unwrap();
        assert!(manager.upgrade_to_exclusive(tx2, "db", "col", "j").is_ok());
        manager.release_locks(tx2);
        assert!(manager.acquire_exclusive(tx1, "db", "col", "j").is_ok());
    }
}
