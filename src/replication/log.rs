use std::sync::{Arc, RwLock};
use rocksdb::{IteratorMode, Options, DB};
use super::protocol::LogEntry;

const LOG_PREFIX: &[u8] = b"repl:";
const SEQ_KEY: &[u8] = b"repl:_sequence";

#[derive(Clone)]
pub struct ReplicationLog {
    db: Arc<RwLock<DB>>,
    sequence: Arc<RwLock<u64>>,
    node_id: String,
}

impl ReplicationLog {
    pub fn new(data_dir: &str, node_id: String) -> Result<Self, String> {
        let path = format!("{}/replication_v2", data_dir);
        let mut opts = Options::default();
        opts.create_if_missing(true);
        
        let db = DB::open(&opts, &path)
            .map_err(|e| format!("Failed to open replication log: {}", e))?;
            
        let sequence = match db.get(SEQ_KEY) {
            Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).parse().unwrap_or(0),
            _ => 0,
        };

        Ok(Self {
            db: Arc::new(RwLock::new(db)),
            sequence: Arc::new(RwLock::new(sequence)),
            node_id,
        })
    }

    pub fn append(&self, mut entry: LogEntry) -> Result<u64, String> {
        let mut seq = self.sequence.write().unwrap();
        let db = self.db.write().unwrap();
        
        *seq += 1;
        entry.sequence = *seq;
        if entry.node_id.is_empty() {
            entry.node_id = self.node_id.clone();
        }
        if entry.origin_sequence.is_none() {
            entry.origin_sequence = Some(*seq);
        }
        
        let key = format!("repl:{:020}", *seq);
        let value = serde_json::to_vec(&entry).map_err(|e| e.to_string())?;
        
        db.put(key.as_bytes(), &value).map_err(|e| e.to_string())?;
        db.put(SEQ_KEY, seq.to_string().as_bytes()).map_err(|e| e.to_string())?;
        
        Ok(*seq)
    }

    pub fn append_batch(&self, entries: Vec<LogEntry>) -> Result<u64, String> {
        let mut seq = self.sequence.write().unwrap();
        let db = self.db.write().unwrap();
        let mut batch = rocksdb::WriteBatch::default();
        
        let mut last_seq = *seq;
        
        for mut entry in entries {
            last_seq += 1;
            entry.sequence = last_seq;
            if entry.node_id.is_empty() {
                entry.node_id = self.node_id.clone();
            }
            if entry.origin_sequence.is_none() {
                entry.origin_sequence = Some(last_seq);
            }
            
            let key = format!("repl:{:020}", last_seq);
            let value = serde_json::to_vec(&entry).map_err(|e| e.to_string())?;
            batch.put(key.as_bytes(), &value);
        }
        
        batch.put(SEQ_KEY, last_seq.to_string().as_bytes());
        db.write(batch).map_err(|e| e.to_string())?;
        
        *seq = last_seq;
        Ok(*seq)
    }


    pub fn get_entries_after(&self, after_seq: u64, limit: usize) -> Vec<LogEntry> {
        let db = self.db.read().unwrap();
        let start_key = format!("repl:{:020}", after_seq + 1);
        let iter = db.iterator(IteratorMode::From(start_key.as_bytes(), rocksdb::Direction::Forward));
        
        let mut entries = Vec::new();
        for item in iter {
            if entries.len() >= limit { break; }
            if let Ok((key, value)) = item {
                if !key.starts_with(LOG_PREFIX) { break; }
                if let Ok(entry) = serde_json::from_slice::<LogEntry>(&value) {
                    entries.push(entry);
                }
            }
        }
        entries
    }

    pub fn current_sequence(&self) -> u64 {
        *self.sequence.read().unwrap()
    }

    pub fn count(&self) -> u64 {
        *self.sequence.read().unwrap()
    }
}

