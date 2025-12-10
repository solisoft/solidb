use rocksdb::{IteratorMode, Options, DB};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use super::HybridLogicalClock;

/// Type of operation in the replication log
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Operation {
    // Document operations
    Insert,
    Update,
    Delete,
    // Collection operations
    CreateCollection,
    DeleteCollection,
    TruncateCollection,
    // Database operations
    CreateDatabase,
    DeleteDatabase,
    // Blob operations
    PutBlobChunk,
    DeleteBlob,
}

/// A single entry in the replication log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplicationEntry {
    /// Sequence number (monotonic per node)
    pub sequence: u64,

    /// Origin node ID
    pub node_id: String,

    /// Hybrid logical clock timestamp
    pub hlc: HybridLogicalClock,

    /// Database name
    pub database: String,

    /// Collection name
    pub collection: String,

    /// Type of operation
    pub operation: Operation,

    /// Document key
    pub document_key: String,

    /// Document data (None for deletes)
    pub document_data: Option<Vec<u8>>,

    /// Previous revision (for conflict detection)
    pub prev_rev: Option<String>,

    /// Chunk index (for blob chunks)
    pub chunk_index: Option<u32>,
}

impl ReplicationEntry {
    pub fn new(
        sequence: u64,
        node_id: String,
        hlc: HybridLogicalClock,
        database: String,
        collection: String,
        operation: Operation,
        document_key: String,
        document_data: Option<Vec<u8>>,
        prev_rev: Option<String>,
    ) -> Self {
        Self {
            sequence,
            node_id,
            hlc,
            database,
            collection,
            operation,
            document_key,
            document_data,
            prev_rev,
            chunk_index: None,
        }
    }

    pub fn new_blob_chunk(
        sequence: u64,
        node_id: String,
        hlc: HybridLogicalClock,
        database: String,
        collection: String,
        document_key: String,
        chunk_index: u32,
        data: Vec<u8>,
    ) -> Self {
        Self {
            sequence,
            node_id,
            hlc,
            database,
            collection,
            operation: Operation::PutBlobChunk,
            document_key,
            document_data: Some(data),
            prev_rev: None,
            chunk_index: Some(chunk_index),
        }
    }

    /// Serialize to bytes for storage/transmission
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("Failed to serialize ReplicationEntry")
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

const REPL_LOG_PREFIX: &[u8] = b"repl:";
const REPL_SEQ_KEY: &[u8] = b"repl:_sequence";

/// Persistent replication log backed by RocksDB
pub struct PersistentReplicationLog {
    node_id: String,
    db: Arc<RwLock<DB>>,
    sequence: Arc<RwLock<u64>>,
    max_entries: usize,
    // In-memory cache for recent entries
    cache: Arc<RwLock<VecDeque<ReplicationEntry>>>,
    cache_size: usize,
}

impl PersistentReplicationLog {
    pub fn new(node_id: String, data_dir: &str, max_entries: usize) -> Result<Self, String> {
        let repl_path = format!("{}/replication", data_dir);

        let mut opts = Options::default();
        opts.create_if_missing(true);

        let db = DB::open(&opts, &repl_path)
            .map_err(|e| format!("Failed to open replication log: {}", e))?;

        // Load current sequence from disk
        let sequence = match db.get(REPL_SEQ_KEY) {
            Ok(Some(bytes)) => {
                let seq_str = String::from_utf8_lossy(&bytes);
                seq_str.parse::<u64>().unwrap_or(0)
            }
            _ => 0,
        };

        tracing::debug!(
            "[REPL-LOG] Initialized at sequence {} (path: {})",
            sequence,
            repl_path
        );

        let log = Self {
            node_id,
            db: Arc::new(RwLock::new(db)),
            sequence: Arc::new(RwLock::new(sequence)),
            max_entries,
            cache: Arc::new(RwLock::new(VecDeque::with_capacity(10000))),
            cache_size: 10000, // Keep cache small to avoid memory bloat with large imports
        };

        // Load recent entries into cache
        log.load_cache();

        Ok(log)
    }

    fn load_cache(&self) {
        let db = self.db.read().unwrap();
        let mut cache = self.cache.write().unwrap();
        cache.clear();

        let iter = db.iterator(IteratorMode::From(
            REPL_LOG_PREFIX,
            rocksdb::Direction::Forward,
        ));
        let mut entries: Vec<ReplicationEntry> = Vec::new();

        for item in iter {
            if let Ok((key, value)) = item {
                if !key.starts_with(REPL_LOG_PREFIX) || key.as_ref() == REPL_SEQ_KEY {
                    continue;
                }
                if let Ok(entry) = ReplicationEntry::from_bytes(&value) {
                    entries.push(entry);
                }
            }
        }

        // Keep only last cache_size entries
        let start = entries.len().saturating_sub(self.cache_size);
        for entry in entries.into_iter().skip(start) {
            cache.push_back(entry);
        }

        tracing::debug!("[REPL-LOG] Loaded {} entries into cache", cache.len());
    }

