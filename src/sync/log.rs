//! Sync log for recording mutations that need to be replicated
//!
//! This module provides a local write-ahead log for mutations that
//! need to be synchronized to other nodes in the cluster.

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use crate::storage::RocksDb as DB;
use rust_rocksdb::{Direction, IteratorMode, Options, WriteBatch};
use serde::{Deserialize, Serialize};

use super::protocol::{Operation, SyncEntry};
use crate::cluster::HybridLogicalClock;

const LOG_PREFIX: &[u8] = b"sync_log:";
const SEQ_KEY: &[u8] = b"sync_log:_sequence";

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

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_max_write_buffer_number(4);
        opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
        opts.set_keep_log_file_num(5);
        opts.set_recycle_log_file_num(3);

        let db = DB::open(&opts, &log_path).map_err(|e| e.to_string())?;
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
            .iterator(IteratorMode::From(LOG_PREFIX, Direction::Forward));
        let mut count = 0;

        for (key, value) in iter.flatten() {
            if !key.starts_with(LOG_PREFIX) || key.as_ref() == SEQ_KEY {
                continue;
            }

            if let Ok(entry) = serde_json::from_slice::<LogEntry>(&value) {
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
        if self.disabled {
            return self.current_sequence();
        }

        let mut seq = self.sequence.write().unwrap();
        *seq += 1;
        entry.sequence = *seq;

        if entry.node_id.is_empty() {
            entry.node_id = self.node_id.clone();
        }

        // Use a single WriteBatch so the entry and the bumped sequence go
        // through one `db.write` (one fsync) instead of two independent
        // `put` calls (two fsyncs). This roughly halves the per-write
        // disk cost on hot insert/update paths.
        let key = format!("sync_log:{:020}", *seq);
        let value = serde_json::to_vec(&entry).unwrap();

        let mut batch = WriteBatch::default();
        batch.put(key.as_bytes(), &value);
        batch.put(SEQ_KEY, seq.to_be_bytes());

        if let Err(e) = self.db.write(&batch) {
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

    /// Append multiple entries atomically
    pub fn append_batch(&self, mut entries: Vec<LogEntry>) -> u64 {
        if self.disabled || entries.is_empty() {
            return self.current_sequence();
        }

        let mut seq = self.sequence.write().unwrap();
        let mut batch = WriteBatch::default();

        for entry in &mut entries {
            *seq += 1;
            entry.sequence = *seq;

            if entry.node_id.is_empty() {
                entry.node_id = self.node_id.clone();
            }

            let key = format!("sync_log:{:020}", *seq);
            let value = serde_json::to_vec(&entry).unwrap();
            batch.put(key.as_bytes(), &value);
        }

        batch.put(SEQ_KEY, seq.to_be_bytes());
        if let Err(e) = self.db.write(&batch) {
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

    /// Returns true if append/append_batch are no-ops.
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// Returns the sequence of the oldest retained entry, or None if the log is empty.
    pub fn oldest_sequence(&self) -> Option<u64> {
        let iter = self
            .db
            .iterator(IteratorMode::From(LOG_PREFIX, Direction::Forward));
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
            .iterator(IteratorMode::From(LOG_PREFIX, Direction::Forward));
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

        // Walk the to-be-pruned range, building a WriteBatch of explicit
        // deletes. This is portable across rocksdb wrappers and gives an
        // exact removed count for reporting.
        let mut batch = WriteBatch::default();
        let mut removed = 0u64;
        let iter = self
            .db
            .iterator(IteratorMode::From(start_key.as_bytes(), Direction::Forward));
        for (key, _) in iter.flatten() {
            if !key.starts_with(LOG_PREFIX) || key.as_ref() == SEQ_KEY {
                continue;
            }
            if key.as_ref() >= end_key.as_bytes() {
                break;
            }
            batch.delete(&key);
            removed += 1;
        }

        if removed == 0 {
            return Ok(0);
        }

        self.db.write(&batch).map_err(|e| e.to_string())?;

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
            .iterator(IteratorMode::From(start_key.as_bytes(), Direction::Forward));

        let mut entries = Vec::new();
        for (key, value) in iter.flatten() {
            if !key.starts_with(LOG_PREFIX) || key.as_ref() == SEQ_KEY {
                continue;
            }

            if let Ok(entry) = serde_json::from_slice::<LogEntry>(&value) {
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

    /// Get current sequence number
    pub fn current_sequence(&self) -> u64 {
        *self.sequence.read().unwrap()
    }

    /// Get node ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Convenience method to append a columnar operation
    pub fn append_columnar(
        &self,
        database: &str,
        collection: &str,
        operation: Operation,
        key: String,
        data: Option<Vec<u8>>,
    ) -> u64 {
        let entry = LogEntry {
            sequence: 0,
            node_id: String::new(),
            database: database.to_string(),
            collection: collection.to_string(),
            operation,
            key,
            data,
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        self.append(entry)
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
}
