//! Pending column-family drop registry.
//!
//! Dropping a RocksDB column family rewrites + fsyncs the entire OPTIONS
//! file (one section per CF) under the DB mutex, so with thousands of CFs
//! each `drop_cf` costs hundreds of milliseconds. `delete_database` would
//! otherwise block for `collections × that cost` (measured: 18s for a
//! 25-collection database on a 1794-CF instance).
//!
//! Instead, `delete_database` removes the database from `_meta` immediately
//! (making it invisible to all metadata-driven paths) and schedules the CF
//! drops here; a background thread performs the expensive drops while the
//! request returns instantly.
//!
//! Each scheduled drop is persisted as a `pending_drop:{cf}` marker in the
//! `_meta` CF — written in the same atomic batch that deletes the `db:{name}`
//! key — so drops interrupted by a crash or restart are resumed on startup.
//!
//! Recreating a database/collection with the same name while its old CF is
//! still doomed is handled by *claiming*: the creator atomically takes
//! ownership of a `Pending` CF, drops it synchronously, and recreates it
//! fresh. If the background dropper is mid-`drop_cf` on that exact CF
//! (`Dropping`), the creator waits for it to finish instead.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use rust_rocksdb::WriteBatch;

use super::engine::META_CF;
use super::RocksDb as DB;
use crate::error::{DbError, DbResult};

/// `_meta` key prefix for persisted drop markers.
const MARKER_PREFIX: &str = "pending_drop:";

/// How long a deleted collection's column family is kept around so a
/// same-name recreate can reuse it instead of paying `drop_cf` + `create_cf`.
///
/// Observed on a dev instance: the same collection name is deleted and
/// auto-created again minutes apart by successive test runs
/// (`default_spec/casc_profile_owners`, two minutes; `csw_admin_test/orders`,
/// 358 times over four days). A grace measured in seconds would almost never
/// catch those, so the default is generous — the column family holds no data
/// while it waits, because `delete_collection` wipes it up front.
const DEFAULT_REUSE_GRACE_SECS: u64 = 300;

/// How often the reaper looks for column families whose grace has expired.
const REAP_INTERVAL: Duration = Duration::from_secs(5);

/// Slice length for interruptible sleeps, so shutdown never waits a full tick.
const SLEEP_SLICE: Duration = Duration::from_millis(50);

/// Shortest pause after a background drop, even a fast one.
const MIN_BREATHER: Duration = Duration::from_millis(25);

/// One background `drop_cf` at a time, for the whole process.
///
/// `rust-rocksdb`'s `drop_cf` keeps the column-family map's *write* lock for
/// the whole call — the guard is a temporary of its `match` scrutinee — and
/// the call rewrites the entire OPTIONS file: 0.3–0.7s on an instance with a
/// few thousand CFs. Every `cf_handle`, so every read and write in every
/// database, takes that lock to read. With one dropper thread per deleted
/// database (a test suite drops its three worker databases at once) and 25ms
/// between drops, the lock was taken back to back for as long as the drops
/// lasted: a one-document read on an unrelated database was measured at
/// 33.7s while three worker databases were being dropped (25/09/2026).
static DROP_GATE: Mutex<()> = Mutex::new(());

/// Background drops running right now, and the most ever seen at once — so a
/// test can prove the gate holds.
#[cfg(test)]
static DROPS_IN_FLIGHT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
static MOST_DROPS_IN_FLIGHT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