    /// Append a new entry to the log
    pub fn append(&self, mut entry: ReplicationEntry) -> u64 {
        let mut seq = self.sequence.write().unwrap();
        let db = self.db.write().unwrap();
        let mut cache = self.cache.write().unwrap();

        *seq += 1;
        entry.sequence = *seq;
        entry.node_id = self.node_id.clone();

        // Create key: repl:00000000000000001234
        let key = format!("repl:{:020}", *seq);
        let value = entry.to_bytes();

        // Write to RocksDB
        if let Err(e) = db.put(key.as_bytes(), &value) {
            tracing::error!("[REPL-LOG] Failed to persist entry: {}", e);
        }

        // Update sequence on disk
        if let Err(e) = db.put(REPL_SEQ_KEY, seq.to_string().as_bytes()) {
            tracing::error!("[REPL-LOG] Failed to persist sequence: {}", e);
        }

        // Update cache
        cache.push_back(entry);
        while cache.len() > self.cache_size {
            cache.pop_front();
        }

        // Trim old entries if needed
        if *seq > self.max_entries as u64 {
            let trim_before = *seq - self.max_entries as u64;
            self.trim_before(trim_before, &db);
        }

        *seq
    }

    /// Append a batch of entries to the log atomically
    pub fn append_batch(&self, entries: Vec<ReplicationEntry>) -> u64 {
        if entries.is_empty() {
            return self.current_sequence();
        }

        let mut seq = self.sequence.write().unwrap();
        let db = self.db.write().unwrap();
        let mut cache = self.cache.write().unwrap();
        let mut batch = rocksdb::WriteBatch::default();

        let count = entries.len();

        for (_i, mut entry) in entries.into_iter().enumerate() {
            *seq += 1;
            entry.sequence = *seq;
            entry.node_id = self.node_id.clone();

            let key = format!("repl:{:020}", *seq);
            let value = entry.to_bytes();
            batch.put(key.as_bytes(), &value);

            // Add to cache (skip very large entries to save memory)
            let entry_size = entry.document_data.as_ref().map(|d| d.len()).unwrap_or(0);
            if entry_size < 10_000 { // Only cache entries smaller than 10KB
                cache.push_back(entry);
            }
        }

        // Update sequence on disk
        batch.put(REPL_SEQ_KEY, seq.to_string().as_bytes());

        // Write the batch atomically
        if let Err(e) = db.write(batch) {
             tracing::error!("[REPL-LOG] Failed to persist batch of {} entries: {}", count, e);
        }

        while cache.len() > self.cache_size {
            cache.pop_front();
        }

        // Trim old entries if needed (check only once per batch)
        if *seq > self.max_entries as u64 {
            let trim_before = *seq - self.max_entries as u64;
            self.trim_before(trim_before, &db);
        }

        *seq
    }

    fn trim_before(&self, before_sequence: u64, db: &DB) {
        let prefix = format!("repl:{:020}", 0);
        let end_key = format!("repl:{:020}", before_sequence);

        let iter = db.iterator(IteratorMode::From(
            prefix.as_bytes(),
            rocksdb::Direction::Forward,
        ));
        let mut to_delete = Vec::new();

        for item in iter {
            if let Ok((key, _)) = item {
                let key_str = String::from_utf8_lossy(&key);
                if key_str.as_ref() >= end_key.as_str()
                    || !key_str.starts_with("repl:")
                    || key.as_ref() == REPL_SEQ_KEY
                {
                    break;
                }
                to_delete.push(key.to_vec());
            }
        }

        for key in to_delete {
            let _ = db.delete(&key);
        }
    }

    /// Get entries after a given sequence number (with optional limit)
    pub fn get_entries_after(&self, after_sequence: u64) -> Vec<ReplicationEntry> {
        self.get_entries_after_limit(after_sequence, None)
    }

    /// Get entries after a given sequence number with a limit
    pub fn get_entries_after_limit(&self, after_sequence: u64, limit: Option<usize>) -> Vec<ReplicationEntry> {
        // Always read from disk for correctness - cache may have gaps due to size filtering
        // This is safer and disk reads with limits are efficient

        // Fall back to disk
        let db = self.db.read().unwrap();
        let start_key = format!("repl:{:020}", after_sequence + 1);
        let iter = db.iterator(IteratorMode::From(
            start_key.as_bytes(),
            rocksdb::Direction::Forward,
        ));

        let mut entries = Vec::new();
        let max_entries = limit.unwrap_or(usize::MAX);
        
        for item in iter {
            if entries.len() >= max_entries {
                break;
            }
            if let Ok((key, value)) = item {
                if !key.starts_with(REPL_LOG_PREFIX) || key.as_ref() == REPL_SEQ_KEY {
                    continue;
                }
                if let Ok(entry) = ReplicationEntry::from_bytes(&value) {
                    entries.push(entry);
                }
            }
        }

        entries
    }

