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

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use rust_rocksdb::WriteBatch;

use super::engine::META_CF;
use super::RocksDb as DB;
use crate::error::{DbError, DbResult};

/// `_meta` key prefix for persisted drop markers.
const MARKER_PREFIX: &str = "pending_drop:";

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
        }
        db.write(&batch).map_err(|e| {
            DbError::InternalError(format!("Failed to schedule collection drops: {}", e))
        })?;

        for cf in cfs {
            self.states.insert(cf.clone(), DropState::Pending);
        }
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

        for cf in &cfs {
            self.states.insert(cf.clone(), DropState::Pending);
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

    /// Drop the scheduled CFs on a background thread. Each successful drop
    /// removes its persisted marker; failed drops stay marked so they are
    /// retried on the next startup.
    pub fn spawn_dropper(db: Arc<DB>, registry: Arc<Self>, cfs: Vec<String>) {
        if cfs.is_empty() {
            return;
        }
        std::thread::spawn(move || {
            let start = Instant::now();
            let total = cfs.len();
            let mut dropped = 0usize;
            for (i, cf) in cfs.iter().enumerate() {
                // Breathe between drops: each drop_cf holds the DB mutex for
                // its full OPTIONS rewrite, and back-to-back drops starve
                // concurrent foreground CF ops (an immediate recreate of the
                // same database would otherwise wait for the whole queue).
                if i > 0 {
                    std::thread::sleep(Duration::from_millis(25));
                }
                if !registry.begin_drop(cf) {
                    continue; // claimed by a concurrent recreate
                }
                if db.cf_handle(cf).is_some() {
                    if let Err(e) = db.drop_cf(cf) {
                        tracing::warn!("Background drop of column family '{}' failed: {}", cf, e);
                        registry.release_claim(cf);
                        continue;
                    }
                    dropped += 1;
                }
                registry.complete(&db, cf);
            }
            tracing::info!(
                "Background-dropped {}/{} column families in {:.2?}",
                dropped,
                total,
                start.elapsed()
            );
        });
    }
}
