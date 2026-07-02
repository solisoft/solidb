use super::{Operation, Transaction, TransactionId};
use crate::error::{DbError, DbResult};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Condvar, Mutex,
};

/// Write-Ahead Log entry types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WalEntry {
    /// Transaction begin
    Begin {
        tx_id: TransactionId,
        timestamp: u64,
    },
    /// Operation within a transaction
    Operation {
        tx_id: TransactionId,
        operation: Operation,
    },
    /// Transaction commit
    Commit {
        tx_id: TransactionId,
        timestamp: u64,
    },
    /// Transaction abort/rollback
    Abort {
        tx_id: TransactionId,
        timestamp: u64,
    },
    /// Checkpoint marker (can truncate log before this point)
    Checkpoint { timestamp: u64 },
}

impl WalEntry {
    /// Get the transaction ID for this entry (if applicable)
    pub fn tx_id(&self) -> Option<TransactionId> {
        match self {
            WalEntry::Begin { tx_id, .. } => Some(*tx_id),
            WalEntry::Operation { tx_id, .. } => Some(*tx_id),
            WalEntry::Commit { tx_id, .. } => Some(*tx_id),
            WalEntry::Abort { tx_id, .. } => Some(*tx_id),
            WalEntry::Checkpoint { .. } => None,
        }
    }
}

/// Write-Ahead Log writer with batching and group commit.
///
/// Commits used to serialize one-fsync-at-a-time behind the file mutex,
/// capping commit throughput at ~1/fsync-latency regardless of concurrency.
/// Entries are now tagged with a monotonically increasing sequence number;
/// one committer at a time becomes the sync leader and fsyncs *everything*
/// buffered so far, while concurrent committers wait on a condvar until the
/// leader's fsync covers their sequence — N concurrent commits share one
/// fsync instead of paying for N.
pub struct WalWriter {
    file: Arc<Mutex<File>>,
    path: PathBuf,
    buffer: Arc<Mutex<WalBuffer>>,
    batch_size: usize,
    pending_writes: Arc<AtomicUsize>,
    sync_state: Arc<Mutex<SyncState>>,
    sync_done: Arc<Condvar>,
}

/// Entries waiting for their fsync, tagged with the sequence of the most
/// recently buffered entry so waiters can tell when theirs became durable.
struct WalBuffer {
    entries: VecDeque<String>,
    last_assigned: u64,
}

struct SyncState {
    /// A leader is currently writing + fsyncing.
    syncing: bool,
    /// Highest sequence number known durable on disk.
    synced: u64,
}

impl WalWriter {
    /// Create a new WAL writer with batching disabled by default
    pub fn new<P: AsRef<Path>>(path: P) -> DbResult<Self> {
        Self::with_batch_size(path, 1)
    }