    /// Get the current sequence number
    pub fn current_sequence(&self) -> u64 {
        *self.sequence.read().unwrap()
    }

    /// Get entry count (approximate from cache)
    pub fn len(&self) -> usize {
        self.cache.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.read().unwrap().is_empty()
    }
}

impl Clone for PersistentReplicationLog {
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id.clone(),
            db: Arc::clone(&self.db),
            sequence: Arc::clone(&self.sequence),
            max_entries: self.max_entries,
            cache: Arc::clone(&self.cache),
            cache_size: self.cache_size,
        }
    }
}

/// In-memory replication log with bounded size (for testing/simple deployments)
pub struct ReplicationLog {
    node_id: String,
    entries: Arc<RwLock<VecDeque<ReplicationEntry>>>,
    sequence: Arc<RwLock<u64>>,
    max_entries: usize,
}

impl ReplicationLog {
    pub fn new(node_id: String, max_entries: usize) -> Self {
        Self {
            node_id,
            entries: Arc::new(RwLock::new(VecDeque::new())),
            sequence: Arc::new(RwLock::new(0)),
            max_entries,
        }
    }

    /// Append a new entry to the log
    pub fn append(&self, mut entry: ReplicationEntry) -> u64 {
        let mut seq = self.sequence.write().unwrap();
        let mut entries = self.entries.write().unwrap();

        *seq += 1;
        entry.sequence = *seq;
        entry.node_id = self.node_id.clone();

        entries.push_back(entry);

        // Trim old entries if we exceed max
        while entries.len() > self.max_entries {
            entries.pop_front();
        }

        *seq
    }

    /// Append a batch of entries (in-memory version)
    pub fn append_batch(&self, entries: Vec<ReplicationEntry>) -> u64 {
        let mut seq = self.sequence.write().unwrap();
        let mut log_entries = self.entries.write().unwrap();

        for mut entry in entries {
            *seq += 1;
            entry.sequence = *seq;
            entry.node_id = self.node_id.clone();
            log_entries.push_back(entry);
        }

        while log_entries.len() > self.max_entries {
            log_entries.pop_front();
        }

        *seq
    }

    /// Get entries after a given sequence number
    pub fn get_entries_after(&self, after_sequence: u64) -> Vec<ReplicationEntry> {
        let entries = self.entries.read().unwrap();
        entries
            .iter()
            .filter(|e| e.sequence > after_sequence)
            .cloned()
            .collect()
    }

    /// Get the current sequence number
    pub fn current_sequence(&self) -> u64 {
        *self.sequence.read().unwrap()
    }

    /// Get all entries (for debugging/testing)
    pub fn all_entries(&self) -> Vec<ReplicationEntry> {
        self.entries.read().unwrap().iter().cloned().collect()
    }

    /// Get entry count
    pub fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.read().unwrap().is_empty()
    }
}

impl Clone for ReplicationLog {
    fn clone(&self) -> Self {
        Self {
            node_id: self.node_id.clone(),
            entries: Arc::clone(&self.entries),
            sequence: Arc::clone(&self.sequence),
            max_entries: self.max_entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_replication_log_append() {
        let log = ReplicationLog::new("node-1".to_string(), 100);

        let entry = ReplicationEntry::new(
            0,
            "".to_string(),
            HybridLogicalClock::new(1000, 0, "node-1".to_string()),
            "testdb".to_string(),
            "users".to_string(),
            Operation::Insert,
            "user-1".to_string(),
            Some(b"{}".to_vec()),
            None,
        );

        let seq1 = log.append(entry.clone());
        let seq2 = log.append(entry.clone());

        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn test_get_entries_after() {
        let log = ReplicationLog::new("node-1".to_string(), 100);

        for _ in 0..5 {
            let entry = ReplicationEntry::new(
                0,
                "".to_string(),
                HybridLogicalClock::new(1000, 0, "node-1".to_string()),
                "testdb".to_string(),
                "users".to_string(),
                Operation::Insert,
                "user-1".to_string(),
                None,
                None,
            );
            log.append(entry);
        }

        let entries = log.get_entries_after(3);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].sequence, 4);
        assert_eq!(entries[1].sequence, 5);
    }

    #[test]
    fn test_log_trimming() {
        let log = ReplicationLog::new("node-1".to_string(), 3);

        for _ in 0..5 {
            let entry = ReplicationEntry::new(
                0,
                "".to_string(),
                HybridLogicalClock::new(1000, 0, "node-1".to_string()),
                "testdb".to_string(),
                "users".to_string(),
                Operation::Insert,
                "user-1".to_string(),
                None,
                None,
            );
            log.append(entry);
        }

        assert_eq!(log.len(), 3);
        let entries = log.all_entries();
        assert_eq!(entries[0].sequence, 3);
    }
}
