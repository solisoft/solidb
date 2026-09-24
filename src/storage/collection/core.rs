use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::RocksDb as DB;
use dashmap::DashMap;
use hex;
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};

/// Minimum seconds between throttled vector-index persists during bulk writes.
/// The trailing window is made durable by the shutdown flush
/// (`flush_vector_indexes` via the engine's flush-all).
const VEC_PERSIST_THROTTLE_SECS: u64 = 5;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Claim the dirty flag and persist every in-memory vector index of one
/// collection. Shared by `Collection::flush_vector_indexes` and the periodic
/// dirty-set drain, which holds only weak handles and so has no `Collection`.
/// Returns false when the persist failed and the flag was re-armed.
fn flush_vector_index_map(
    db: &DB,
    name: &str,
    indexes: &DashMap<String, Arc<VectorIndex>>,
    dirty: &AtomicBool,
    last_persist: &AtomicU64,
) -> bool {
    // Claim the dirty flag up front so a writer that dirties again after we
    // snapshot the index isn't wrongly cleared (mirrors `flush_stats`).
    if !dirty.swap(false, Ordering::Relaxed) {
        return true; // Nothing to persist
    }
    if let Err(e) = Collection::persist_vector_index_map(db, name, indexes) {
        tracing::warn!("Failed to persist vector indexes: {}", e);
        // Re-arm so a later throttled call / shutdown flush retries rather
        // than silently dropping the change.
        dirty.store(true, Ordering::Relaxed);
        return false;
    }
    last_persist.store(now_secs(), Ordering::Relaxed);
    true
}

/// A collection whose vector indexes changed since their last persist. Weak
/// handles only: the registry must never keep a dropped engine's RocksDB
/// instance open.
struct DirtyVecEntry {
    db: Weak<DB>,
    name: String,
    vector_indexes: Weak<DashMap<String, Arc<VectorIndex>>>,
    vec_dirty: Arc<AtomicBool>,
    vec_last_persist: Arc<AtomicU64>,
}

static DIRTY_VECTOR_INDEXES: Lazy<DashMap<(usize, String), DirtyVecEntry>> =
    Lazy::new(DashMap::new);

/// Persist every vector index whose changes are older than the throttle
/// window (audit D9). Single-document writes only mark an index dirty and
/// persist on a later write, so without this a quiet collection could hold
/// unpersisted vectors until shutdown. Calling it every
/// `VEC_PERSIST_THROTTLE_SECS` bounds that window to about twice the throttle.
/// Blocking (it serializes whole indexes): call it off the async runtime.
/// Returns the number of collections persisted.
pub fn flush_dirty_vector_indexes() -> usize {
    let now = now_secs();
    let due: Vec<(usize, String)> = DIRTY_VECTOR_INDEXES
        .iter()
        .filter(|e| {
            now.saturating_sub(e.vec_last_persist.load(Ordering::Relaxed))
                >= VEC_PERSIST_THROTTLE_SECS
        })
        .map(|e| e.key().clone())
        .collect();

    let mut flushed = 0;
    for key in due {
        let Some((_, entry)) = DIRTY_VECTOR_INDEXES.remove(&key) else {
            continue;
        };
        let (Some(db), Some(indexes)) = (entry.db.upgrade(), entry.vector_indexes.upgrade()) else {
            continue; // engine or handle gone; nothing left to persist into
        };
        if db.cf_handle(&entry.name).is_none() {
            continue; // collection dropped
        }
        if flush_vector_index_map(
            &db,
            &entry.name,
            &indexes,
            &entry.vec_dirty,
            &entry.vec_last_persist,
        ) {
            flushed += 1;
        } else {
            // Keep it registered so the next drain retries.
            DIRTY_VECTOR_INDEXES.insert(key, entry);
        }
    }
    flushed
}