    /// Create a new WAL writer with configurable batch size
    pub fn with_batch_size<P: AsRef<Path>>(path: P, batch_size: usize) -> DbResult<Self> {
        let path = path.as_ref().to_path_buf();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| DbError::InternalError(format!("Failed to open WAL: {}", e)))?;

        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            path,
            buffer: Arc::new(Mutex::new(WalBuffer {
                entries: VecDeque::new(),
                last_assigned: 0,
            })),
            batch_size: batch_size.max(1),
            pending_writes: Arc::new(AtomicUsize::new(0)),
            sync_state: Arc::new(Mutex::new(SyncState {
                syncing: false,
                synced: 0,
            })),
            sync_done: Arc::new(Condvar::new()),
        })
    }

    /// Buffer a serialized entry and return its sequence number.
    fn buffer_entry(&self, entry: &WalEntry) -> DbResult<u64> {
        let json = serde_json::to_string(entry)
            .map_err(|e| DbError::InternalError(format!("Failed to serialize WAL entry: {}", e)))?;
        let mut buffer = self.buffer.lock().unwrap();
        buffer.entries.push_back(json);
        buffer.last_assigned += 1;
        Ok(buffer.last_assigned)
    }

    /// Write a WAL entry (batched)
    pub fn write(&self, entry: &WalEntry) -> DbResult<()> {
        self.buffer_entry(entry)?;
        let pending = self.pending_writes.fetch_add(1, Ordering::SeqCst) + 1;

        // If batch is full, flush
        if pending >= self.batch_size {
            self.flush()?;
        }

        Ok(())
    }

    /// Flush buffered writes to disk
    pub fn flush(&self) -> DbResult<()> {
        let target = {
            let buffer = self.buffer.lock().unwrap();
            if buffer.entries.is_empty() {
                return Ok(());
            }
            buffer.last_assigned
        };
        self.sync_up_to(target)
    }

    /// Block until every entry up to `target` is durable. One caller at a
    /// time becomes the leader and fsyncs the whole buffer; the rest wait on
    /// the condvar and return as soon as a leader's fsync covers them.
    fn sync_up_to(&self, target: u64) -> DbResult<()> {
        let mut state = self.sync_state.lock().unwrap();
        loop {
            if state.synced >= target {
                return Ok(());
            }
            if !state.syncing {
                state.syncing = true;
                drop(state);

                let result = self.write_and_sync();

                let mut state = self.sync_state.lock().unwrap();
                state.syncing = false;
                if let Ok(up_to) = &result {
                    state.synced = state.synced.max(*up_to);
                }
                self.sync_done.notify_all();
                // Our entry was buffered before we became leader, so a
                // successful sync always covers `target`.
                return result.map(|_| ());
            }
            // Follower: wait for the in-flight sync, then re-check.
            state = self.sync_done.wait(state).unwrap();
        }
    }

    /// Drain the buffer, write everything, fsync once. Returns the sequence
    /// of the last entry made durable.
    fn write_and_sync(&self) -> DbResult<u64> {
        let (entries, up_to) = {
            let mut buffer = self.buffer.lock().unwrap();
            let entries: Vec<String> = buffer.entries.drain(..).collect();
            (entries, buffer.last_assigned)
        };

        if !entries.is_empty() {
            let mut file = self.file.lock().unwrap();
            for json in &entries {
                writeln!(file, "{}", json).map_err(|e| {
                    DbError::InternalError(format!("Failed to write WAL entry: {}", e))
                })?;
            }
            // Ensure durability - flush to disk
            file.sync_all()
                .map_err(|e| DbError::InternalError(format!("Failed to sync WAL: {}", e)))?;
        }

        self.pending_writes.store(0, Ordering::SeqCst);
        Ok(up_to)
    }

    /// Force flush without batching (for critical operations like commit)
    pub fn force_sync(&self) -> DbResult<()> {
        self.flush()
    }

    /// Write transaction begin
    pub fn write_begin(&self, tx_id: TransactionId) -> DbResult<()> {
        self.write(&WalEntry::Begin {
            tx_id,
            timestamp: tx_id.as_u64(),
        })
    }

    /// Write transaction operation
    pub fn write_operation(&self, tx_id: TransactionId, operation: Operation) -> DbResult<()> {
        self.write(&WalEntry::Operation { tx_id, operation })
    }

    /// Write transaction commit (always forces sync). Concurrent commits
    /// group: whoever becomes the sync leader fsyncs every buffered entry,
    /// and the rest return as soon as that fsync covers theirs.
    pub fn write_commit(&self, tx_id: TransactionId) -> DbResult<()> {
        let entry = WalEntry::Commit {
            tx_id,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        };

        // For commit, we always want to ensure durability
        let seq = self.buffer_entry(&entry)?;
        self.sync_up_to(seq)
    }

    /// Write transaction abort
    pub fn write_abort(&self, tx_id: TransactionId) -> DbResult<()> {
        self.write(&WalEntry::Abort {
            tx_id,
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        })
    }

    /// Write checkpoint marker (forces sync)
    pub fn write_checkpoint(&self) -> DbResult<()> {
        let entry = WalEntry::Checkpoint {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() as u64,
        };

        let seq = self.buffer_entry(&entry)?;
        self.sync_up_to(seq)
    }

    /// Get WAL file path
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Set batch size (0 disables batching)
    pub fn set_batch_size(&self, size: usize) {
        // Can't change batch size at runtime safely, this is just for config
        let _ = size;
    }
}

