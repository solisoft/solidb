use super::TransactionId;
use crate::error::{DbError, DbResult};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

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
    /// The key each waiting transaction is asleep on. Who it waits for is
    /// whoever holds that key *now* ([`Self::holders`]): a key can change
    /// hands without its sleepers being woken, so a list of holders taken
    /// when the wait began would hide a deadlock with the new one.
    waits_for: HashMap<TransactionId, LockKey>,
    /// The transactions asleep on each key, in the order they arrived. A
    /// release wakes the first one of each key it frees, and nobody else:
    /// waking every sleeper for every release had all of them take the
    /// state mutex in turn to find, nearly always, a key that was not
    /// theirs.
    waiters: HashMap<LockKey, VecDeque<Waiter>>,
}

/// A transaction asleep on a key: how to wake it, and since when it waits.
struct Waiter {
    woken: Arc<Condvar>,
    since: Instant,
}

impl LockState {
    /// Whether a transaction that arrives now must queue for `key` even if
    /// it could take it: the one that has waited longest has waited `past`
    /// or more. A release wakes that one, but whoever asks before it runs
    /// takes the key, and under a steady stream of arrivals it could lose
    /// every time until its wait ran out. Overtaking is still allowed
    /// before `past`: handing every release to a sleeping thread leaves the
    /// key idle until that thread is scheduled.
    fn longest_wait_is_past(&self, key: &LockKey, past: Duration) -> bool {
        self.waiters
            .get(key)
            .and_then(VecDeque::front)
            .is_some_and(|first| first.since.elapsed() >= past)
    }

    /// The transactions holding `key`, in either mode.
    fn holders<'a>(&'a self, key: &LockKey) -> impl Iterator<Item = TransactionId> + 'a {
        self.exclusive_locks
            .get(key)
            .into_iter()
            .chain(self.shared_locks.get(key).into_iter().flatten())
            .copied()
    }

    /// Wake the transaction that has waited longest for `key`.
    fn wake_next(&self, key: &LockKey) {
        if let Some(first) = self.waiters.get(key).and_then(VecDeque::front) {
            first.woken.notify_one();
        }
    }

    /// `me` no longer waits for `key`. Unless it left with the exclusive
    /// lock, the next in line looks: it may have been `me` that a release
    /// woke, and the key may be free (or shareable).
    fn leave(&mut self, key: &LockKey, me: &Arc<Condvar>, holds_exclusive: bool) {
        if let Some(queue) = self.waiters.get_mut(key) {
            queue.retain(|w| !Arc::ptr_eq(&w.woken, me));
            if queue.is_empty() {
                self.waiters.remove(key);
            }
        }
        if !holds_exclusive {
            self.wake_next(key);
        }
    }
}

/// How long a transaction waits for a lock another one holds before it is
/// refused, unless `SOLIDB_TX_LOCK_WAIT_MS` says otherwise (`0`: never
/// wait, the behaviour up to 2.9.3).
const DEFAULT_LOCK_WAIT: Duration = Duration::from_millis(500);

/// The fraction of the wait after which the transaction that has waited
/// longest for a key is no longer overtaken by those that arrive
/// ([`LockState::longest_wait_is_past`]): a quarter, 125 ms by default.
const OVERTAKING_ENDS: u32 = 4;

/// Whether the calling thread may wait for a lock.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Waits {
    /// On a multi-threaded tokio runtime (the server), handing the worker's
    /// other tasks to another thread for the time of the wait. Not on a
    /// current-thread runtime, where the transaction that holds the lock
    /// could not run meanwhile, nor outside any runtime.
    WhereTheHolderCanRun,
    /// On any thread: for tests that drive transactions from plain threads.
    #[cfg(test)]
    Always,
}

/// A lock that could not be taken: who holds it, and what to say.
struct Blocked {
    holders: Vec<TransactionId>,
    conflict: String,
}

pub struct LockManager {
    state: Mutex<LockState>,
    wait: Duration,
    waits: Waits,
}