fn reuse_grace() -> Duration {
    let secs = std::env::var("SOLIDB_CF_REUSE_GRACE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_REUSE_GRACE_SECS);
    Duration::from_secs(secs)
}

#[derive(Clone, Copy, PartialEq)]
enum DropState {
    /// Scheduled, not yet started — a recreate may claim it.
    Pending,
    /// `drop_cf` is executing right now (by the dropper or a claimant).
    Dropping,
}

/// Outcome of [`PendingCfDrops::claim_for_recreate`].
pub enum Claim {
    /// Caller now owns the doomed CF: drop it, then call `complete`.
    Claimed,
    /// The background dropper is mid-drop on this CF — wait for it.
    InProgress,
    /// The CF is not scheduled for drop.
    NotPending,
}

/// In-memory registry of CFs awaiting a background drop, backed by
/// persisted `pending_drop:{cf}` markers in `_meta` for crash recovery.
#[derive(Default)]
pub struct PendingCfDrops {
    states: DashMap<String, DropState>,
    /// Handles for the threads spawned by [`Self::spawn_dropper`].
    ///
    /// These used to be dropped on the floor. A dropper still inside RocksDB's
    /// OPTIONS rewrite when the process exits races the static destructors that
    /// free RocksDB's global option-type registry, and the process dies with
    /// SIGSEGV or `std::bad_alloc`/SIGABRT *after* every test has reported ok.
    /// Keeping the handles lets [`Self::join_droppers`] close that window.
    droppers: Mutex<Vec<JoinHandle<()>>>,
    /// When each CF was scheduled, for the reuse grace period.
    scheduled_at: DashMap<String, Instant>,
    /// Set by [`Self::join_droppers`]. A reaper that sees it exits without
    /// dropping anything: the persisted markers survive, and the next startup
    /// resumes them. Shutting down is not a reason to pay for OPTIONS
    /// rewrites.
    shutting_down: Arc<AtomicBool>,
    /// Whether the single reaper thread has been started.
    reaper_started: AtomicBool,
}

impl PendingCfDrops {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// True if this CF is scheduled (or mid-drop) — callers must treat such
    /// collections as already deleted.
    pub fn contains(&self, cf_name: &str) -> bool {
        self.states.contains_key(cf_name)
    }

    /// Atomically persist drop markers for `cfs` and delete the database's
    /// `db:{name}` metadata key (`db_meta_key`) in one write batch, then
    /// register the CFs in memory.
    pub fn schedule(&self, db: &DB, db_meta_key: &str, cfs: &[String]) -> DbResult<()> {
        let meta_cf = db
            .cf_handle(META_CF)
            .ok_or_else(|| DbError::InternalError("_meta column family missing".to_string()))?;

        let mut batch = WriteBatch::default();
        batch.delete_cf(&meta_cf, db_meta_key.as_bytes());
        for cf in cfs {
            batch.put_cf(
                &meta_cf,
                format!("{}{}", MARKER_PREFIX, cf).as_bytes(),
                b"1",
            );
            // Deregister in the same batch: a collection must never be listed
            // as existing while its column family is doomed.
            batch.delete_cf(
                &meta_cf,
                super::collection_registry::entry_key(cf).as_bytes(),
            );
        }
        db.write(&batch).map_err(|e| {
            DbError::InternalError(format!("Failed to schedule collection drops: {}", e))
        })?;

        let now = Instant::now();
        for cf in cfs {
            self.states.insert(cf.clone(), DropState::Pending);
            self.scheduled_at.insert(cf.clone(), now);
        }
        Ok(())
    }

    /// Persist a drop marker for a single CF, with no database metadata key
    /// to remove alongside it.
    ///
    /// This is [`Self::schedule`] for a lone collection: `delete_collection`
    /// used to call `drop_cf` inline, paying a full OPTIONS rewrite before it
    /// could return, and leaving nothing for a same-name recreate to claim —
    /// so a create/delete loop paid two rewrites per cycle. Deferring the drop
    /// makes the collection invisible immediately (every lookup path filters
    /// on [`Self::contains`]) and lets a recreate reuse the CF in place.
    pub fn schedule_one(&self, db: &DB, cf_name: &str) -> DbResult<()> {
        let meta_cf = db
            .cf_handle(META_CF)
            .ok_or_else(|| DbError::InternalError("_meta column family missing".to_string()))?;

        db.put_cf(
            &meta_cf,
            format!("{}{}", MARKER_PREFIX, cf_name).as_bytes(),
            b"1",
        )
        .map_err(|e| {
            DbError::InternalError(format!("Failed to schedule collection drop: {}", e))
        })?;

        self.states.insert(cf_name.to_string(), DropState::Pending);
        self.scheduled_at
            .insert(cf_name.to_string(), Instant::now());
        Ok(())
    }

    /// Load persisted markers left by a previous run (crash / restart during
    /// a background drop) and re-register them. Returns the CFs to drop.
    pub fn resume_from_meta(&self, db: &DB) -> Vec<String> {
        let meta_cf = match db.cf_handle(META_CF) {
            Some(cf) => cf,
            None => return vec![],
        };

        let iter = db.prefix_iterator_cf(&meta_cf, MARKER_PREFIX.as_bytes());
        let cfs: Vec<String> = iter
            .filter_map(|result| {
                result.ok().and_then(|(key, _)| {
                    let key_str = String::from_utf8(key.to_vec()).ok()?;
                    key_str.strip_prefix(MARKER_PREFIX).map(|s| s.to_string())
                })
            })
            .collect();

        // Markers from a previous run get no grace — whatever might have
        // reused them is long gone.
        let expired = Instant::now() - reuse_grace();
        for cf in &cfs {
            self.states.insert(cf.clone(), DropState::Pending);
            self.scheduled_at.insert(cf.clone(), expired);
        }
        cfs
    }

    /// Attempt to take ownership of a doomed CF so it can be dropped
    /// synchronously and recreated fresh (collection re-created with the
    /// same name before the background drop got to it).
    pub fn claim_for_recreate(&self, cf_name: &str) -> Claim {
        use dashmap::mapref::entry::Entry;
        match self.states.entry(cf_name.to_string()) {
            Entry::Occupied(mut entry) => match entry.get() {
                DropState::Pending => {
                    *entry.get_mut() = DropState::Dropping;
                    Claim::Claimed
                }
                DropState::Dropping => Claim::InProgress,
            },
            Entry::Vacant(_) => Claim::NotPending,
        }
    }

    /// Hand a claimed CF back (claimant failed to drop it) so the background
    /// dropper or a restart retries it.
    pub fn release_claim(&self, cf_name: &str) {
        self.states.insert(cf_name.to_string(), DropState::Pending);
    }

    /// Mark the drop finished: delete the persisted marker and forget the CF.
    pub fn complete(&self, db: &DB, cf_name: &str) {
        if let Some(meta_cf) = db.cf_handle(META_CF) {
            let _ = db.delete_cf(&meta_cf, format!("{}{}", MARKER_PREFIX, cf_name).as_bytes());
        }
        self.states.remove(cf_name);
        self.scheduled_at.remove(cf_name);
        // A same-name recreate must not inherit this incarnation's cached
        // index definitions.
        super::collection::index_meta::invalidate_index_meta(db, cf_name);
    }

    /// Block until the background dropper finishes this CF (used when a
    /// recreate races the in-flight `drop_cf` of the same name).
    pub fn wait_until_dropped(&self, cf_name: &str, timeout: Duration) -> DbResult<()> {
        let start = Instant::now();
        while self.states.contains_key(cf_name) {
            if start.elapsed() > timeout {
                return Err(DbError::InternalError(format!(
                    "Timed out waiting for pending drop of collection '{}'",
                    cf_name
                )));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    /// Dropper-side claim: only proceeds on CFs still `Pending` (a recreate
    /// may have claimed them in the meantime).
    fn begin_drop(&self, cf_name: &str) -> bool {
        use dashmap::mapref::entry::Entry;
        match self.states.entry(cf_name.to_string()) {
            Entry::Occupied(mut entry) if *entry.get() == DropState::Pending => {
                *entry.get_mut() = DropState::Dropping;
                true
            }
            _ => false,
        }
    }

    /// Interruptible sleep: returns `false` if shutdown was signalled.
    fn nap(&self, total: Duration) -> bool {
        let deadline = Instant::now() + total;
        while Instant::now() < deadline {
            if self.shutting_down.load(Ordering::Relaxed) {
                return false;
            }
            std::thread::sleep(SLEEP_SLICE.min(deadline - Instant::now()));
        }
        !self.shutting_down.load(Ordering::Relaxed)
    }

    /// Drop `cf` without starving everyone else of the column-family lock.
    ///
    /// Takes [`DROP_GATE`], so background drops never overlap, and keeps it
    /// through a pause as long as the drop itself: whatever queued behind the
    /// lock gets through before the next drop takes it again. A request thus
    /// waits for one drop at most, and drops hold the lock half the time at
    /// most. The flag is `false` when shutdown cut the pause short.
    fn drop_in_background(&self, db: &DB, cf: &str) -> (Result<(), rust_rocksdb::Error>, bool) {
        // Poisoned only if a drop panicked; the gate guards no data.
        let _gate = DROP_GATE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let start = Instant::now();
        #[cfg(test)]
        {
            let now = DROPS_IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1;
            MOST_DROPS_IN_FLIGHT.fetch_max(now, Ordering::SeqCst);
        }
        let result = super::cf_ops::timed(|| db.drop_cf(cf));
        #[cfg(test)]
        DROPS_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        let keep_going = self.nap(start.elapsed().max(MIN_BREATHER));
        (result, keep_going)
    }

    /// CFs whose reuse grace has run out.
    fn due_for_drop(&self, grace: Duration) -> Vec<String> {
        self.scheduled_at
            .iter()
            .filter(|entry| entry.value().elapsed() >= grace)
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Start the single reaper thread, if it is not already running.
    ///
    /// A deleted collection's CF is kept for [`reuse_grace`] so a same-name
    /// recreate can wipe and reuse it — two OPTIONS rewrites saved — and this
    /// is what eventually reclaims the ones nobody came back for. One thread
    /// for the whole process: spawning one per deletion would put a suite that
    /// drops a hundred collections into a hundred sleeping threads.
    pub fn ensure_reaper(db: Arc<DB>, registry: Arc<Self>) {
        if registry.reaper_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let grace = reuse_grace();
        let registry_for_handle = Arc::clone(&registry);
        let handle = std::thread::spawn(move || {
            loop {
                if !registry.nap(REAP_INTERVAL) {
                    return;
                }
                for cf in registry.due_for_drop(grace) {
                    if registry.shutting_down.load(Ordering::Relaxed) {
                        return;
                    }
                    // A recreate may have claimed it since the scan.
                    if !registry.begin_drop(&cf) {
                        continue;
                    }
                    let mut keep_going = true;
                    if db.cf_handle(&cf).is_some() {
                        let (result, carry_on) = registry.drop_in_background(&db, &cf);
                        keep_going = carry_on;
                        if let Err(e) = result {
                            tracing::warn!("Reaping column family '{}' failed: {}", cf, e);
                            registry.release_claim(&cf);
                            if !keep_going {
                                return;
                            }
                            continue;
                        }
                    }
                    registry.complete(&db, &cf);
                    if !keep_going {
                        return;
                    }
                }
            }
        });

        let locked = registry_for_handle.droppers.lock();
        if let Ok(mut handles) = locked {
            handles.retain(|h| !h.is_finished());
            handles.push(handle);
        }
    }

    /// Drop the scheduled CFs on a background thread. Each successful drop
    /// removes its persisted marker; failed drops stay marked so they are
    /// retried on the next startup.
    pub fn spawn_dropper(db: Arc<DB>, registry: Arc<Self>, cfs: Vec<String>) {
        if cfs.is_empty() {
            return;
        }
        // The closure takes ownership of `registry`; keep a handle so the
        // JoinHandle can be filed against the same registry afterwards.
        let registry_for_handle = Arc::clone(&registry);
        let handle = std::thread::spawn(move || {
            let start = Instant::now();
            let total = cfs.len();
            let mut dropped = 0usize;
            for cf in &cfs {
                // Shutdown: leave the rest marked for the next startup.
                if registry.shutting_down.load(Ordering::Relaxed) {
                    return;
                }
                if !registry.begin_drop(cf) {
                    continue; // claimed by a concurrent recreate
                }
                // `drop_in_background` breathes after each drop: back-to-back
                // drops starve every request of the column-family lock, and a
                // concurrent recreate of the same database of the DB mutex.
                let mut keep_going = true;
                if db.cf_handle(cf).is_some() {
                    let (result, carry_on) = registry.drop_in_background(&db, cf);
                    keep_going = carry_on;
                    if let Err(e) = result {
                        tracing::warn!("Background drop of column family '{}' failed: {}", cf, e);
                        registry.release_claim(cf);
                        if !keep_going {
                            return;
                        }
                        continue;
                    }
                    dropped += 1;
                }
                registry.complete(&db, cf);
                if !keep_going {
                    return;
                }
            }
            tracing::info!(
                "Background-dropped {}/{} column families in {:.2?}",
                dropped,
                total,
                start.elapsed()
            );
        });

        // Bound to a local rather than matched inline: as the tail expression
        // of the function, an inline `if let` keeps the lock's temporary alive
        // past `registry_for_handle`, which the borrow checker rejects.
        let locked = registry_for_handle.droppers.lock();
        if let Ok(mut handles) = locked {
            // Reap the ones that have already finished, so a long-lived
            // instance that deletes many databases does not accumulate dead
            // handles for the lifetime of the process.
            handles.retain(|h| !h.is_finished());
            handles.push(handle);
        }
    }

    /// Block until every dropper thread spawned by [`Self::spawn_dropper`] has
    /// finished.
    ///
    /// Called from `StorageEngine`'s `Drop`, and only by the last live handle
    /// (see the `liveness` token there). Without it a dropper can still be
    /// inside `drop_cf` when the process tears down: RocksDB's static
    /// destructors free the global option-type registry underneath it and the
    /// process aborts with SIGSEGV or `std::bad_alloc` *after* the work is
    /// reported as successful. That is why `rbac_admin_endpoints_tests` failed
    /// at process exit with all of its tests green, and why the server had the
    /// same race on shutdown.
    ///
    /// This can block for as long as the outstanding drops take — each
    /// OPTIONS rewrite plus an equal pause after it (see
    /// [`Self::drop_in_background`]), cut short by the shutdown flag. That is
    /// the point: the alternative is exiting while RocksDB is mid-write.
    pub fn join_droppers(&self) {
        // Tell the reaper to stop before waiting on it, or the join would
        // block until its next tick — and there is no reason to spend OPTIONS
        // rewrites on the way out: the markers are persisted, so the next
        // startup resumes whatever is left.
        self.shutting_down.store(true, Ordering::Relaxed);

        let handles = match self.droppers.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            // Poisoned means a dropper panicked. Its handle is unusable and
            // there is nothing left to wait for.
            Err(_) => return,
        };
        for handle in handles {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reaper must leave a freshly deleted collection's column family
    /// alone — that window is what a same-name recreate reuses — and take it
    /// once the grace has passed.
    #[test]
    fn due_for_drop_respects_the_reuse_grace() {
        let registry = PendingCfDrops::default();
        let grace = Duration::from_secs(300);

        registry
            .scheduled_at
            .insert("db:fresh".to_string(), Instant::now());
        registry
            .scheduled_at
            .insert("db:stale".to_string(), Instant::now() - grace);

        let due = registry.due_for_drop(grace);
        assert_eq!(due, vec!["db:stale".to_string()]);
    }

    /// Markers recovered from a previous run carry an already-expired
    /// timestamp: nothing from that run is coming back to claim them.
    #[test]
    fn resumed_markers_are_immediately_due() {
        let registry = PendingCfDrops::default();
        let grace = reuse_grace();
        registry
            .scheduled_at
            .insert("db:resumed".to_string(), Instant::now() - grace);

        assert_eq!(registry.due_for_drop(grace), vec!["db:resumed".to_string()]);
    }

    /// Droppers started together — one per deleted database — take turns
    /// instead of holding the column-family lock back to back.
    #[test]
    fn background_drops_never_overlap() {
        use rust_rocksdb::Options;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let names: Vec<String> = (0..8).map(|i| format!("gate_test:c{i}")).collect();
        let mut families = vec![META_CF.to_string()];
        families.extend(names.iter().cloned());
        let db = Arc::new(DB::open_cf(&opts, dir.path(), &families).unwrap());
        let registry = Arc::new(PendingCfDrops::default());

        let threads: Vec<_> = names
            .chunks(2)
            .map(|pair| {
                let (db, registry, pair) = (Arc::clone(&db), Arc::clone(&registry), pair.to_vec());
                std::thread::spawn(move || {
                    for cf in &pair {
                        let (result, keep_going) = registry.drop_in_background(&db, cf);
                        result.unwrap();
                        assert!(keep_going);
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }

        assert!(names.iter().all(|cf| db.cf_handle(cf).is_none()));
        assert_eq!(MOST_DROPS_IN_FLIGHT.load(Ordering::SeqCst), 1);
    }

    /// A signalled shutdown cuts a nap short instead of waiting it out, so a
    /// process exit never blocks for a grace period.
    #[test]
    fn nap_returns_early_once_shutdown_is_signalled() {
        let registry = Arc::new(PendingCfDrops::default());
        registry.shutting_down.store(true, Ordering::Relaxed);

        let start = Instant::now();
        assert!(!registry.nap(Duration::from_secs(30)));
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