/// WAL reader for recovery
pub struct WalReader {
    path: PathBuf,
}

impl WalReader {
    /// Create a new WAL reader
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    /// Read all WAL entries
    pub fn read_all(&self) -> DbResult<Vec<WalEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)
            .map_err(|e| DbError::InternalError(format!("Failed to open WAL: {}", e)))?;

        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for (line_num, line) in reader.lines().enumerate() {
            let line = line.map_err(|e| {
                DbError::InternalError(format!("Failed to read WAL line {}: {}", line_num, e))
            })?;

            if line.trim().is_empty() {
                continue;
            }

            let entry: WalEntry = serde_json::from_str(&line).map_err(|e| {
                DbError::InternalError(format!(
                    "Failed to parse WAL entry at line {}: {}",
                    line_num, e
                ))
            })?;

            entries.push(entry);
        }

        Ok(entries)
    }

    /// Replay WAL and return committed transactions
    pub fn replay(&self) -> DbResult<Vec<Transaction>> {
        let entries = self.read_all()?;
        let mut transactions = std::collections::HashMap::new();
        let mut committed = Vec::new();

        for entry in entries {
            match entry {
                WalEntry::Begin { tx_id, timestamp } => {
                    let mut tx = Transaction::new(super::IsolationLevel::ReadCommitted);
                    tx.id = tx_id;
                    tx.read_timestamp = timestamp;
                    transactions.insert(tx_id, tx);
                }
                WalEntry::Operation { tx_id, operation } => {
                    if let Some(tx) = transactions.get_mut(&tx_id) {
                        tx.add_operation(operation);
                    }
                }
                WalEntry::Commit { tx_id, .. } => {
                    if let Some(mut tx) = transactions.remove(&tx_id) {
                        tx.commit();
                        committed.push(tx);
                    }
                }
                WalEntry::Abort { tx_id, .. } => {
                    transactions.remove(&tx_id);
                }
                WalEntry::Checkpoint { .. } => {
                    // Checkpoint marker - can be used for truncation
                }
            }
        }

        Ok(committed)
    }
}