impl LockManager {
    pub fn new() -> Self {
        let wait = std::env::var("SOLIDB_TX_LOCK_WAIT_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(DEFAULT_LOCK_WAIT, Duration::from_millis);
        Self {
            state: Mutex::new(LockState::default()),
            wait,
            waits: Waits::WhereTheHolderCanRun,
        }
    }

    /// A manager whose locks wait up to `wait` on any thread.
    #[cfg(test)]
    fn waiting(wait: Duration) -> Self {
        Self {
            state: Mutex::new(LockState::default()),
            wait,
            waits: Waits::Always,
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
        self.acquire(
            tx_id,
            LockKey::new(database, collection, key),
            LockType::Shared,
            "",
        )?;
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
        self.acquire_exclusive_for(tx_id, database, collection, key, true)
    }

    /// [`Self::acquire_exclusive`], waiting for a held lock only if
    /// `may_wait`. A transaction whose reads are checked at commit does not
    /// wait: once the holder commits, what this one read is stale and it
    /// is refused anyway, later.
    pub fn acquire_exclusive_for(
        &self,
        tx_id: TransactionId,
        database: &str,
        collection: &str,
        key: &str,
        may_wait: bool,
    ) -> DbResult<()> {
        self.acquire_or_refuse(
            tx_id,
            LockKey::new(database, collection, key),
            LockType::Exclusive,
            "Read conflict",
            may_wait,
        )?;
        tracing::debug!(
            "Transaction {} acquired EXCLUSIVE lock on {}/{}/{}",
            tx_id,
            database,
            collection,
            key
        );
        Ok(())
    }

    /// Take the lock, waiting for the transactions that hold it to finish:
    /// up to the manager's wait, and not at all when waiting would close a
    /// cycle of transactions waiting for each other (a deadlock), which is
    /// refused at once. Refusals are `TransactionConflict`s: roll back and
    /// retry.
    ///
    /// Up to 2.9.3 a held lock was refused at once. Among ten hot accounts
    /// that was more than one refused attempt per committed transfer, each a
    /// rolled-back transaction and a retry after a backoff.
    fn acquire(
        &self,
        tx_id: TransactionId,
        lock_key: LockKey,
        kind: LockType,
        reader_conflict: &str,
    ) -> DbResult<()> {
        self.acquire_or_refuse(tx_id, lock_key, kind, reader_conflict, true)
    }

    fn acquire_or_refuse(
        &self,
        tx_id: TransactionId,
        lock_key: LockKey,
        kind: LockType,
        reader_conflict: &str,
        may_wait: bool,
    ) -> DbResult<()> {
        let mut state = self.state();
        let mut deadline: Option<Instant> = None;
        // This transaction's place in the key's queue, once it has slept.
        let mut me: Option<Arc<Condvar>> = None;
        loop {
            // A newcomer that can wait lets a long waiter go first. One
            // that holds the key already does not: the waiters wait for it.
            let yields = me.is_none()
                && may_wait
                && !self.wait.is_zero()
                && state.longest_wait_is_past(&lock_key, self.wait / OVERTAKING_ENDS)
                && !state.holders(&lock_key).any(|h| h == tx_id)
                && self.may_wait();
            let blocked = if yields {
                Err(Blocked {
                    holders: state.holders(&lock_key).collect(),
                    conflict: format!(
                        "Write conflict: Key {}/{}/{} has transactions waiting for it",
                        lock_key.database, lock_key.collection, lock_key.key
                    ),
                })
            } else {
                match kind {
                    LockType::Shared => Self::take_shared(&mut state, tx_id, &lock_key),
                    LockType::Exclusive => {
                        Self::take_exclusive(&mut state, tx_id, &lock_key, reader_conflict)
                    }
                }
            };
            let blocked = match blocked {
                Ok(()) => {
                    state.waits_for.remove(&tx_id);
                    if let Some(me) = &me {
                        state.leave(&lock_key, me, kind == LockType::Exclusive);
                    }
                    state.tx_locks.entry(tx_id).or_default().insert(lock_key);
                    return Ok(());
                }
                Err(blocked) => blocked,
            };
            let refuse = |state: &mut LockState, why: String| {
                state.waits_for.remove(&tx_id);
                if let Some(me) = &me {
                    state.leave(&lock_key, me, false);
                }
                Err(DbError::TransactionConflict(why))
            };
            if !may_wait || self.wait.is_zero() || !self.may_wait() {
                return refuse(&mut state, blocked.conflict);
            }
            if Self::closes_a_cycle(&state, tx_id, &blocked.holders) {
                return refuse(
                    &mut state,
                    format!(
                        "{} (deadlock: that transaction is waiting for this one)",
                        blocked.conflict
                    ),
                );
            }
            let now = Instant::now();
            let until = *deadline.get_or_insert(now + self.wait);
            if now >= until {
                return refuse(
                    &mut state,
                    format!("{} (waited {} ms)", blocked.conflict, self.wait.as_millis()),
                );
            }
            state.waits_for.insert(tx_id, lock_key.clone());
            let woken = me.get_or_insert_with(|| {
                let woken = Arc::new(Condvar::new());
                state
                    .waiters
                    .entry(lock_key.clone())
                    .or_default()
                    .push_back(Waiter {
                        woken: woken.clone(),
                        since: now,
                    });
                woken
            });
            state = Self::wait_for_release(woken, state, until - now);
        }
    }

    /// Whether this thread may sleep until a lock is released.
    fn may_wait(&self) -> bool {
        match self.waits {
            Waits::WhereTheHolderCanRun => tokio::runtime::Handle::try_current()
                .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread),
            #[cfg(test)]
            Waits::Always => true,
        }
    }

    /// Sleep until `woken` is signalled, or `timeout`. On a runtime worker the
    /// worker's other tasks move to another thread first: the transaction
    /// that holds the lock may be one of them.
    fn wait_for_release<'a>(
        woken: &Condvar,
        state: MutexGuard<'a, LockState>,
        timeout: Duration,
    ) -> MutexGuard<'a, LockState> {
        let sleep = || {
            woken
                .wait_timeout(state, timeout)
                .map_or_else(|p| p.into_inner().0, |(state, _)| state)
        };
        let on_worker = tokio::runtime::Handle::try_current()
            .is_ok_and(|h| h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread);
        if on_worker {
            tokio::task::block_in_place(sleep)
        } else {
            sleep()
        }
    }

    /// Whether `tx_id` waiting for `holders` would wait for itself: one of
    /// them, or one they wait for in turn, is waiting for `tx_id`.
    fn closes_a_cycle(state: &LockState, tx_id: TransactionId, holders: &[TransactionId]) -> bool {
        let mut seen: HashSet<TransactionId> = HashSet::new();
        let mut todo: Vec<TransactionId> = holders.to_vec();
        while let Some(holder) = todo.pop() {
            if holder == tx_id {
                return true;
            }
            if seen.insert(holder) {
                if let Some(key) = state.waits_for.get(&holder) {
                    todo.extend(state.holders(key).filter(|h| *h != holder));
                }
            }
        }
        false
    }

    /// Grant `tx_id` a shared lock on `lock_key`, or say who holds it
    /// exclusively. Caller holds the state mutex.
    fn take_shared(
        state: &mut LockState,
        tx_id: TransactionId,
        lock_key: &LockKey,
    ) -> Result<(), Blocked> {
        if let Some(owner) = state.exclusive_locks.get(lock_key) {
            if *owner == tx_id {
                return Ok(());
            }
            return Err(Blocked {
                holders: vec![*owner],
                conflict: format!(
                    "Write conflict: Key {}/{}/{} is locked by transaction {}",
                    lock_key.database, lock_key.collection, lock_key.key, owner
                ),
            });
        }
        state
            .shared_locks
            .entry(lock_key.clone())
            .or_default()
            .insert(tx_id);
        Ok(())
    }

    /// Grant `tx_id` the exclusive lock on `lock_key`, or say who holds it.
    /// Caller holds the state mutex for the whole check-and-insert.
    fn take_exclusive(
        state: &mut LockState,
        tx_id: TransactionId,
        lock_key: &LockKey,
        reader_conflict: &str,
    ) -> Result<(), Blocked> {
        if let Some(owner) = state.exclusive_locks.get(lock_key) {
            if *owner == tx_id {
                return Ok(());
            }
            return Err(Blocked {
                holders: vec![*owner],
                conflict: format!(
                    "Write conflict: Key {}/{}/{} is locked by transaction {}",
                    lock_key.database, lock_key.collection, lock_key.key, owner
                ),
            });
        }

        if let Some(readers) = state.shared_locks.get(lock_key) {
            let others: Vec<TransactionId> =
                readers.iter().filter(|r| **r != tx_id).copied().collect();
            if !others.is_empty() {
                return Err(Blocked {
                    conflict: format!(
                        "{}: Key {}/{}/{} is locked by {} reader(s)",
                        reader_conflict,
                        lock_key.database,
                        lock_key.collection,
                        lock_key.key,
                        others.len()
                    ),
                    holders: others,
                });
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
        self.acquire(
            tx_id,
            LockKey::new(database, collection, key),
            LockType::Exclusive,
            "Cannot upgrade",
        )?;
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
        state.waits_for.remove(&tx_id);
        // Whoever has waited longest for each of these keys looks again.
        for key in &keys {
            state.wake_next(key);
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

    /// A held lock is waited for: the waiter gets it when the holder is
    /// done, and is refused only once its wait is over.
    #[test]
    fn a_held_lock_is_waited_for() {
        use std::sync::Arc;
        let manager = Arc::new(LockManager::waiting(Duration::from_secs(5)));
        let (tx1, tx2) = (TransactionId::from_u64(1), TransactionId::from_u64(2));
        manager.acquire_exclusive(tx1, "db", "col", "k").unwrap();

        let waiter = {
            let manager = manager.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                manager
                    .acquire_exclusive(tx2, "db", "col", "k")
                    .map(|()| started.elapsed())
            })
        };
        std::thread::sleep(Duration::from_millis(100));
        manager.release_locks(tx1);
        let waited = waiter.join().unwrap().expect("granted once released");
        assert!(waited >= Duration::from_millis(90), "{waited:?}");
        assert_eq!(
            manager
                .state()
                .exclusive_locks
                .get(&LockKey::new("db", "col", "k")),
            Some(&tx2)
        );

        // Nobody releases it: refused after the wait, not before.
        let short = LockManager::waiting(Duration::from_millis(60));
        short.acquire_exclusive(tx1, "db", "col", "k").unwrap();
        let started = Instant::now();
        let res = short.acquire_shared(tx2, "db", "col", "k");
        assert!(matches!(&res, Err(DbError::TransactionConflict(m)) if m.contains("waited")));
        assert!(started.elapsed() >= Duration::from_millis(60));
        assert!(short.state().waits_for.is_empty());
    }

    /// Many transactions asleep on one key each get it in turn, woken one
    /// at a time; sleepers on another key are not kept from theirs, and
    /// nothing is left in the queues.
    #[test]
    fn each_release_wakes_the_next_in_line() {
        use std::sync::atomic::{AtomicU64, Ordering};
        use std::sync::Arc;
        let manager = Arc::new(LockManager::waiting(Duration::from_secs(20)));
        let holder = TransactionId::from_u64(1);
        manager.acquire_exclusive(holder, "db", "col", "a").unwrap();
        manager.acquire_exclusive(holder, "db", "col", "b").unwrap();

        let inside = Arc::new(AtomicU64::new(0));
        let threads: Vec<_> = (0..16u64)
            .map(|i| {
                let (manager, inside) = (manager.clone(), inside.clone());
                std::thread::spawn(move || {
                    let tx = TransactionId::from_u64(10 + i);
                    let key = if i % 2 == 0 { "a" } else { "b" };
                    // Readers of `b` share it; writers of `a` exclude.
                    if key == "a" {
                        manager.acquire_exclusive(tx, "db", "col", key).unwrap();
                        assert_eq!(inside.fetch_add(1, Ordering::SeqCst), 0);
                        std::thread::sleep(Duration::from_millis(2));
                        inside.fetch_sub(1, Ordering::SeqCst);
                    } else {
                        manager.acquire_shared(tx, "db", "col", key).unwrap();
                    }
                    manager.release_locks(tx);
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            manager
                .state()
                .waiters
                .values()
                .map(|q| q.len())
                .sum::<usize>(),
            16
        );
        manager.release_locks(holder);
        for t in threads {
            t.join().unwrap();
        }
        let state = manager.state();
        assert!(state.waiters.is_empty() && state.waits_for.is_empty());
        assert!(state.exclusive_locks.is_empty() && state.shared_locks.is_empty());
    }

    /// A transaction that has waited long enough is not overtaken: one that
    /// arrives when the key is free queues behind it, where before it took
    /// the key, and a stream of arrivals could starve the waiter until its
    /// wait ran out.
    #[test]
    fn a_long_waiter_is_not_overtaken() {
        use std::sync::Arc;
        let manager = Arc::new(LockManager::waiting(Duration::from_millis(400)));
        let (holder, waiter, late) = (
            TransactionId::from_u64(1),
            TransactionId::from_u64(2),
            TransactionId::from_u64(3),
        );
        let key = LockKey::new("db", "col", "k");
        manager.acquire_exclusive(holder, "db", "col", "k").unwrap();
        let waiting = {
            let manager = manager.clone();
            std::thread::spawn(move || manager.acquire_exclusive(waiter, "db", "col", "k"))
        };
        // Past a quarter of the wait.
        std::thread::sleep(Duration::from_millis(150));

        // Free the key and ask for it at once, without letting the waiter
        // run in between: the state mutex is held across both.
        let asks = {
            let manager = manager.clone();
            let mut state = manager.state();
            let gate = state.exclusive_locks.remove(&key);
            assert_eq!(gate, Some(holder));
            state.tx_locks.remove(&holder);
            let asks = {
                let manager = manager.clone();
                std::thread::spawn(move || manager.acquire_exclusive(late, "db", "col", "k"))
            };
            // The late one is blocked on the mutex, then queues.
            std::thread::sleep(Duration::from_millis(30));
            state.wake_next(&key);
            drop(state);
            asks
        };
        waiting.join().unwrap().expect("the waiter gets the key");
        assert_eq!(manager.state().exclusive_locks.get(&key), Some(&waiter));
        manager.release_locks(waiter);
        asks.join().unwrap().expect("then the late one");
        manager.release_locks(late);
        let state = manager.state();
        assert!(state.waiters.is_empty() && state.waits_for.is_empty());

        // Before that, an arrival takes a free key although someone waits.
        drop(state);
        manager.acquire_exclusive(holder, "db", "col", "k").unwrap();
        let waiting = {
            let manager = manager.clone();
            std::thread::spawn(move || manager.acquire_exclusive(waiter, "db", "col", "k"))
        };
        std::thread::sleep(Duration::from_millis(30));
        {
            let mut state = manager.state();
            state.exclusive_locks.remove(&key);
            state.tx_locks.remove(&holder);
            assert!(LockManager::take_exclusive(&mut state, late, &key, "").is_ok());
            assert!(!state.longest_wait_is_past(&key, manager.wait / OVERTAKING_ENDS));
            state.exclusive_locks.remove(&key);
            state.wake_next(&key);
        }
        waiting.join().unwrap().unwrap();
    }

    /// Transfers among ten keys from thirty-two threads. Taking the two
    /// keys in order nothing is refused; in any order, only deadlocks are,
    /// and at once: nobody is left asleep on a free key, nor in a cycle
    /// that formed when a key changed hands while they slept, so nobody's
    /// wait runs out.
    #[test]
    fn no_wake_and_no_deadlock_is_missed_among_a_few_hot_keys() {
        hot_keys(true);
        hot_keys(false);
    }

    fn hot_keys(in_order: bool) {
        use std::sync::Arc;
        let manager = Arc::new(LockManager::waiting(Duration::from_secs(10)));
        let threads: Vec<_> = (0..32u64)
            .map(|t| {
                let manager = manager.clone();
                std::thread::spawn(move || {
                    let mut x = 0x9E37_79B9_7F4A_7C15u64.wrapping_mul(t + 1);
                    for i in 0..2_000u64 {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        let (a, b) = (x % 10, (x >> 8) % 10);
                        if a == b {
                            continue;
                        }
                        let tx = TransactionId::from_u64(1 + t * 1_000_000 + i);
                        let keys = if in_order {
                            [a.min(b), a.max(b)]
                        } else {
                            [a, b]
                        };
                        for k in keys {
                            match manager.acquire_exclusive(tx, "db", "col", &k.to_string()) {
                                Ok(()) => {}
                                Err(DbError::TransactionConflict(m))
                                    if !in_order && m.contains("deadlock") =>
                                {
                                    break
                                }
                                Err(e) => panic!("in_order {in_order}: {e}"),
                            }
                        }
                        manager.release_locks(tx);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert!(manager.state().waiters.is_empty());
    }

    /// A sleeper that gives up hands its turn on: the one behind it is not
    /// left asleep on a key that is free.
    #[test]
    fn a_waiter_that_gives_up_wakes_the_one_behind_it() {
        use std::sync::Arc;
        let manager = Arc::new(LockManager::waiting(Duration::from_millis(150)));
        let (tx1, tx2, tx3) = (
            TransactionId::from_u64(1),
            TransactionId::from_u64(2),
            TransactionId::from_u64(3),
        );
        manager.acquire_exclusive(tx1, "db", "col", "k").unwrap();
        let first = {
            let manager = manager.clone();
            std::thread::spawn(move || manager.acquire_exclusive(tx2, "db", "col", "k"))
        };
        std::thread::sleep(Duration::from_millis(100));
        // Arrives 100 ms later, so its own wait ends 100 ms after tx2's.
        let second = {
            let manager = manager.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                manager
                    .acquire_exclusive(tx3, "db", "col", "k")
                    .map(|()| started.elapsed())
            })
        };
        assert!(first.join().unwrap().is_err(), "tx2 waited its 150 ms out");
        // tx1 releases while only tx3 sleeps: it is woken, not left to
        // its timeout.
        manager.release_locks(tx1);
        let waited = second.join().unwrap().expect("granted");
        assert!(waited < Duration::from_millis(140), "{waited:?}");
    }

    /// Two transactions each holding what the other wants: the one that
    /// would close the cycle is refused at once, and the other goes on when
    /// it gives its lock up.
    #[test]
    fn a_deadlock_is_refused_at_once() {
        use std::sync::Arc;
        let manager = Arc::new(LockManager::waiting(Duration::from_secs(30)));
        let (tx1, tx2) = (TransactionId::from_u64(1), TransactionId::from_u64(2));
        manager.acquire_exclusive(tx1, "db", "col", "a").unwrap();
        manager.acquire_exclusive(tx2, "db", "col", "b").unwrap();

        let first = {
            let manager = manager.clone();
            std::thread::spawn(move || manager.acquire_exclusive(tx1, "db", "col", "b"))
        };
        // Wait until tx1 is waiting for tx2.
        while !manager.state().waits_for.contains_key(&tx1) {
            std::thread::yield_now();
        }
        let started = Instant::now();
        let res = manager.acquire_exclusive(tx2, "db", "col", "a");
        assert!(
            matches!(&res, Err(DbError::TransactionConflict(m)) if m.contains("deadlock")),
            "{res:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(5));

        manager.release_locks(tx2);
        first.join().unwrap().expect("tx1 gets b once tx2 let go");
    }

    /// Without a runtime that can run the holder meanwhile (here: no
    /// runtime at all), a held lock is refused at once, as it always was.
    #[test]
    fn no_wait_where_the_holder_could_not_run() {
        let manager = LockManager::new();
        let (tx1, tx2) = (TransactionId::from_u64(1), TransactionId::from_u64(2));
        manager.acquire_exclusive(tx1, "db", "col", "k").unwrap();
        let started = Instant::now();
        assert!(manager.acquire_exclusive(tx2, "db", "col", "k").is_err());
        assert!(started.elapsed() < Duration::from_millis(200));
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
