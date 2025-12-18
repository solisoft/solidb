//! Sync log for recording mutations that need to be replicated
//!
//! This module provides a local write-ahead log for mutations that
//! need to be synchronized to other nodes in the cluster.

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use rocksdb::{DB, Options, IteratorMode};
use serde::{Serialize, Deserialize};

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
            operation: self.operation.clone(),
            document_key: self.key.clone(),
            document_data: self.data.clone(),
            shard_id: None,
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
}

impl SyncLog {
    /// Create a new sync log
    pub fn new(node_id: String, data_dir: &str, max_cache_size: usize) -> Result<Self, String> {
        let log_path = format!("{}/sync_log", data_dir);
        
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.set_max_write_buffer_number(4);
        opts.set_write_buffer_size(64 * 1024 * 1024); // 64MB
        
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
        };
        
        log.load_cache();
        Ok(log)
    }
    
    /// Load recent entries into cache
    fn load_cache(&self) {
        let mut cache = self.cache.write().unwrap();
        cache.clear();
        
        let iter = self.db.iterator(IteratorMode::From(LOG_PREFIX, rocksdb::Direction::Forward));
        let mut count = 0;
        
        for item in iter {
            if let Ok((key, value)) = item {
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
    }
    
    /// Append an entry to the log
    pub fn append(&self, mut entry: LogEntry) -> u64 {
        let mut seq = self.sequence.write().unwrap();
        *seq += 1;
        entry.sequence = *seq;
        
        if entry.node_id.is_empty() {
            entry.node_id = self.node_id.clone();
        }
        
        // Write to RocksDB
        let key = format!("sync_log:{:020}", *seq);
        let value = serde_json::to_vec(&entry).unwrap();
        
        if let Err(e) = self.db.put(key.as_bytes(), &value) {
            tracing::error!("SyncLog: Failed to write entry {}: {}", *seq, e);
        }
        if let Err(e) = self.db.put(SEQ_KEY, seq.to_be_bytes()) {
            tracing::error!("SyncLog: Failed to write sequence {}: {}", *seq, e);
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
        if entries.is_empty() {
            return self.current_sequence();
        }
        
        let mut seq = self.sequence.write().unwrap();
        let mut batch = rocksdb::WriteBatch::default();
        
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
        let iter = self.db.iterator(IteratorMode::From(start_key.as_bytes(), rocksdb::Direction::Forward));
        
        let mut entries = Vec::new();
        for item in iter {
            if let Ok((key, value)) = item {
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
}

impl Clone for SyncLog {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            node_id: self.node_id.clone(),
            sequence: self.sequence.clone(),
            cache: self.cache.clone(),
            max_cache_size: self.max_cache_size,
        }
    }
}
// Operation is already exported via mod.rs
