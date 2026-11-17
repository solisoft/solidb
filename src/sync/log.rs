//! Sync log for recording mutations that need to be replicated
//!
//! This module provides a local write-ahead log for mutations that
//! need to be synchronized to other nodes in the cluster.

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use crate::storage::kv::{Direction, IterMode, KvBatch};
use crate::storage::RocksDb as DB;
use serde::{Deserialize, Serialize};

use super::protocol::{Operation, SyncEntry};
use crate::cluster::HybridLogicalClock;

const LOG_PREFIX: &[u8] = b"sync_log:";

/// First byte of an entry stored as MessagePack. JSON entries (every entry
/// written before 2.0.2) start with `{`, so the two are told apart by it.
const MSGPACK_ENTRY: u8 = 0xB1;

/// On-disk form of a log entry.
///
/// Entries used to be JSON, where `data` (the document, already JSON) became
/// an array of decimal byte values — about four times its size — written to a
/// second RocksDB on every document write, then flushed and compacted. On a
/// primary-key update that was a large part of the ~35 µs the log costs.
fn encode_entry(entry: &LogEntry) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 + entry.data.as_ref().map_or(0, Vec::len));
    out.push(MSGPACK_ENTRY);
    rmp_serde::encode::write_named(&mut out, entry).expect("encoding a LogEntry cannot fail");
    out
}

/// Read an entry in either on-disk form.
fn decode_entry(bytes: &[u8]) -> Option<LogEntry> {
    match bytes.split_first() {
        Some((&MSGPACK_ENTRY, rest)) => rmp_serde::from_slice(rest).ok(),
        _ => serde_json::from_slice(bytes).ok(),
    }
}
const SEQ_KEY: &[u8] = b"sync_log:_sequence";
/// Present once the log has been activated (see [`SyncLog::defer_until_needed`]).
/// Outside `LOG_PREFIX`, so no entry scan or prune ever sees it.
const ACTIVE_KEY: &[u8] = b"_meta:active";

