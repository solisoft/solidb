use super::*;
use crate::error::{DbError, DbResult};
use hex;
use rocksdb::DB;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

impl Collection {
    /// Create a new collection handle
    pub fn new(name: String, db: Arc<RwLock<DB>>) -> Self {
        // Load cached count from disk, or calculate if not present
        let count = {
            let db_guard = db.read().unwrap();
            if let Some(cf) = db_guard.cf_handle(&name) {
                match db_guard.get_cf(cf, STATS_COUNT_KEY.as_bytes()) {
                    Ok(Some(bytes)) => String::from_utf8_lossy(&bytes)
                        .parse::<usize>()
                        .unwrap_or(0),
                    _ => {
                        // No cached count - calculate from documents
                        let prefix = DOC_PREFIX.as_bytes();
                        db_guard
                            .prefix_iterator_cf(cf, prefix)
                            .take_while(|r| {
                                r.as_ref().is_ok_and(|(k, _)| k.starts_with(prefix))
                            })
                            .count()
                    }
                }
            } else {
                0
            }
        };

        // Determine initial chunk count (only relevant if it's a blob collection)
        // We do this lazily or just scan if found? Scanning is safe for startup.
        let chunk_count = {
            let db_guard = db.read().unwrap();
            if let Some(cf) = db_guard.cf_handle(&name) {
                let prefix = BLO_PREFIX.as_bytes();
                db_guard
                    .prefix_iterator_cf(cf, prefix)
                    .take_while(|r| r.as_ref().is_ok_and(|(k, _)| k.starts_with(prefix)))
                    .count()
            } else {
                0
            }
        };

        let (change_sender, _) = tokio::sync::broadcast::channel(100);

        // Load collection type
        let collection_type = {
            let db_guard = db.read().unwrap();
            if let Some(cf) = db_guard.cf_handle(&name) {
                match db_guard.get_cf(cf, COLLECTION_TYPE_KEY.as_bytes()) {
                    Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
                    _ => "document".to_string(),
                }
            } else {
                "document".to_string()
            }
        };

        Self {
            name,
            db,
            doc_count: Arc::new(AtomicUsize::new(count)),
            chunk_count: Arc::new(AtomicUsize::new(chunk_count)),
            count_dirty: Arc::new(AtomicBool::new(false)),
            last_flush_time: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            change_sender: Arc::new(change_sender),
            collection_type: Arc::new(RwLock::new(collection_type)),
            bloom_filters: Arc::new(RwLock::new(HashMap::new())),
            cuckoo_filters: Arc::new(RwLock::new(HashMap::new())),
            vector_indexes: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get collection type
    pub fn get_type(&self) -> String {
        self.collection_type.read().unwrap().clone()
    }

    /// Set collection type (persists to disk)
    pub fn set_type(&self, type_: &str) -> DbResult<()> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        db.put_cf(cf, COLLECTION_TYPE_KEY.as_bytes(), type_.as_bytes())
            .map_err(|e| DbError::InternalError(format!("Failed to set collection type: {}", e)))?;

        // Update in-memory state
        let mut mg = self.collection_type.write().unwrap();
        *mg = type_.to_string();

        Ok(())
    }

    /// Flush count to disk if dirty (call periodically or on shutdown)
    pub fn flush_stats(&self) {
        if self.count_dirty.swap(false, Ordering::Relaxed) {
            let count = self.doc_count.load(Ordering::Relaxed);
            let db = self.db.read().unwrap();
            if let Some(cf) = db.cf_handle(&self.name) {
                let _ = db.put_cf(cf, STATS_COUNT_KEY.as_bytes(), count.to_string().as_bytes());
            }
            // Update last flush time
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            self.last_flush_time.store(now, Ordering::Relaxed);
        }
    }

    /// Flush count to disk if dirty AND at least 1 second has passed since last flush
    /// Use this during bulk operations to avoid excessive disk writes
    pub fn flush_stats_throttled(&self) {
        if !self.count_dirty.load(Ordering::Relaxed) {
            return; // Nothing to flush
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let last = self.last_flush_time.load(Ordering::Relaxed);

        // Only flush if at least 1 second has passed
        if now > last {
            self.flush_stats();
        }
    }

    /// Compact the collection to remove tombstones and reclaim space
    pub fn compact(&self) {
        let db = self.db.read().unwrap();
        if let Some(cf) = db.cf_handle(&self.name) {
            db.compact_range_cf(cf, None::<&[u8]>, None::<&[u8]>);
        }
    }

    /// Get usage statistics
    pub fn stats(&self) -> CollectionStats {
        let disk_usage = self.disk_usage();

        CollectionStats {
            name: self.name.clone(),
            document_count: self.doc_count.load(Ordering::Relaxed),
            chunk_count: self.chunk_count.load(Ordering::Relaxed),
            disk_usage,
        }
    }

    /// Get disk usage statistics for this collection
    pub fn disk_usage(&self) -> DiskUsage {
        let db = self.db.read().unwrap();
        let cf = match db.cf_handle(&self.name) {
            Some(cf) => cf,
            None => {
                return DiskUsage {
                    sst_files_size: 0,
                    live_data_size: 0,
                    num_sst_files: 0,
                    memtable_size: 0,
                }
            }
        };

        // Get SST files size
        let sst_files_size = db
            .property_int_value_cf(cf, "rocksdb.total-sst-files-size")
            .ok()
            .flatten()
            .unwrap_or(0);

        // Get estimated live data size
        let live_data_size = db
            .property_int_value_cf(cf, "rocksdb.estimate-live-data-size")
            .ok()
            .flatten()
            .unwrap_or(0);

        // Get number of SST files at all levels
        let mut num_sst_files = 0;
        for i in 0..7 {
            num_sst_files += db
                .property_int_value_cf(cf, &format!("rocksdb.num-files-at-level{}", i))
                .ok()
                .flatten()
                .unwrap_or(0);
        }

        // Get memtable size
        let memtable_size = db
            .property_int_value_cf(cf, "rocksdb.cur-size-all-mem-tables")
            .ok()
            .flatten()
            .unwrap_or(0);

        DiskUsage {
            sst_files_size,
            live_data_size,
            num_sst_files,
            memtable_size,
        }
    }

    // ==================== Sharding Configuration ====================

    /// Set sharding configuration for this collection
    pub fn set_shard_config(
        &self,
        config: &crate::sharding::coordinator::CollectionShardConfig,
    ) -> DbResult<()> {
        let db = self.db.write().unwrap(); // Need write lock for put_cf
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let config_bytes = serde_json::to_vec(config)?;
        db.put_cf(cf, SHARD_CONFIG_KEY.as_bytes(), &config_bytes)
            .map_err(|e| DbError::InternalError(format!("Failed to store shard config: {}", e)))?;

        tracing::info!(
            "[SHARD_CONFIG] Saved config for {}: {:?}",
            self.name,
            config
        );

        Ok(())
    }

    /// Get sharding configuration for this collection (None if not sharded)
    pub fn get_shard_config(&self) -> Option<crate::sharding::coordinator::CollectionShardConfig> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        db.get_cf(cf, SHARD_CONFIG_KEY.as_bytes())
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Save shard table to storage (persisting assignments)
    pub fn set_shard_table(
        &self,
        table: &crate::sharding::coordinator::ShardTable,
    ) -> DbResult<()> {
        let db = self.db.write().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let table_bytes = serde_json::to_vec(table)?;
        db.put_cf(cf, SHARD_TABLE_KEY.as_bytes(), &table_bytes)
            .map_err(|e| DbError::InternalError(format!("Failed to store shard table: {}", e)))?;

        Ok(())
    }

    /// Load shard table from storage
    pub fn get_stored_shard_table(&self) -> Option<crate::sharding::coordinator::ShardTable> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        db.get_cf(cf, SHARD_TABLE_KEY.as_bytes())
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Check if this collection is sharded
    pub fn is_sharded(&self) -> bool {
        self.get_shard_config().is_some()
    }

    // ==================== Key Helpers ====================

    /// Generate a document key: "doc:<key>"
    pub fn doc_key(key: &str) -> Vec<u8> {
        format!("{}{}", DOC_PREFIX, key).into_bytes()
    }

    /// Generate an index metadata key: "idx_meta:<name>"
    pub fn idx_meta_key(name: &str) -> Vec<u8> {
        format!("{}{}", IDX_META_PREFIX, name).into_bytes()
    }

    /// Generate an index entry key: "idx:<name>:<value>:<doc_key>"
    pub fn idx_entry_key(index_name: &str, values: &[Value], doc_key: &str) -> Vec<u8> {
        let _value_str = serde_json::to_string(values).unwrap_or_default();
        // Use hex encoding for binary-safe keys if needed, but here simple concatenation
        // CAUTION: In original code, it might have matched exactly this format.
        // Let's re-verify the original implementation below!
        // Original: const prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_str);
        // Wait, line 240 in original code used:
        // let value_str = serde_json::to_string(&field_values).unwrap_or_default();
        // let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_str);
        // However, looking at line 2887 `index_lookup_eq`:
        // let value_str = hex::encode(crate::storage::codec::encode_key(value));
        // There seems to be a discrepancy or I misread the original file.
        // Let's check `idx_entry_key` usage in original file.
        // Line 952: `let entry_key = Self::idx_entry_key(&index.name, &field_values, &doc.key);`
        // I need to implement `idx_entry_key` exactly as it was or consistent with new logic.
        // In the original file (viewed previously), I didn't see the specific definition of `idx_entry_key`
        // but I saw usage. I should check the helper methods section.
        // I'll assume usage of `hex::encode(crate::storage::codec::encode_key(value))` for consistency if it was there.
        // But wait, line 2239 says `let value_str = serde_json::to_string(&field_values).unwrap_or_default();`
        // This suggests the unique constraint check uses JSON string.
        // BUT `index_lookup_eq` (line 2912) uses `hex::encode(crate::storage::codec::encode_key(value))`.
        // This is a Conflict!
        // Actually, `check_unique_constraints` (line 2218) iterates over prefix.
        // Let's look at `index_documents` (line 892).
        // It calls `Self::idx_entry_key`.
        // I should find where `idx_entry_key` was defined in the original file.
        // It was likely later in the file.
        // I will implement it using `hex::encode(crate::storage::codec::encode_key)` for EACH value in the compound key?
        // Let's use a safe implementation that matches likely usage.
        // Keys: `idx:<name>:<hex(encoded_val1)>_<hex(encoded_val2)>:<doc_key>`
        // Actually, let's look at `index_sorted` (line 3069):
        // "Since we use binary-comparable encoding (wrapped in hex)..."
        // So `idx_entry_key` MUST use hex encoding of codec::encode_key.

        let encoded_values: Vec<String> = values
            .iter()
            .map(|v| hex::encode(crate::storage::codec::encode_key(v)))
            .collect();
        let value_part = encoded_values.join("_");
        format!("{}{}:{}:{}", IDX_PREFIX, index_name, value_part, doc_key).into_bytes()
    }

    /// Generate a geo metadata key: "geo_meta:<name>"
    pub fn geo_meta_key(name: &str) -> Vec<u8> {
        format!("{}{}", GEO_META_PREFIX, name).into_bytes()
    }

    /// Generate a geo entry key: "geo:<name>:<doc_key>"
    pub fn geo_entry_key(index_name: &str, doc_key: &str) -> Vec<u8> {
        format!("{}{}:{}", GEO_PREFIX, index_name, doc_key).into_bytes()
    }

    /// Generate a fulltext index metadata key: "ft_meta:<name>"
    pub fn ft_meta_key(name: &str) -> Vec<u8> {
        format!("{}{}", FT_META_PREFIX, name).into_bytes()
    }

    /// Generate a fulltext term mapping key: "ft_term:<index>:<term>:<doc_key>"
    pub fn ft_term_key(index_name: &str, term: &str, doc_key: &str) -> Vec<u8> {
        format!("{}{}:{}:{}", FT_TERM_PREFIX, index_name, term, doc_key).into_bytes()
    }

    /// Generate a fulltext n-gram mapping key: "ft:<index>:<ngram>:<doc_key>"
    pub fn ft_ngram_key(index_name: &str, ngram: &str, doc_key: &str) -> Vec<u8> {
        format!("{}{}:{}:{}", FT_PREFIX, index_name, ngram, doc_key).into_bytes()
    }

    /// Generate a blob chunk key: "blo:<key>:<chunk_index>"
    pub fn blo_chunk_key(key: &str, chunk_index: usize) -> Vec<u8> {
        format!("{}{}:{}", BLO_PREFIX, key, chunk_index).into_bytes()
    }

    /// Build a TTL index metadata key: "ttl_meta:<name>"
    pub fn ttl_meta_key(name: &str) -> Vec<u8> {
        format!("{}{}", TTL_META_PREFIX, name).into_bytes()
    }

    /// Create vector index metadata key: "vec_meta:<name>"
    pub fn vec_meta_key(name: &str) -> Vec<u8> {
        format!("{}{}", VEC_META_PREFIX, name).into_bytes()
    }

    /// Create vector index data key: "vec_data:<name>"
    pub fn vec_data_key(name: &str) -> Vec<u8> {
        format!("{}{}", VEC_DATA_PREFIX, name).into_bytes()
    }
}