/// Truncate WAL file up to the last checkpoint
pub fn truncate_wal<P: AsRef<Path>>(path: P) -> DbResult<()> {
    let reader = WalReader::new(&path);
    let entries = reader.read_all()?;

    // Find last checkpoint
    let last_checkpoint_idx = entries
        .iter()
        .enumerate()
        .rev()
        .find(|(_, e)| matches!(e, WalEntry::Checkpoint { .. }))
        .map(|(idx, _)| idx);

    if let Some(checkpoint_idx) = last_checkpoint_idx {
        // Keep only entries after last checkpoint
        let entries_to_keep = &entries[checkpoint_idx + 1..];

        // Rewrite WAL file
        let temp_path = path.as_ref().with_extension("wal.tmp");
        let mut temp_file = File::create(&temp_path)
            .map_err(|e| DbError::InternalError(format!("Failed to create temp WAL: {}", e)))?;

        for entry in entries_to_keep {
            let json = serde_json::to_string(entry)
                .map_err(|e| DbError::InternalError(format!("Failed to serialize entry: {}", e)))?;
            writeln!(temp_file, "{}", json)
                .map_err(|e| DbError::InternalError(format!("Failed to write temp WAL: {}", e)))?;
        }

        temp_file
            .sync_all()
            .map_err(|e| DbError::InternalError(format!("Failed to sync temp WAL: {}", e)))?;

        // Atomic rename
        std::fs::rename(&temp_path, path.as_ref())
            .map_err(|e| DbError::InternalError(format!("Failed to rename WAL: {}", e)))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::TransactionState;
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_wal_write_and_read() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let writer = WalWriter::new(&wal_path).unwrap();
        let tx_id = TransactionId::new();

        writer.write_begin(tx_id).unwrap();
        writer
            .write_operation(
                tx_id,
                Operation::Insert {
                    database: "_system".to_string(),
                    collection: "users".to_string(),
                    key: "user1".to_string(),
                    data: serde_json::json!({"name": "Alice"}),
                },
            )
            .unwrap();
        writer.write_commit(tx_id).unwrap();

        let reader = WalReader::new(&wal_path);
        let entries = reader.read_all().unwrap();

        assert_eq!(entries.len(), 3);
        assert!(matches!(entries[0], WalEntry::Begin { .. }));
        assert!(matches!(entries[1], WalEntry::Operation { .. }));
        assert!(matches!(entries[2], WalEntry::Commit { .. }));
    }

    #[test]
    fn test_wal_replay() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let writer = WalWriter::new(&wal_path).unwrap();
        let tx_id = TransactionId::new();

        writer.write_begin(tx_id).unwrap();
        writer
            .write_operation(
                tx_id,
                Operation::Insert {
                    database: "_system".to_string(),
                    collection: "users".to_string(),
                    key: "user1".to_string(),
                    data: serde_json::json!({"name": "Alice"}),
                },
            )
            .unwrap();
        writer.write_commit(tx_id).unwrap();

        let reader = WalReader::new(&wal_path);
        let committed = reader.replay().unwrap();

        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].id, tx_id);
        assert_eq!(committed[0].state, TransactionState::Committed);
        assert_eq!(committed[0].operations.len(), 1);
    }

    #[test]
    fn test_wal_truncate() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let writer = WalWriter::new(&wal_path).unwrap();

        // Write some entries
        let tx1 = TransactionId::new();
        writer.write_begin(tx1).unwrap();
        writer.write_commit(tx1).unwrap();

        // Checkpoint
        writer.write_checkpoint().unwrap();

        // More entries
        let tx2 = TransactionId::new();
        writer.write_begin(tx2).unwrap();
        writer.write_commit(tx2).unwrap();

        // Truncate
        truncate_wal(&wal_path).unwrap();

        // Should only have entries after checkpoint
        let reader = WalReader::new(&wal_path);
        let entries = reader.read_all().unwrap();

        // Should have begin and commit for tx2 only
        assert_eq!(entries.len(), 2);
    }

    /// Group commit: many threads committing concurrently must all land
    /// durably, exactly once each, and replay cleanly.
    #[test]
    fn test_wal_concurrent_group_commit() {
        let dir = tempdir().unwrap();
        let wal_path = dir.path().join("test.wal");

        let writer = Arc::new(WalWriter::new(&wal_path).unwrap());
        const THREADS: usize = 8;
        const COMMITS_PER_THREAD: usize = 50;

        let handles: Vec<_> = (0..THREADS)
            .map(|_| {
                let writer = Arc::clone(&writer);
                std::thread::spawn(move || {
                    for _ in 0..COMMITS_PER_THREAD {
                        let tx_id = TransactionId::new();
                        writer.write_begin(tx_id).unwrap();
                        writer.write_commit(tx_id).unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }

        let reader = WalReader::new(&wal_path);
        let entries = reader.read_all().unwrap();
        let commits = entries
            .iter()
            .filter(|e| matches!(e, WalEntry::Commit { .. }))
            .count();
        assert_eq!(commits, THREADS * COMMITS_PER_THREAD);

        let committed = reader.replay().unwrap();
        assert_eq!(committed.len(), THREADS * COMMITS_PER_THREAD);
    }
}