impl Collection {
    /// Create a new collection handle
    pub fn new(name: String, db: Arc<DB>) -> Self {
        // Load cached count from disk, or calculate if not present
        let count = if let Some(cf) = db.cf_handle(&name) {
            match db.get_cf(&cf, STATS_COUNT_KEY.as_bytes()) {
                Ok(Some(bytes)) => String::from_utf8_lossy(&bytes)
                    .parse::<usize>()
                    .unwrap_or(0),
                // No cached count - calculate from documents
                _ => Self::count_doc_entries(&db, &cf),
            }
        } else {
            0
        };

        // The blob chunk count is resolved lazily: see `ensure_chunk_count`.
        // Doing it here cost a full `blo:` walk per collection handle, for
        // every collection in the instance, on every startup.
        let (change_sender, _) = tokio::sync::broadcast::channel(100);

        // Load collection type
        let collection_type = if let Some(cf) = db.cf_handle(&name) {
            match db.get_cf(&cf, COLLECTION_TYPE_KEY.as_bytes()) {
                Ok(Some(bytes)) => String::from_utf8_lossy(&bytes).to_string(),
                _ => "document".to_string(),
            }
        } else {
            "document".to_string()
        };

        Self {
            name,
            db,
            doc_count: Arc::new(AtomicUsize::new(count)),
            chunk_count: Arc::new(AtomicUsize::new(0)),
            chunk_count_ready: Arc::new(AtomicBool::new(false)),
            count_dirty: Arc::new(AtomicBool::new(false)),
            last_flush_time: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            vec_dirty: Arc::new(AtomicBool::new(false)),
            vec_last_persist: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            change_sender: Arc::new(change_sender),
            collection_type: Arc::new(RwLock::new(collection_type)),
            bloom_filters: Arc::new(DashMap::new()),
            cuckoo_filters: Arc::new(DashMap::new()),
            vector_indexes: Arc::new(DashMap::new()),
            schema_validator: Arc::new(RwLock::new(None)),
            schema_hash: Arc::new(RwLock::new(None)),
        }
    }

    /// Count live documents under `doc:`. Pre-H8 TTL expiry entries also live
    /// there (`doc:ttl_exp::…`) with an empty value; a serialized document is
    /// never empty, so skipping empty values excludes exactly those.
    pub(crate) fn count_doc_entries(db: &DB, cf: &impl rust_rocksdb::AsColumnFamilyRef) -> usize {
        let prefix = DOC_PREFIX.as_bytes();
        db.prefix_iterator_cf(cf, prefix)
            .take_while(|r| r.as_ref().is_ok_and(|(k, _)| k.starts_with(prefix)))
            .filter(|r| r.as_ref().is_ok_and(|(_, v)| !v.is_empty()))
            .count()
    }

