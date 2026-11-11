//! Transaction commits written in groups by one thread, so that no
//! connection's thread waits for an fsync.
//!
//! A durable commit used to hold a thread from its write to the end of its
//! group's fsync: the driver handed its async worker to another thread
//! (`block_in_place`), the engine put the writer to sleep, and both were
//! woken afterwards. With 32 committers on 8 CPUs, about a third of a
//! transfer's CPU was scheduling. Here a connection prepares its commit
//! (locks, checks, the batch) and hands it over; the committer writes
//! every commit waiting at that moment as one batch with one fsync,
//! finishes each (generations, events, locks released) and tells its
//! connection, which waited as a future.
//!
//! One thread does it all. A second one, finishing each group while the
//! first wrote the next, was tried: no faster over five round trips, and
//! 14% slower in one request at 32 clients (a ninth busy thread on eight
//! CPUs).
//!
//! What a prepared commit holds is released by the committer, never by a
//! task that would need a free worker to be polled: a worker blocked on a
//! key a suspended task held could otherwise wait for ever.

use std::sync::mpsc;
use std::sync::{Arc, Weak};

use super::collection::txn::StagedTransaction;
use super::collection::Collection;
use super::keyspace::HeldCommitGate;
use super::kv::{Kv as DB, KvBatch, KvWriteOptions};
use crate::error::{DbError, DbResult};

/// A commit ready to be written: its batch, and what it holds until the
/// write has landed.
#[derive(Default)]
pub(crate) struct PreparedCommit {
    pub(crate) batch: KvBatch,
    /// Each written collection with what to do once the batch is in.
    pub(crate) staged: Vec<(Collection, StagedTransaction)>,
    /// Key and unique-value locks, released before the collections are
    /// finished.
    pub(crate) guards: Vec<super::collection::locks::WriteGuard>,
    /// Commit gates, released last.
    pub(crate) gates: Vec<HeldCommitGate>,
}

impl PreparedCommit {
    /// The batch is written: release the locks, move the generations and
    /// publish the changes, then open the gates.
    pub(crate) fn finish(self) {
        drop(self.guards);
        for (collection, plan) in self.staged {
            collection.finish_transaction_operations(plan);
        }
        drop(self.gates);
    }
}

struct Job {
    prepared: PreparedCommit,
    sync: bool,
    done: tokio::sync::oneshot::Sender<DbResult<()>>,
}

/// Commits gathered into one write at most.
const MAX_GROUP: usize = 256;

pub(crate) struct GroupCommitter {
    jobs: mpsc::Sender<Job>,
}

impl GroupCommitter {
    pub(crate) fn start(db: &Arc<DB>) -> Self {
        let (jobs, queue) = mpsc::channel();
        let db = Arc::downgrade(db);
        std::thread::Builder::new()
            .name("solidb-commit".to_string())
            .spawn(move || run(db, queue))
            .expect("the commit thread starts");
        GroupCommitter { jobs }
    }

    /// Write `prepared` with the commits waiting beside it, and finish it.
    pub(crate) async fn commit(&self, prepared: PreparedCommit, sync: bool) -> DbResult<()> {
        let (done, finished) = tokio::sync::oneshot::channel();
        let job = Job {
            prepared,
            sync,
            done,
        };
        if self.jobs.send(job).is_err() {
            return Err(DbError::InternalError(
                "the commit thread has stopped".to_string(),
            ));
        }
        finished.await.unwrap_or_else(|_| {
            Err(DbError::InternalError(
                "the commit thread dropped a commit".to_string(),
            ))
        })
    }
}

fn run(db: Weak<DB>, queue: mpsc::Receiver<Job>) {
    while let Ok(first) = queue.recv() {
        let mut group = vec![first];
        while group.len() < MAX_GROUP {
            match queue.try_recv() {
                Ok(job) => group.push(job),
                Err(_) => break,
            }
        }
        let Some(db) = db.upgrade() else {
            fail(group, "the database is closed");
            continue;
        };
        // One batch for as many as the engine merges; a batch it cannot
        // merge starts the next write.
        let mut batch = KvBatch::default();
        let mut sync = false;
        let mut merged: Vec<Job> = Vec::with_capacity(group.len());
        for mut job in group {
            if let Err(alone) = batch.append(std::mem::take(&mut job.prepared.batch)) {
                write(
                    &db,
                    std::mem::take(&mut batch),
                    sync,
                    std::mem::take(&mut merged),
                );
                batch = alone;
                sync = false;
            }
            sync |= job.sync;
            merged.push(job);
        }
        write(&db, batch, sync, merged);
    }
}

fn write(db: &DB, batch: KvBatch, sync: bool, jobs: Vec<Job>) {
    if jobs.is_empty() {
        return;
    }
    let mut opts = KvWriteOptions::default();
    opts.set_sync(sync);
    match db.write_opt(batch, &opts) {
        Ok(()) => {
            for job in jobs {
                job.prepared.finish();
                let _ = job.done.send(Ok(()));
            }
        }
        Err(e) => fail(jobs, &format!("Failed to commit transaction batch: {}", e)),
    }
}

/// Nothing was written: each commit lets go of what it held and its
/// connection is told why.
fn fail(jobs: Vec<Job>, why: &str) {
    for job in jobs {
        drop(job.prepared);
        let _ = job.done.send(Err(DbError::InternalError(why.to_string())));
    }
}