/// How long the log keeps its entries whatever its peers have received,
/// in seconds (`SOLIDB_SYNC_LOG_KEEP_SECS`; 0 or unset: not kept). With
/// it the log is written from the start, even on a node with no peer:
/// it is the history a point-in-time restore replays.
pub fn keep_secs() -> u64 {
    std::env::var("SOLIDB_SYNC_LOG_KEEP_SECS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// What [`SyncLog::archive_after`] wrote.
#[derive(Debug, Default, Clone, Serialize)]
pub struct ArchiveSummary {
    pub entries: u64,
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub first_timestamp: u64,
    pub last_timestamp: u64,
}

/// Entry for the sync log (similar to old LogEntry for compatibility)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub sequence: u64,
    pub node_id: String,
    pub database: String,
    pub collection: String,
    pub operation: Operation,
    pub key: String,
    #[serde(with = "serde_bytes")]
    pub data: Option<Vec<u8>>,
    pub timestamp: u64,
    #[serde(default)]
    pub origin_sequence: Option<u64>,
}

impl LogEntry {
    /// Build an entry stamped "now" with the local node filled in by `append`.
    pub fn new_op(
        database: impl Into<String>,
        collection: impl Into<String>,
        operation: Operation,
        key: impl Into<String>,
        data: Option<Vec<u8>>,
    ) -> Self {
        Self {
            sequence: 0,
            node_id: String::new(),
            database: database.into(),
            collection: collection.into(),
            operation,
            key: key.into(),
            data,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        }
    }

    /// Convert to SyncEntry for replication
    pub fn to_sync_entry(&self, hlc: &HybridLogicalClock) -> SyncEntry {
        SyncEntry {
            sequence: self.sequence,
            origin_node: self.node_id.clone(),
            origin_sequence: self.origin_sequence.unwrap_or(self.sequence),
            hlc_ts: hlc.physical_time,
            hlc_count: hlc.logical_counter,
            database: self.database.clone(),
            collection: self.collection.clone(),
            operation: self.operation,
            document_key: self.key.clone(),
            document_data: self.data.clone(),
            shard_id: None,
            // New offline sync fields
            version_vector: None,
            parent_vectors: Vec::new(),
            is_delta: false,
            delta_data: None,
            session_id: None,
            device_id: Some(self.node_id.clone()),
        }
    }
}

/// Persistent sync log backed by RocksDB
pub struct SyncLog {
    db: Arc<DB>,
    node_id: String,
    sequence: Arc<RwLock<u64>>,
    cache: Arc<RwLock<VecDeque<LogEntry>>>,
    max_cache_size: usize,
    /// When true, append/append_batch are no-ops. Used in standalone mode
    /// to avoid the disk overhead of a replication log no peer will ever read.
    disabled: bool,
    /// Whether appends are written. Shared by every clone. Starts true; a
    /// node with no peer defers it until something needs the log.
    active: Arc<std::sync::atomic::AtomicBool>,
    /// Set once the activation marker is known to be on disk.
    marked: Arc<std::sync::atomic::AtomicBool>,
}

impl SyncLog {
    /// Create a new sync log (writes enabled).
    pub fn new(node_id: String, data_dir: &str, max_cache_size: usize) -> Result<Self, String> {
        Self::new_with_options(node_id, data_dir, max_cache_size, false)
    }

    /// Create a new sync log with explicit options.
    /// `disabled = true` makes append/append_batch no-ops (still readable).
    pub fn new_with_options(
        node_id: String,
        data_dir: &str,
        max_cache_size: usize,
        disabled: bool,
    ) -> Result<Self, String> {
        let log_path = format!("{}/sync_log", data_dir);

        let db = crate::storage::kv::open_sync_log(std::path::Path::new(&log_path))
            .map_err(|e| e.to_string())?;
        let db = Arc::new(db);

        // Load current sequence
        let sequence = match db.get(SEQ_KEY) {
            Ok(Some(bytes)) => {
                let arr: [u8; 8] = bytes.as_slice().try_into().unwrap_or([0u8; 8]);
                u64::from_be_bytes(arr)
            }
            _ => 0,
        };

        let log = Self {
            db,
            node_id,
            sequence: Arc::new(RwLock::new(sequence)),
            cache: Arc::new(RwLock::new(VecDeque::with_capacity(max_cache_size))),
            max_cache_size,
            disabled,
            active: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            marked: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        log.load_cache();
        Ok(log)
    }

    /// Load recent entries into cache
    fn load_cache(&self) {
        let mut cache = self.cache.write().unwrap();
        cache.clear();

        let iter = self
            .db
            .iterator(IterMode::From(LOG_PREFIX, Direction::Forward));
        let mut count = 0;

        for (key, value) in iter.flatten() {
            if !key.starts_with(LOG_PREFIX) || key.as_ref() == SEQ_KEY {
                continue;
            }

            if let Some(entry) = decode_entry(&value) {
                cache.push_back(entry);
                count += 1;

                if count >= self.max_cache_size {
                    break;
                }
            }
        }
    }

    /// Append an entry to the log
    pub fn append(&self, mut entry: LogEntry) -> u64 {
        if !self.writes() {
            return self.current_sequence();
        }

        let mut seq = self.sequence.write().unwrap();
        *seq += 1;
        entry.sequence = *seq;

        if entry.node_id.is_empty() {
            entry.node_id = self.node_id.clone();
        }

        // Use a single KvBatch so the entry and the bumped sequence go
        // through one `db.write` (one fsync) instead of two independent
        // `put` calls (two fsyncs). This roughly halves the per-write
        // disk cost on hot insert/update paths.
        let key = format!("sync_log:{:020}", *seq);
        let value = encode_entry(&entry);

        let mut batch = KvBatch::default();
        let default_cf = self.db.default_cf();
        batch.put_cf(&default_cf, key.as_bytes(), &value);
        batch.put_cf(&default_cf, SEQ_KEY, seq.to_be_bytes());

        if let Err(e) = self.db.write(batch) {
            tracing::error!("SyncLog: Failed to write entry {}: {}", *seq, e);
        }

        // Update cache
        let mut cache = self.cache.write().unwrap();
        cache.push_back(entry);
        if cache.len() > self.max_cache_size {
            cache.pop_front();
        }

        *seq
    }

    /// Log a document-level operation (insert/update/delete) for replication.
    pub fn log_document_op(
        &self,
        database: &str,
        collection: &str,
        operation: Operation,
        key: &str,
        data: Option<Vec<u8>>,
    ) -> u64 {
        self.append(LogEntry::new_op(database, collection, operation, key, data))
    }

    /// Log a collection truncate so replicas clear the collection too.
    /// Every direct caller of `Collection::truncate()` outside the HTTP
    /// handler (binary driver, materialized-view refresh) must use this,
    /// otherwise replicas silently keep the old documents.
    pub fn log_truncate(&self, database: &str, collection: &str) -> u64 {
        self.append(LogEntry::new_op(
            database,
            collection,
            Operation::TruncateCollection,
            "",
            None,
        ))
    }

    /// Append multiple entries atomically
    pub fn append_batch(&self, mut entries: Vec<LogEntry>) -> u64 {
        if !self.writes() || entries.is_empty() {
            return self.current_sequence();
        }

        let mut seq = self.sequence.write().unwrap();
        let mut batch = KvBatch::default();
        let default_cf = self.db.default_cf();

        for entry in &mut entries {
            *seq += 1;
            entry.sequence = *seq;

            if entry.node_id.is_empty() {
                entry.node_id = self.node_id.clone();
            }

            let key = format!("sync_log:{:020}", *seq);
            let value = encode_entry(entry);
            batch.put_cf(&default_cf, key.as_bytes(), &value);
        }

        batch.put_cf(&default_cf, SEQ_KEY, seq.to_be_bytes());
        if let Err(e) = self.db.write(batch) {
            tracing::error!("SyncLog: Failed to write batch ending at {}: {}", *seq, e);
        }

        // Update cache
        let mut cache = self.cache.write().unwrap();
        for entry in entries {
            cache.push_back(entry);
            if cache.len() > self.max_cache_size {
                cache.pop_front();
            }
        }

        *seq
    }

    fn writes(&self) -> bool {
        !self.disabled && self.active.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Stop writing entries until [`Self::activate`] is called, unless this
    /// log was activated before (the marker survives restarts).
    ///
    /// For a node with no configured peer. Every document write appends an
    /// entry here — a second RocksDB, a second WAL write, under a lock all
    /// writers share — and on a node nobody replicates from, nothing reads
    /// it: a primary-key update cost ~70 µs of CPU with the log and ~45
    /// without. The readers are peers (sync connections) and offline-sync
    /// devices, and both activate the log before they read it.
    pub fn defer_until_needed(&self) {
        if self.disabled {
            return;
        }
        let activated_before = matches!(self.db.get(ACTIVE_KEY), Ok(Some(_)));
        if !activated_before {
            self.active
                .store(false, std::sync::atomic::Ordering::Release);
            tracing::info!(
                "Sync log: no peer configured; entries are written once a peer or an \
                 offline-sync device connects"
            );
        }
    }

    /// Start writing entries, for good: the marker keeps the log active
    /// across restarts, as it always was before 2.0.3.
    ///
    /// Must run before anything is served from the log. A write that found
    /// the log inactive had already committed its data (entries are appended
    /// after the write), so a full sync or snapshot taken after this call
    /// includes it.
    pub fn activate(&self, reason: &str) {
        if self.disabled {
            return;
        }
        // Persisted even when already active: a node started with `--peer`
        // writes from the start, and must keep doing so if it is later
        // restarted without the flag (as the seed of the cluster it formed).
        if !self.marked.swap(true, std::sync::atomic::Ordering::AcqRel)
            && !matches!(self.db.get(ACTIVE_KEY), Ok(Some(_)))
        {
            if let Err(e) = self.db.put(ACTIVE_KEY, b"1") {
                tracing::error!("Sync log: failed to persist activation: {}", e);
            }
        }
        if !self.active.swap(true, std::sync::atomic::Ordering::AcqRel) {
            tracing::info!("Sync log activated ({reason})");
        }
    }

    /// Whether entries are currently written.
    pub fn is_active(&self) -> bool {
        self.writes()
    }

    /// Returns true if append/append_batch are no-ops.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Returns the sequence of the oldest retained entry, or None if the log is empty.
    pub fn oldest_sequence(&self) -> Option<u64> {
        let iter = self
            .db
            .iterator(IterMode::From(LOG_PREFIX, Direction::Forward));
        for (key, _) in iter.flatten() {
            if !key.starts_with(LOG_PREFIX) || key.as_ref() == SEQ_KEY {
                continue;
            }
            // Key format: "sync_log:{:020}" — sequence starts at byte 9
            if key.len() >= LOG_PREFIX.len() + 20 {
                let seq_bytes = &key[LOG_PREFIX.len()..LOG_PREFIX.len() + 20];
                if let Ok(s) = std::str::from_utf8(seq_bytes) {
                    if let Ok(seq) = s.parse::<u64>() {
                        return Some(seq);
                    }
                }
            }
        }
        None
    }

    /// Approximate count of entries currently retained on disk.
    /// Walks the entire keyspace, so callers should not use this in hot paths.
    pub fn entry_count(&self) -> u64 {
        let iter = self
            .db
            .iterator(IterMode::From(LOG_PREFIX, Direction::Forward));
        let mut count = 0u64;
        for (key, _) in iter.flatten() {
            if !key.starts_with(LOG_PREFIX) || key.as_ref() == SEQ_KEY {
                continue;
            }
            count += 1;
        }
        count
    }

    /// Delete every log entry with `sequence < before_sequence`.
    ///
    /// Returns the number of entries removed.
    ///
    /// Safety: the caller is responsible for choosing `before_sequence` such
    /// that every peer has already pulled entries up to that point.
    pub fn prune_before(&self, before_sequence: u64) -> Result<u64, String> {
        if before_sequence == 0 {
            return Ok(0);
        }

        let start_key = format!("sync_log:{:020}", 0u64);
        let end_key = format!("sync_log:{:020}", before_sequence);

        // Walk the to-be-pruned range, building a KvBatch of explicit
        // deletes. This is portable across rocksdb wrappers and gives an
        // exact removed count for reporting.
        let mut batch = KvBatch::default();
        let default_cf = self.db.default_cf();
        let mut removed = 0u64;
        let iter = self
            .db
            .iterator(IterMode::From(start_key.as_bytes(), Direction::Forward));
        for (key, _) in iter.flatten() {
            if !key.starts_with(LOG_PREFIX) || key.as_ref() == SEQ_KEY {
                continue;
            }
            if key.as_ref() >= end_key.as_bytes() {
                break;
            }
            batch.delete_cf(&default_cf, &key);
            removed += 1;
        }

        if removed == 0 {
            return Ok(0);
        }

        self.db.write(batch).map_err(|e| e.to_string())?;

        // Compact the deleted range so SST files actually shrink on disk.
        self.db
            .compact_range(Some(start_key.as_bytes()), Some(end_key.as_bytes()));

        // Drop matching entries from in-memory cache.
        {
            let mut cache = self.cache.write().unwrap();
            cache.retain(|e| e.sequence >= before_sequence);
        }

        tracing::info!(
            "SyncLog::prune_before({}): removed {} entries",
            before_sequence,
            removed
        );

        Ok(removed)
    }

    /// Get entries after a sequence number
    pub fn get_entries_after(&self, after_sequence: u64, limit: usize) -> Vec<LogEntry> {
        // Try cache first
        let cache = self.cache.read().unwrap();
        let cached: Vec<_> = cache
            .iter()
            .filter(|e| e.sequence > after_sequence)
            .take(limit)
            .cloned()
            .collect();

        // Critical: Only return from cache if we have the IMMEDIATE NEXT entry.
        if let Some(first) = cached.first() {
            if first.sequence == after_sequence + 1 {
                return cached;
            }
        }

        // Fall back to disk
        let start_key = format!("sync_log:{:020}", after_sequence + 1);
        let iter = self
            .db
            .iterator(IterMode::From(start_key.as_bytes(), Direction::Forward));

        let mut entries = Vec::new();
        for (key, value) in iter.flatten() {
            if !key.starts_with(LOG_PREFIX) || key.as_ref() == SEQ_KEY {
                continue;
            }

            if let Some(entry) = decode_entry(&value) {
                if entry.sequence > after_sequence {
                    entries.push(entry);
                    if entries.len() >= limit {
                        break;
                    }
                }
            }
        }

        entries
    }

    /// The first sequence whose entry is stamped at or after `at_ms`
    /// (milliseconds since the epoch): every entry below it is older.
    /// `current_sequence() + 1` when no entry is that recent. Entries are
    /// stamped in the order they are appended, so this is a bisection; a
    /// clock that stepped back can only make it keep more.
    pub fn first_sequence_since(&self, at_ms: u64) -> u64 {
        let end = self.current_sequence() + 1;
        let (mut lo, mut hi) = (self.oldest_sequence().unwrap_or(end).min(end), end);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.get_entries_after(mid - 1, 1).into_iter().next() {
                Some(entry) if entry.timestamp < at_ms => lo = entry.sequence + 1,
                _ => hi = mid,
            }
        }
        lo
    }

    /// Write every entry after `after_sequence` to a new file at `path`,
    /// oldest first: what a point-in-time restore replays over a
    /// checkpoint. Each record is its length (four bytes, little endian)
    /// then the entry as the log stores it. Returns how many entries, and
    /// the first and last of them.
    pub fn archive_after(
        &self,
        after_sequence: u64,
        path: &std::path::Path,
    ) -> std::io::Result<ArchiveSummary> {
        use std::io::Write;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut out = std::io::BufWriter::new(file);
        let mut summary = ArchiveSummary::default();
        let mut after = after_sequence;
        loop {
            let entries = self.get_entries_after(after, 1024);
            let Some(last) = entries.last() else {
                break;
            };
            after = last.sequence;
            for entry in &entries {
                let bytes = encode_entry(entry);
                out.write_all(&(bytes.len() as u32).to_le_bytes())?;
                out.write_all(&bytes)?;
                if summary.entries == 0 {
                    summary.first_sequence = entry.sequence;
                    summary.first_timestamp = entry.timestamp;
                }
                summary.entries += 1;
                summary.last_sequence = entry.sequence;
                summary.last_timestamp = entry.timestamp;
            }
        }
        out.flush()?;
        out.get_ref().sync_all()?;
        Ok(summary)
    }

    /// The entries of an archive written by [`Self::archive_after`], in
    /// order. A file cut short (a copy interrupted) is an error, not a
    /// shorter history.
    pub fn read_archive(path: &std::path::Path) -> std::io::Result<Vec<LogEntry>> {
        let bytes = std::fs::read(path)?;
        let cut =
            || std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "log archive cut short");
        let mut entries = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let length = bytes.get(at..at + 4).ok_or_else(cut)?;
            let length = u32::from_le_bytes([length[0], length[1], length[2], length[3]]) as usize;
            let record = bytes.get(at + 4..at + 4 + length).ok_or_else(cut)?;
            entries.push(decode_entry(record).ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "log archive: unreadable entry",
                )
            })?);
            at += 4 + length;
        }
        Ok(entries)
    }

    /// Get current sequence number
    pub fn current_sequence(&self) -> u64 {
        *self.sequence.read().unwrap()
    }

    /// Get node ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }
}