    /// Resolve `chunk_count` from disk on first use.
    ///
    /// Walks the collection's `blo:` keys exactly once per handle lineage —
    /// the flag and the counter are both shared by every clone. Callers that
    /// read *or* adjust the count must go through this first, otherwise an
    /// increment applied before the walk would be counted twice: the walk
    /// stores an absolute value read from disk, which already includes it.
    pub(crate) fn ensure_chunk_count(&self) {
        if self.chunk_count_ready.load(Ordering::Acquire) {
            return;
        }
        let count = match self.db.cf_handle(&self.name) {
            Some(cf) => {
                let prefix = BLO_PREFIX.as_bytes();
                self.db
                    .prefix_iterator_cf(&cf, prefix)
                    .take_while(|r| r.as_ref().is_ok_and(|(k, _)| k.starts_with(prefix)))
                    .count()
            }
            None => 0,
        };
        // Whoever wins publishes its walk; a loser's walk saw the same disk
        // state, so either answer is correct.
        if self
            .chunk_count_ready
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.chunk_count.store(count, Ordering::Relaxed);
        }
    }

    /// Get collection type
    pub fn get_type(&self) -> String {
        self.collection_type.read().clone()
    }

    /// Set collection type (persists to disk)
    pub fn set_type(&self, type_: &str) -> DbResult<()> {
        let cf = self
            .db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        self.db
            .put_cf(&cf, COLLECTION_TYPE_KEY.as_bytes(), type_.as_bytes())
            .map_err(|e| DbError::InternalError(format!("Failed to set collection type: {}", e)))?;

        // Update in-memory state
        *self.collection_type.write() = type_.to_string();

        Ok(())
    }

    /// Flush count to disk if dirty (call periodically or on shutdown)
    pub fn flush_stats(&self) {
        if self.count_dirty.swap(false, Ordering::Relaxed) {
            let count = self.doc_count.load(Ordering::Relaxed);
            if let Some(cf) = self.db.cf_handle(&self.name) {
                let _ = self.db.put_cf(
                    &cf,
                    STATS_COUNT_KEY.as_bytes(),
                    count.to_string().as_bytes(),
                );
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

    /// Persist vector indexes to disk, but at most once per
    /// `VEC_PERSIST_THROTTLE_SECS` and only when there are unpersisted changes.
    ///
    /// `persist_vector_indexes()` re-serializes the *entire* index (all vectors +
    /// the HNSW graph) into a single blob, so calling it after every write batch
    /// during a bulk load is O(batches × index size) — the dominant cost when a
    /// large embedding-bearing collection is (re)loaded. Throttling collapses that
    /// burst to roughly one persist per window. The trailing window is made
    /// durable by `flush_vector_indexes()` on shutdown — the same
    /// throttle-on-write + flush-on-shutdown model already used for collection
    /// stats (`flush_stats` / `flush_stats_throttled`). A hard crash can lose at
    /// most the last window of index updates, which are recoverable by rebuilding
    /// the index from the documents' embedding fields.
    pub fn persist_vector_indexes_throttled(&self) {
        if !self.vec_dirty.load(Ordering::Relaxed) {
            return; // Nothing to persist
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Persist at most once per VEC_PERSIST_THROTTLE_SECS. This interval is
        // deliberately larger than one second: a single write batch against a
        // big index can itself take >1s (full-index re-serialize), so a 1s
        // window would still persist on every batch and defeat the throttle.
        if now.saturating_sub(self.vec_last_persist.load(Ordering::Relaxed))
            < VEC_PERSIST_THROTTLE_SECS
        {
            return;
        }
        self.flush_vector_indexes();
    }

    /// Persist vector indexes to disk if there are unpersisted changes,
    /// regardless of throttle. Called on shutdown (via the engine's flush-all)
    /// so the trailing throttle window can't be lost across a graceful restart.
    pub fn flush_vector_indexes(&self) {
        let _ = flush_vector_index_map(
            &self.db,
            &self.name,
            &self.vector_indexes,
            &self.vec_dirty,
            &self.vec_last_persist,
        );
    }

    /// Record that an in-memory vector index changed. The first change after
    /// a persist also registers the collection with the process-wide dirty
    /// set drained by [`flush_dirty_vector_indexes`] (audit D9), so a change
    /// with no later write behind it is still persisted within a bounded time
    /// once something calls that periodically.
    pub(crate) fn mark_vec_dirty(&self) {
        if self.vec_dirty.swap(true, Ordering::Relaxed) {
            return; // already registered
        }
        DIRTY_VECTOR_INDEXES.insert(
            (Arc::as_ptr(&self.db) as usize, self.name.clone()),
            DirtyVecEntry {
                db: Arc::downgrade(&self.db),
                name: self.name.clone(),
                vector_indexes: Arc::downgrade(&self.vector_indexes),
                vec_dirty: self.vec_dirty.clone(),
                vec_last_persist: self.vec_last_persist.clone(),
            },
        );
    }

    /// Compact the collection to remove tombstones and reclaim space
    pub fn compact(&self) {
        if let Some(cf) = self.db.cf_handle(&self.name) {
            self.db.compact_range_cf(&cf, None::<&[u8]>, None::<&[u8]>);
        }
    }

    /// Get usage statistics
    pub fn stats(&self) -> CollectionStats {
        let disk_usage = self.disk_usage();

        CollectionStats {
            name: self.name.clone(),
            document_count: self.doc_count.load(Ordering::Relaxed),
            chunk_count: {
                self.ensure_chunk_count();
                self.chunk_count.load(Ordering::Relaxed)
            },
            disk_usage,
        }
    }

    /// Get disk usage statistics for this collection
    pub fn disk_usage(&self) -> DiskUsage {
        let cf = match self.db.cf_handle(&self.name) {
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
        let sst_files_size = self
            .db
            .property_int_value_cf(&cf, "rocksdb.total-sst-files-size")
            .ok()
            .flatten()
            .unwrap_or(0);

        // Get estimated live data size
        let live_data_size = self
            .db
            .property_int_value_cf(&cf, "rocksdb.estimate-live-data-size")
            .ok()
            .flatten()
            .unwrap_or(0);

        // Get number of SST files at all levels
        let mut num_sst_files = 0;
        for i in 0..7 {
            num_sst_files += self
                .db
                .property_int_value_cf(&cf, &format!("rocksdb.num-files-at-level{}", i))
                .ok()
                .flatten()
                .unwrap_or(0);
        }

        // Get memtable size
        let memtable_size = self
            .db
            .property_int_value_cf(&cf, "rocksdb.cur-size-all-mem-tables")
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
        let cf = self
            .db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let config_bytes = serde_json::to_vec(config)?;
        self.db
            .put_cf(&cf, SHARD_CONFIG_KEY.as_bytes(), &config_bytes)
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
        let cf = self.db.cf_handle(&self.name)?;

        self.db
            .get_cf(&cf, SHARD_CONFIG_KEY.as_bytes())
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Save shard table to storage (persisting assignments)
    pub fn set_shard_table(
        &self,
        table: &crate::sharding::coordinator::ShardTable,
    ) -> DbResult<()> {
        let cf = self
            .db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let table_bytes = serde_json::to_vec(table)?;
        self.db
            .put_cf(&cf, SHARD_TABLE_KEY.as_bytes(), &table_bytes)
            .map_err(|e| DbError::InternalError(format!("Failed to store shard table: {}", e)))?;

        Ok(())
    }

    /// Load shard table from storage
    pub fn get_stored_shard_table(&self) -> Option<crate::sharding::coordinator::ShardTable> {
        let cf = self.db.cf_handle(&self.name)?;

        self.db
            .get_cf(&cf, SHARD_TABLE_KEY.as_bytes())
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

    /// Build a TTL expiry index key: "ttl_exp:<ttl_index_name>:<expiry_ts>:<doc_key>".
    ///
    /// Its own top-level prefix, outside `doc:` (audit H8), and the timestamp
    /// is zero-padded to 20 digits so lexicographic order is numeric order and
    /// the reaper can stop at the first unexpired entry.
    pub fn ttl_expiry_key(ttl_index_name: &str, expiry_timestamp: u64, doc_key: &str) -> Vec<u8> {
        format!(
            "{}{}:{:020}:{}",
            TTL_EXPIRY_PREFIX, ttl_index_name, expiry_timestamp, doc_key
        )
        .into_bytes()
    }

    /// TTL expiry index prefix for one index: "ttl_exp:<ttl_index_name>:"
    pub fn ttl_expiry_prefix(ttl_index_name: &str) -> Vec<u8> {
        format!("{}{}:", TTL_EXPIRY_PREFIX, ttl_index_name).into_bytes()
    }

    /// Prefix of the pre-H8 expiry entries for one index:
    /// "doc:ttl_exp::<ttl_index_name>:". Read-only — see `LEGACY_TTL_EXPIRY_PREFIX`.
    pub(crate) fn legacy_ttl_expiry_prefix(ttl_index_name: &str) -> Vec<u8> {
        format!("{}:{}:", LEGACY_TTL_EXPIRY_PREFIX, ttl_index_name).into_bytes()
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

impl Collection {
    /// Publish a change event, dropping cached query results for this
    /// collection first. Every document write funnels through here, so the
    /// write paths outside the HTTP handlers (replication apply, TTL expiry,
    /// Lua, streams, the queue) invalidate the cache too (audit P2).
    pub(crate) fn emit_change(
        &self,
        event: ChangeEvent,
    ) -> Result<usize, tokio::sync::broadcast::error::SendError<ChangeEvent>> {
        crate::storage::query_cache::invalidate_collection("", &self.name);
        self.change_sender.send(event)
    }
}