impl Clone for SyncLog {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            node_id: self.node_id.clone(),
            sequence: self.sequence.clone(),
            cache: self.cache.clone(),
            max_cache_size: self.max_cache_size,
            disabled: self.disabled,
            active: self.active.clone(),
            marked: self.marked.clone(),
        }
    }
}
// Operation is already exported via mod.rs

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_entry(seq: u64) -> LogEntry {
        LogEntry {
            sequence: seq,
            node_id: "test_node".to_string(),
            database: "test_db".to_string(),
            collection: "test_coll".to_string(),
            operation: Operation::Insert,
            key: format!("key_{}", seq),
            data: Some(b"test data".to_vec()),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        }
    }

    #[test]
    fn test_log_entry_creation() {
        let entry = create_test_entry(1);

        assert_eq!(entry.sequence, 1);
        assert_eq!(entry.node_id, "test_node");
        assert_eq!(entry.database, "test_db");
        assert_eq!(entry.collection, "test_coll");
        assert!(entry.data.is_some());
    }

    #[test]
    fn test_log_entry_to_sync_entry() {
        let entry = create_test_entry(5);
        let hlc = HybridLogicalClock::new(
            chrono::Utc::now().timestamp_millis() as u64,
            0,
            "node1".to_string(),
        );

        let sync_entry = entry.to_sync_entry(&hlc);

        assert_eq!(sync_entry.sequence, 5);
        assert_eq!(sync_entry.origin_node, "test_node");
        assert_eq!(sync_entry.database, "test_db");
        assert_eq!(sync_entry.document_key, "key_5");
    }

    #[test]
    fn test_log_entry_serialization() {
        let entry = create_test_entry(1);

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("test_node"));
        assert!(json.contains("test_db"));

        let deserialized: LogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry.sequence, deserialized.sequence);
        assert_eq!(entry.key, deserialized.key);
    }

    #[test]
    fn test_sync_log_new() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();

        assert_eq!(log.node_id(), "node1");
        assert_eq!(log.current_sequence(), 0);
    }

    #[test]
    fn test_sync_log_append() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();

        let entry = create_test_entry(0);
        let seq = log.append(entry);

        assert_eq!(seq, 1);
        assert_eq!(log.current_sequence(), 1);
    }

    #[test]
    fn test_sync_log_append_multiple() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();

        for _ in 0..5 {
            log.append(create_test_entry(0));
        }

        assert_eq!(log.current_sequence(), 5);
    }

    #[test]
    fn test_sync_log_append_batch() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();

        let entries = vec![
            create_test_entry(0),
            create_test_entry(0),
            create_test_entry(0),
        ];

        let seq = log.append_batch(entries);

        assert_eq!(seq, 3);
        assert_eq!(log.current_sequence(), 3);
    }

    #[test]
    fn test_sync_log_append_batch_empty() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();

        let seq = log.append_batch(vec![]);
        assert_eq!(seq, 0);
    }

    #[test]
    fn test_sync_log_get_entries_after() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();

        for _ in 0..5 {
            log.append(create_test_entry(0));
        }

        let entries = log.get_entries_after(2, 10);
        assert_eq!(entries.len(), 3); // Sequences 3, 4, 5
        assert_eq!(entries[0].sequence, 3);
    }

    #[test]
    fn test_sync_log_get_entries_with_limit() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();

        for _ in 0..10 {
            log.append(create_test_entry(0));
        }

        let entries = log.get_entries_after(0, 3);
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_sync_log_clone() {
        let tmp = TempDir::new().unwrap();
        let log1 = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();

        log1.append(create_test_entry(0));

        let log2 = log1.clone();

        // Both share the same state
        assert_eq!(log1.current_sequence(), log2.current_sequence());
    }

    #[test]
    fn test_sync_log_fills_node_id() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("my_node".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();

        let mut entry = create_test_entry(0);
        entry.node_id = String::new(); // Empty node_id

        log.append(entry);

        let entries = log.get_entries_after(0, 1);
        assert_eq!(entries[0].node_id, "my_node");
    }

    #[test]
    fn test_prune_before_removes_older_entries() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();

        for _ in 0..10 {
            log.append(create_test_entry(0));
        }
        assert_eq!(log.current_sequence(), 10);
        assert_eq!(log.entry_count(), 10);
        assert_eq!(log.oldest_sequence(), Some(1));

        // Prune everything strictly before sequence 6 (keeps 6..=10).
        let removed = log.prune_before(6).unwrap();
        assert_eq!(removed, 5);
        assert_eq!(log.entry_count(), 5);
        assert_eq!(log.oldest_sequence(), Some(6));
        // current_sequence is unchanged — append_only counter.
        assert_eq!(log.current_sequence(), 10);

        // Surviving entries are reachable via get_entries_after.
        let entries = log.get_entries_after(0, 100);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries.first().unwrap().sequence, 6);
        assert_eq!(entries.last().unwrap().sequence, 10);
    }

    #[test]
    fn test_prune_before_zero_is_noop() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();
        for _ in 0..3 {
            log.append(create_test_entry(0));
        }
        assert_eq!(log.prune_before(0).unwrap(), 0);
        assert_eq!(log.entry_count(), 3);
    }

    #[test]
    fn test_prune_before_when_empty() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();
        assert_eq!(log.prune_before(100).unwrap(), 0);
        assert_eq!(log.oldest_sequence(), None);
        assert_eq!(log.entry_count(), 0);
    }

    /// Entries written as JSON before 2.0.2 and MessagePack entries written
    /// since are both read back, from the cache at open and from disk.
    #[test]
    fn json_and_msgpack_entries_are_both_read() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        {
            let log = SyncLog::new("node1".to_string(), path, 0).unwrap();
            let mut legacy = create_test_entry(1);
            legacy.data = Some(br#"{"views":7}"#.to_vec());
            let mut batch = KvBatch::default();
            let default_cf = log.db.default_cf();
            batch.put_cf(
                &default_cf,
                format!("sync_log:{:020}", 1).as_bytes(),
                serde_json::to_vec(&legacy).unwrap(),
            );
            batch.put_cf(&default_cf, SEQ_KEY, 1u64.to_be_bytes());
            log.db.write(batch).unwrap();
        }
        for cache in [0, 100] {
            let log = SyncLog::new("node1".to_string(), path, cache).unwrap();
            if log.current_sequence() == 1 {
                let mut entry = create_test_entry(0);
                entry.data = Some(br#"{"views":42}"#.to_vec());
                assert_eq!(log.append(entry), 2);
            }
            let entries = log.get_entries_after(0, 10);
            assert_eq!(entries.len(), 2, "cache size {cache}");
            assert_eq!(entries[0].data.as_deref(), Some(&br#"{"views":7}"#[..]));
            assert_eq!(entries[1].data.as_deref(), Some(&br#"{"views":42}"#[..]));
            assert_eq!(entries[1].sequence, 2);
        }
        let log = SyncLog::new("node1".to_string(), path, 0).unwrap();
        let raw = log
            .db
            .get(format!("sync_log:{:020}", 2).as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(raw[0], MSGPACK_ENTRY);
    }

    /// A deferred log writes nothing until activated, then keeps writing,
    /// including after a restart.
    #[test]
    fn deferred_log_activates_and_stays_active() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().to_str().unwrap();
        {
            let log = SyncLog::new("node1".to_string(), path, 100).unwrap();
            log.defer_until_needed();
            assert!(!log.is_active());
            assert_eq!(log.append(create_test_entry(0)), 0);
            assert_eq!(log.entry_count(), 0);

            let clone = log.clone();
            clone.activate("test");
            assert!(log.is_active());
            assert_eq!(log.append(create_test_entry(0)), 1);
            assert_eq!(log.get_entries_after(0, 10).len(), 1);
        }
        let log = SyncLog::new("node1".to_string(), path, 100).unwrap();
        log.defer_until_needed();
        assert!(log.is_active(), "activation must survive a restart");
        assert_eq!(log.append(create_test_entry(0)), 2);
    }

    #[test]
    fn a_disabled_log_is_never_activated() {
        let tmp = TempDir::new().unwrap();
        let log =
            SyncLog::new_with_options("n".to_string(), tmp.path().to_str().unwrap(), 10, true)
                .unwrap();
        log.activate("test");
        assert!(!log.is_active());
        assert_eq!(log.append(create_test_entry(0)), 0);
    }

    #[test]
    fn test_disabled_log_appends_are_noops() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new_with_options(
            "node1".to_string(),
            tmp.path().to_str().unwrap(),
            100,
            true, // disabled
        )
        .unwrap();

        assert!(log.is_disabled());
        let seq = log.append(create_test_entry(0));
        assert_eq!(seq, 0);
        assert_eq!(log.current_sequence(), 0);
        assert_eq!(log.entry_count(), 0);

        let seq = log.append_batch(vec![create_test_entry(0), create_test_entry(0)]);
        assert_eq!(seq, 0);
        assert_eq!(log.entry_count(), 0);
    }

    #[test]
    fn test_oldest_sequence_tracks_first_entry() {
        let tmp = TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 100).unwrap();
        assert_eq!(log.oldest_sequence(), None);
        log.append(create_test_entry(0));
        assert_eq!(log.oldest_sequence(), Some(1));
        for _ in 0..4 {
            log.append(create_test_entry(0));
        }
        log.prune_before(3).unwrap();
        assert_eq!(log.oldest_sequence(), Some(3));
    }
    #[test]
    fn entries_are_found_by_their_age_and_archived() {
        let tmp = tempfile::TempDir::new().unwrap();
        let log = SyncLog::new("node1".to_string(), tmp.path().to_str().unwrap(), 0).unwrap();
        // An empty log: nothing is recent, nothing to archive.
        assert_eq!(log.first_sequence_since(0), 1);
        for n in 0..50u64 {
            let mut entry = LogEntry::new_op(
                "db",
                "c",
                Operation::Insert,
                format!("k{n}"),
                Some(vec![n as u8; 3]),
            );
            entry.timestamp = 1_000 + n * 10;
            log.append(entry);
        }
        // Stamps 1000, 1010 … 1490 under sequences 1 … 50.
        assert_eq!(log.first_sequence_since(0), 1);
        assert_eq!(log.first_sequence_since(1_000), 1);
        assert_eq!(log.first_sequence_since(1_001), 2);
        assert_eq!(log.first_sequence_since(1_250), 26);
        assert_eq!(log.first_sequence_since(1_490), 50);
        assert_eq!(log.first_sequence_since(1_491), 51);
        // After a prune the oldest entry kept is where the search starts.
        log.prune_before(20).unwrap();
        assert_eq!(log.first_sequence_since(0), 20);
        assert_eq!(log.first_sequence_since(1_250), 26);

        let file = tmp.path().join("archive.log");
        let summary = log.archive_after(30, &file).unwrap();
        assert_eq!(
            (
                summary.entries,
                summary.first_sequence,
                summary.last_sequence
            ),
            (20, 31, 50)
        );
        assert_eq!(
            (summary.first_timestamp, summary.last_timestamp),
            (1_300, 1_490)
        );
        let entries = SyncLog::read_archive(&file).unwrap();
        assert_eq!(entries.len(), 20);
        assert_eq!(entries[0].key, "k30");
        assert_eq!(entries[19].data, Some(vec![49u8; 3]));
        assert!(entries
            .windows(2)
            .all(|pair| pair[0].sequence + 1 == pair[1].sequence));
        // An existing file is not written over; a file cut short is an
        // error, not a shorter history.
        assert!(log.archive_after(30, &file).is_err());
        let bytes = std::fs::read(&file).unwrap();
        std::fs::write(&file, &bytes[..bytes.len() - 2]).unwrap();
        assert!(SyncLog::read_archive(&file).is_err());
    }
}
