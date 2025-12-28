use rocksdb::DB;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use uuid::Uuid;

use super::document::Document;
use super::geo::{GeoIndex, GeoIndexStats};
use super::index::{
    extract_field_value, generate_ngrams, levenshtein_distance, tokenize, FulltextMatch, Index,
    IndexStats, IndexType, TtlIndex, TtlIndexStats, NGRAM_SIZE,
};
use crate::error::{DbError, DbResult};

/// Key prefixes for different data types
const DOC_PREFIX: &str = "doc:";
const IDX_PREFIX: &str = "idx:";
const IDX_META_PREFIX: &str = "idx_meta:";
const GEO_PREFIX: &str = "geo:";
const GEO_META_PREFIX: &str = "geo_meta:";
const FT_PREFIX: &str = "ft:"; // Fulltext n-gram entries
const FT_META_PREFIX: &str = "ft_meta:"; // Fulltext index metadata
const FT_TERM_PREFIX: &str = "ft_term:"; // Fulltext term → doc mapping
const STATS_COUNT_KEY: &str = "_stats:count"; // Document count
const SHARD_CONFIG_KEY: &str = "_stats:shard_config"; // Sharding configuration
const SHARD_TABLE_KEY: &str = "_stats:shard_table";   // Sharding assignment table
const COLLECTION_TYPE_KEY: &str = "_stats:type"; // Collection type (document, edge)
const BLO_PREFIX: &str = "blo:"; // Blob chunk prefix
const TTL_META_PREFIX: &str = "ttl_meta:"; // TTL index metadata

/// Type of change event
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChangeType {
    Insert,
    Update,
    Delete,
}

/// Real-time change event
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChangeEvent {
    #[serde(rename = "type")]
    pub type_: ChangeType,
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_data: Option<Value>,
}
/// Fulltext index metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FulltextIndex {
    name: String,
    #[serde(alias = "field", deserialize_with = "crate::storage::index::deserialize_fields")]
    fields: Vec<String>,
    #[serde(default = "default_min_length")]
    min_length: usize,
}

fn default_min_length() -> usize {
    3
}

/// Collection statistics including disk usage
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CollectionStats {
    pub name: String,
    pub document_count: usize,
    pub chunk_count: usize,
    pub disk_usage: DiskUsage,
}



/// Disk usage statistics for a collection
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiskUsage {
    /// Total size of SST files in bytes
    pub sst_files_size: u64,
    /// Estimated live data size in bytes
    pub live_data_size: u64,
    /// Number of SST files
    pub num_sst_files: u64,
    /// Size of memtables in bytes
    pub memtable_size: u64,
}

/// Represents a collection of documents backed by RocksDB
pub struct Collection {
    /// Collection name (column family name)
    pub name: String,
    /// RocksDB instance
    db: Arc<RwLock<DB>>,
    /// Cached document count (atomic for lock-free updates)
    doc_count: Arc<AtomicUsize>,
    /// Cached blob chunk count (atomic for lock-free updates)
    chunk_count: Arc<AtomicUsize>,
    /// Whether count needs to be persisted to disk
    count_dirty: Arc<AtomicBool>,
    /// Last flush time in seconds since UNIX epoch (for throttling)
    last_flush_time: Arc<std::sync::atomic::AtomicU64>,
    /// Broadcast channel for real-time change events
    pub change_sender: Arc<tokio::sync::broadcast::Sender<ChangeEvent>>,
    /// Collection type (document, edge, blob)
    pub collection_type: Arc<RwLock<String>>,
}

impl Clone for Collection {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            db: self.db.clone(),
            doc_count: self.doc_count.clone(),
            chunk_count: self.chunk_count.clone(),
            count_dirty: self.count_dirty.clone(),
            last_flush_time: self.last_flush_time.clone(),
            change_sender: self.change_sender.clone(),
            collection_type: self.collection_type.clone(),
        }
    }
}

impl std::fmt::Debug for Collection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collection")
            .field("name", &self.name)
            .finish()
    }
}

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
                                r.as_ref().map_or(false, |(k, _)| k.starts_with(prefix))
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
                    .take_while(|r| {
                         r.as_ref().map_or(false, |(k, _)| k.starts_with(prefix))
                    })
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

    /// Validate edge document has required _from and _to fields
    fn validate_edge_document(&self, data: &Value) -> DbResult<()> {
        let obj = data.as_object().ok_or_else(|| {
            DbError::InvalidDocument("Edge document must be a JSON object".to_string())
        })?;

        // Check _from field
        match obj.get("_from") {
            Some(Value::String(s)) if !s.is_empty() => {},
            Some(Value::String(_)) => {
                return Err(DbError::InvalidDocument(
                    "Edge document _from field must be a non-empty string".to_string()
                ));
            }
            Some(_) => {
                return Err(DbError::InvalidDocument(
                    "Edge document _from field must be a string".to_string()
                ));
            }
            None => {
                return Err(DbError::InvalidDocument(
                    "Edge document requires _from field".to_string()
                ));
            }
        }

        // Check _to field
        match obj.get("_to") {
            Some(Value::String(s)) if !s.is_empty() => {},
            Some(Value::String(_)) => {
                return Err(DbError::InvalidDocument(
                    "Edge document _to field must be a non-empty string".to_string()
                ));
            }
            Some(_) => {
                return Err(DbError::InvalidDocument(
                    "Edge document _to field must be a string".to_string()
                ));
            }
            None => {
                return Err(DbError::InvalidDocument(
                    "Edge document requires _to field".to_string()
                ));
            }
        }

        Ok(())
    }

    /// Build a document key
    fn doc_key(key: &str) -> Vec<u8> {
        format!("{}{}", DOC_PREFIX, key).into_bytes()
    }

    /// Build a blob chunk key
    fn blo_chunk_key(key: &str, chunk_index: u32) -> Vec<u8> {
        format!("{}{}:{:010}", BLO_PREFIX, key, chunk_index).into_bytes()
    }

    /// Build an index metadata key
    fn idx_meta_key(index_name: &str) -> Vec<u8> {
        format!("{}{}", IDX_META_PREFIX, index_name).into_bytes()
    }

    /// Build an index entry key
    /// Build an index entry key
    fn idx_entry_key(index_name: &str, values: &[Value], doc_key: &str) -> Vec<u8> {
        let mut encoded = Vec::new();
        for value in values {
            encoded.extend(crate::storage::codec::encode_key(value));
        }
        let hex_value = hex::encode(encoded);
        format!("{}{}:{}:{}", IDX_PREFIX, index_name, hex_value, doc_key).into_bytes()
    }

    /// Build a geo index metadata key
    fn geo_meta_key(index_name: &str) -> Vec<u8> {
        format!("{}{}", GEO_META_PREFIX, index_name).into_bytes()
    }

    /// Build a geo index entry key
    fn geo_entry_key(index_name: &str, doc_key: &str) -> Vec<u8> {
        format!("{}{}:{}", GEO_PREFIX, index_name, doc_key).into_bytes()
    }

    /// Build a fulltext index metadata key
    fn ft_meta_key(index_name: &str) -> Vec<u8> {
        format!("{}{}", FT_META_PREFIX, index_name).into_bytes()
    }

    /// Build a fulltext n-gram entry key (ngram → doc_key)
    fn ft_ngram_key(index_name: &str, ngram: &str, doc_key: &str) -> Vec<u8> {
        format!("{}{}:{}:{}", FT_PREFIX, index_name, ngram, doc_key).into_bytes()
    }

    /// Build a fulltext term entry key (term → doc_key with position)
    fn ft_term_key(index_name: &str, term: &str, doc_key: &str) -> Vec<u8> {
        format!("{}{}:{}:{}", FT_TERM_PREFIX, index_name, term, doc_key).into_bytes()
    }

    // ==================== Blob Operations ====================

    /// Store a blob chunk
    pub fn put_blob_chunk(&self, key: &str, chunk_index: u32, data: &[u8]) -> DbResult<()> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        
        // Optimize: Check existence first to only increment on new chunks?
        // Or assume overwrite is rare for chunks (immutable mostly).
        // Let's check first to be accurate.
        let chunk_key = Self::blo_chunk_key(key, chunk_index);
        let exists = db.get_pinned_cf(cf, &chunk_key).ok().flatten().is_some();

        db.put_cf(cf, chunk_key, data)
            .map_err(|e| DbError::InternalError(format!("Failed to put blob chunk: {}", e)))?;
            
        if !exists {
            self.chunk_count.fetch_add(1, Ordering::Relaxed);
        }

        Ok(())
    }

    /// Get a blob chunk
    pub fn get_blob_chunk(&self, key: &str, chunk_index: u32) -> DbResult<Option<Vec<u8>>> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        
        let res = db.get_cf(cf, Self::blo_chunk_key(key, chunk_index))
            .map_err(|e| DbError::InternalError(format!("Failed to get blob chunk: {}", e)))?;
            
        Ok(res)
    }

    /// Delete all chunks for a blob
    pub fn delete_blob_data(&self, key: &str) -> DbResult<()> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // The prefix for all chunks of this blob is "blo:<key>:"
        let prefix = format!("{}{}:", BLO_PREFIX, key);
        let prefix_bytes = prefix.as_bytes();

        let iter = db.iterator_cf(cf, rocksdb::IteratorMode::From(prefix_bytes, rocksdb::Direction::Forward));
        let mut deleted_count = 0;
        
        for item in iter {
             if let Ok((k, _)) = item {
                 if k.starts_with(prefix_bytes) {
                     db.delete_cf(cf, k)
                        .map_err(|e| DbError::InternalError(format!("Failed to delete blob chunk: {}", e)))?;
                     deleted_count += 1;
                 } else {
                     break;
                 }
             }
        }

        if deleted_count > 0 {
            self.chunk_count.fetch_sub(deleted_count, Ordering::Relaxed);
        }

        Ok(())
    }

    /// Prune documents older than a specified timestamp (Timeseries only)
    pub fn prune_older_than(&self, timestamp_ms: u64) -> DbResult<usize> {
        // Enforce collection type
        if *self.collection_type.read().unwrap() != "timeseries" {
             return Err(DbError::OperationNotSupported("Pruning only supported on timeseries collections".to_string()));
        }

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        // Construct End Key (UUIDv7-compatible lower bound from timestamp)
        // UUIDv7: 48 bits timestamp at the top.
        // We construct a 128-bit integer where the top 48 bits are the timestamp.
        let end_uuid_int = (timestamp_ms as u128) << 80;
        let end_uuid = Uuid::from_u128(end_uuid_int);
        // We construct the key string
        let end_key_str = end_uuid.to_string();
        
        let start_key = Self::doc_key("");
        let end_key = Self::doc_key(&end_key_str);
        
        // Count items to be deleted (scanning keys only)
        // Optimization: For massive retention policies, this scan might be costly.
        // But maintaining accurate doc_count is important.
        let iter = db.iterator_cf(
            cf, 
            rocksdb::IteratorMode::From(&start_key, rocksdb::Direction::Forward)
        );
        
        let mut count = 0;
        
        for item in iter {
             match item {
                 Ok((k, _)) => {
                     // Check if we are still within "doc:" prefix
                     if !k.starts_with(&start_key[0..4]) {
                         break; 
                     }
                     // Check if we reached the end key
                     // Lexicographical comparison of bytes matches UUID comparison
                     if k.as_ref() >= end_key.as_slice() {
                         break;
                     }
                     count += 1;
                 },
                 Err(_) => break,
             }
        }
        
        if count > 0 {
             db.delete_range_cf(cf, &start_key, &end_key)
                 .map_err(|e| DbError::InternalError(format!("Prune failed: {}", e)))?;
                 
             self.doc_count.fetch_sub(count, Ordering::Relaxed);
             self.count_dirty.store(true, Ordering::Relaxed);
        }
        
        Ok(count)
    }

    // ==================== Document Operations ====================

    /// Insert a document into the collection
    pub fn insert(&self, data: Value) -> DbResult<Document> {
        self.insert_internal(data, true)
    }

    /// Insert a document without updating indexes (for bulk imports)
    /// Call rebuild_all_indexes() after bulk import is complete
    pub fn insert_no_index(&self, data: Value) -> DbResult<Document> {
        self.insert_internal(data, false)
    }

    /// Batch insert multiple documents without indexes (fastest for bulk imports)
    /// Uses RocksDB WriteBatch for optimal performance
    /// Returns the list of inserted documents for subsequent indexing
    pub fn insert_batch(&self, documents: Vec<Value>) -> DbResult<Vec<Document>> {
        use rocksdb::WriteBatch;

        let total_docs = documents.len();
        let prep_start = std::time::Instant::now();

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut batch = WriteBatch::default();
        let mut inserted_docs = Vec::with_capacity(total_docs);

        for mut data in documents {
            // Extract or generate key
            let key = if let Some(obj) = data.as_object_mut() {
                if let Some(key_value) = obj.remove("_key") {
                    if let Some(key_str) = key_value.as_str() {
                        key_str.to_string()
                    } else {
                        continue; // Skip invalid documents
                    }
                } else {
                    uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
                }
            } else {
                uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
            };

            let doc = Document::with_key(&self.name, key.clone(), data);
            if let Ok(doc_bytes) = serde_json::to_vec(&doc) {
                batch.put_cf(cf, Self::doc_key(&key), &doc_bytes);
                inserted_docs.push(doc);
            }
        }

        let count = inserted_docs.len();
        let prep_time = prep_start.elapsed();
        tracing::debug!("insert_batch: Prepared {} docs in {:?}", count, prep_time);

        // Write all documents in one batch operation
        let write_start = std::time::Instant::now();
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch insert: {}", e)))?;
        let write_time = write_start.elapsed();
        tracing::debug!("insert_batch: RocksDB write took {:?}", write_time);

        // Update document count
        self.doc_count
            .fetch_add(count, std::sync::atomic::Ordering::Relaxed);
        self.count_dirty
            .store(true, std::sync::atomic::Ordering::Relaxed);

        Ok(inserted_docs)
    }

    /// Batch upsert (insert or update) multiple documents - optimized for replication
    /// Uses RocksDB WriteBatch for optimal performance
    /// Skips index updates for speed - caller should rebuild indexes if needed
    pub fn upsert_batch(&self, documents: Vec<(String, Value)>) -> DbResult<usize> {
        use rocksdb::WriteBatch;

        if documents.is_empty() {
            return Ok(0);
        }

        if *self.collection_type.read().unwrap() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Upsert (update) operations are not allowed on timeseries collections. Use insert_batch instead.".to_string(),
            ));
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut batch = WriteBatch::default();
        let mut insert_count = 0;

        for (key, mut data) in documents {
            // Ensure _key is set
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_key".to_string(), Value::String(key.clone()));
            }

            // Check if document exists to determine insert vs update
            let exists = db.get_cf(cf, Self::doc_key(&key)).ok().flatten().is_some();
            
            let doc = if exists {
                // Update existing document
                if let Ok(bytes) = db.get_cf(cf, Self::doc_key(&key)) {
                    if let Some(bytes) = bytes {
                        if let Ok(mut existing) = serde_json::from_slice::<Document>(&bytes) {
                            existing.update(data);
                            existing
                        } else {
                            Document::with_key(&self.name, key.clone(), data)
                        }
                    } else {
                        Document::with_key(&self.name, key.clone(), data)
                    }
                } else {
                    Document::with_key(&self.name, key.clone(), data)
                }
            } else {
                Document::with_key(&self.name, key.clone(), data)
            };

            if let Ok(doc_bytes) = serde_json::to_vec(&doc) {
                batch.put_cf(cf, Self::doc_key(&key), &doc_bytes);
                if !exists {
                    insert_count += 1;
                }
            }
        }

        let count = batch.len();
        
        // Write all documents in one batch operation
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch upsert: {}", e)))?;

        // Update document count (only for new inserts)
        if insert_count > 0 {
            self.doc_count.fetch_add(insert_count, std::sync::atomic::Ordering::Relaxed);
            self.count_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        Ok(count)
    }

    /// Index only the provided documents (for incremental indexing after batch insert)
    pub fn index_documents(&self, docs: &[Document]) -> DbResult<usize> {
        use rocksdb::WriteBatch;

        let total_start = std::time::Instant::now();

        let indexes = self.get_all_indexes();
        let geo_indexes = self.get_all_geo_indexes();
        let ft_indexes = self.get_all_fulltext_indexes();

        tracing::info!(
            "index_documents: Indexing {} docs with {} regular, {} geo, {} fulltext indexes",
            docs.len(),
            indexes.len(),
            geo_indexes.len(),
            ft_indexes.len()
        );

        if indexes.is_empty() && geo_indexes.is_empty() && ft_indexes.is_empty() {
            return Ok(0);
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Build regular indexes
        if !indexes.is_empty() {
            let idx_start = std::time::Instant::now();
            let mut batch = WriteBatch::default();

            for doc in docs {
                let doc_value = doc.to_value();
                for index in &indexes {
                    let field_values: Vec<Value> = index
                        .fields
                        .iter()
                        .map(|f| extract_field_value(&doc_value, f))
                        .collect();
                    
                    // Only index if no field is null (strict match)?
                    // Or if at least one is not null?
                    // Legacy behavior: !field_value.is_null() meant skipping nulls.
                    // For compound: if ANY field is null, we usually index (null matching).
                    // BUT our `encode_key` handles Null.
                    // The legacy code explicitly skipped nulls to avoid indexing missing fields?
                    // Let's adopt a policy: Index if NOT ALL fields are null (Sparse-ish).
                    // Actually, to support "a=1" lookup even if "b" is missing, we must index.
                    // But if "a" is missing too?
                    // Let's skip only if ALL values are Null.
                    if !field_values.iter().all(|v| v.is_null()) {
                        let entry_key = Self::idx_entry_key(&index.name, &field_values, &doc.key);
                        batch.put_cf(cf, entry_key, doc.key.as_bytes());
                    }
                }
            }

            let _ = db.write(batch);
            tracing::info!(
                "index_documents: Regular indexes took {:?}",
                idx_start.elapsed()
            );
        }

        // Build geo indexes
        if !geo_indexes.is_empty() {
            let geo_start = std::time::Instant::now();
            let mut batch = WriteBatch::default();

            for doc in docs {
                let doc_value = doc.to_value();
                for geo_index in &geo_indexes {
                    let field_value = extract_field_value(&doc_value, &geo_index.field);
                    if !field_value.is_null() {
                        let entry_key = Self::geo_entry_key(&geo_index.name, &doc.key);
                        if let Ok(geo_data) = serde_json::to_vec(&field_value) {
                            batch.put_cf(cf, entry_key, &geo_data);
                        }
                    }
                }
            }

            let _ = db.write(batch);
            tracing::info!(
                "index_documents: Geo indexes took {:?}",
                geo_start.elapsed()
            );
        }

        // Build fulltext indexes
        if !ft_indexes.is_empty() {
            let ft_start = std::time::Instant::now();
            let mut batch = WriteBatch::default();

            for doc in docs {
                let doc_value = doc.to_value();
                for ft_index in &ft_indexes {
                    for field in &ft_index.fields {
                        let field_value = extract_field_value(&doc_value, field);
                        if let Some(text) = field_value.as_str() {
                            let terms = tokenize(text);
                            for term in &terms {
                                if term.len() >= ft_index.min_length {
                                    let term_key = Self::ft_term_key(&ft_index.name, term, &doc.key);
                                    batch.put_cf(cf, term_key, doc.key.as_bytes());
                                }
                            }

                            let ngrams = generate_ngrams(text, NGRAM_SIZE);
                            for ngram in &ngrams {
                                let ngram_key = Self::ft_ngram_key(&ft_index.name, ngram, &doc.key);
                                batch.put_cf(cf, ngram_key, doc.key.as_bytes());
                            }
                        }
                    }
                }
            }

            let _ = db.write(batch);
            tracing::info!(
                "index_documents: Fulltext indexes took {:?}",
                ft_start.elapsed()
            );
        }

        tracing::info!("index_documents: Total time {:?}", total_start.elapsed());
        Ok(docs.len())
    }

    /// Internal insert implementation
    fn insert_internal(&self, mut data: Value, update_indexes: bool) -> DbResult<Document> {
        // Validate edge documents
        if *self.collection_type.read().unwrap() == "edge" {
            self.validate_edge_document(&data)?;
        }

        // Extract or generate key
        let key = if let Some(obj) = data.as_object_mut() {
            if let Some(key_value) = obj.remove("_key") {
                if let Some(key_str) = key_value.as_str() {
                    key_str.to_string()
                } else {
                    return Err(DbError::InvalidDocument(
                        "_key must be a string".to_string(),
                    ));
                }
            } else {
                uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
            }
        } else {
            uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
        };

        let doc = Document::with_key(&self.name, key.clone(), data);
        let doc_value = doc.to_value();

        // Check unique constraints BEFORE saving the document (only if indexes are enabled)
        if update_indexes {
            self.check_unique_constraints(&key, &doc_value)?;
        }

        // Store document
        let doc_bytes = serde_json::to_vec(&doc)?;
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::doc_key(&key), &doc_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to insert document: {}", e)))?;
        }

        // Update indexes (if enabled)
        if update_indexes {
            self.update_indexes_on_insert(&key, &doc_value)?;
            self.update_fulltext_on_insert(&key, &doc_value)?;
        }

        // Update document count
        self.increment_count();

        // Broadcast change event
        let _ = self.change_sender.send(ChangeEvent {
            type_: ChangeType::Insert,
            key: key.clone(),
            data: Some(doc_value),
            old_data: None,
        });

        Ok(doc)
    }

    /// Get a document by key
    pub fn get(&self, key: &str) -> DbResult<Document> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let bytes = db
            .get_cf(cf, Self::doc_key(key))
            .map_err(|e| DbError::InternalError(format!("Failed to get document: {}", e)))?
            .ok_or_else(|| DbError::DocumentNotFound(key.to_string()))?;

        let doc: Document = serde_json::from_slice(&bytes)?;
        Ok(doc)
    }

    /// Get multiple documents by keys
    pub fn get_many(&self, keys: &[String]) -> Vec<Document> {
        keys.iter().filter_map(|k| self.get(k).ok()).collect()
    }

    /// Update a document
    pub fn update(&self, key: &str, data: Value) -> DbResult<Document> {
        if *self.collection_type.read().unwrap() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Update operations are not allowed on timeseries collections".to_string(),
            ));
        }
        // Get old document for index updates
        let old_doc = self.get(key)?;
        let old_value = old_doc.to_value();

        // Create updated document
        let mut doc = old_doc;
        doc.update(data);
        let new_value = doc.to_value();

        // Validate edge documents after update
        if *self.collection_type.read().unwrap() == "edge" {
            self.validate_edge_document(&new_value)?;
        }

        let doc_bytes = serde_json::to_vec(&doc)?;

        // Store updated document
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::doc_key(key), &doc_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to update document: {}", e)))?;
        }

        // Update indexes
        self.update_indexes_on_update(key, &old_value, &new_value)?;

        // Update fulltext indexes (delete old, insert new)
        self.update_fulltext_on_delete(key, &old_value)?;
        self.update_fulltext_on_insert(key, &new_value)?;

        // Broadcast change event
        let _ = self.change_sender.send(ChangeEvent {
            type_: ChangeType::Update,
            key: key.to_string(),
            data: Some(new_value),
            old_data: Some(old_value),
        });

        Ok(doc)
    }

    /// Update a document with revision check (optimistic concurrency control)
    /// Returns error if the current revision doesn't match expected_rev
    pub fn update_with_rev(
        &self,
        key: &str,
        expected_rev: &str,
        data: Value,
    ) -> DbResult<Document> {
        if *self.collection_type.read().unwrap() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Update operations are not allowed on timeseries collections".to_string(),
            ));
        }
        // Get old document for index updates
        let old_doc = self.get(key)?;

        // Check revision matches
        if old_doc.revision() != expected_rev {
            return Err(DbError::ConflictError(format!(
                "Document '{}' has been modified. Expected revision '{}', but current is '{}'",
                key,
                expected_rev,
                old_doc.revision()
            )));
        }

        let old_value = old_doc.to_value();

        // Create updated document
        let mut doc = old_doc;
        doc.update(data);
        let new_value = doc.to_value();
        let doc_bytes = serde_json::to_vec(&doc)?;

        // Store updated document
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::doc_key(key), &doc_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to update document: {}", e)))?;
        }

        // Update indexes
        self.update_indexes_on_update(key, &old_value, &new_value)?;

        // Update fulltext indexes (delete old, insert new)
        self.update_fulltext_on_delete(key, &old_value)?;
        self.update_fulltext_on_insert(key, &new_value)?;

        // Broadcast change event
        let _ = self.change_sender.send(ChangeEvent {
            type_: ChangeType::Update,
            key: key.to_string(),
            data: Some(new_value),
            old_data: Some(old_value),
        });

        Ok(doc)
    }



    /// Delete a document
    pub fn delete(&self, key: &str) -> DbResult<()> {
        // Get document for index cleanup
        let doc = self.get(key)?;
        let doc_value = doc.to_value();

        // Delete document
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.delete_cf(cf, Self::doc_key(key))
                .map_err(|e| DbError::InternalError(format!("Failed to delete document: {}", e)))?;
        }

        // If blob collection, delete chunks
        if *self.collection_type.read().unwrap() == "blob" {
            self.delete_blob_data(key)?;
        }

        // Update indexes
        self.update_indexes_on_delete(key, &doc_value)?;

        // Update fulltext indexes
        self.update_fulltext_on_delete(key, &doc_value)?;

        // Update document count
        self.decrement_count();

        // Broadcast change event
        let _ = self.change_sender.send(ChangeEvent {
            type_: ChangeType::Delete,
            key: key.to_string(),
            data: None,
            old_data: Some(doc_value),
        });

        Ok(())
    }

    /// Batch delete multiple documents
    /// Uses RocksDB WriteBatch for storage efficiency
    /// Updates indexes for each document (sequential)
    pub fn delete_batch(&self, keys: &[String]) -> DbResult<usize> {
        use rocksdb::WriteBatch;

        if keys.is_empty() {
            return Ok(0);
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut batch = WriteBatch::default();
        let mut deleted_count = 0;
        let mut deleted_docs = Vec::new();

        // 1. Prepare batch and handle auxiliary updates (indexes, blobs)
        for key in keys {
            // Get document first (needed for index cleanup)
            if let Ok(Some(bytes)) = db.get_cf(cf, Self::doc_key(key)) {
                if let Ok(doc) = serde_json::from_slice::<Document>(&bytes) {
                    let doc_value = doc.to_value();
                    
                    // Add to batch
                    batch.delete_cf(cf, Self::doc_key(key));
                    
                    // Handle blobs
                    if *self.collection_type.read().unwrap() == "blob" {
                        // This might fail partial batch if error? 
                        // But blob chunks are separate keys.
                        // We can call delete_blob_data (which does its own deletes).
                        // Note: delete_blob_data is not batched in the same batch here.
                        let _ = self.delete_blob_data(key);
                    }
                    
                    // Update indexes (Note: these are separate writes currently)
                    //Ideally indexes would support batching too, but for now we iterate
                    if let Err(e) = self.update_indexes_on_delete(key, &doc_value) {
                        tracing::warn!("Failed to clean indexes for {}: {}", key, e);
                    }
                    if let Err(e) = self.update_fulltext_on_delete(key, &doc_value) {
                         tracing::warn!("Failed to clean fulltext for {}: {}", key, e);
                    }
                    
                    deleted_docs.push((key.clone(), doc_value));
                    deleted_count += 1;
                }
            }
        }
        
        if deleted_count == 0 {
            return Ok(0);
        }

        // 2. Commit storage batch
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch delete: {}", e)))?;

        // 3. Update count
        self.doc_count
            .fetch_sub(deleted_count, std::sync::atomic::Ordering::Relaxed);
        self.count_dirty
            .store(true, std::sync::atomic::Ordering::Relaxed);
            
        // 4. Send Change Events
        for (key, old_data) in deleted_docs {
            let _ = self.change_sender.send(ChangeEvent {
                type_: ChangeType::Delete,
                key: key,
                data: None,
                old_data: Some(old_data),
            });
        }
        
        Ok(deleted_count)
    }

    // ==================== Transactional Document Operations ====================

    /// Insert a document within a transaction (deferred until commit)
    pub fn insert_tx(
        &self,
        tx: &mut crate::transaction::Transaction,
        wal: &crate::transaction::wal::WalWriter,
        data: Value,
    ) -> DbResult<Document> {
        // Extract or generate key
        let mut data = data;
        let key = if let Some(obj) = data.as_object_mut() {
            if let Some(key_value) = obj.remove("_key") {
                if let Some(key_str) = key_value.as_str() {
                    key_str.to_string()
                } else {
                    return Err(DbError::InvalidDocument(
                        "_key must be a string".to_string(),
                    ));
                }
            } else {
                uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
            }
        } else {
            uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
        };

        // Create document
        let doc = Document::with_key(&self.name, key.clone(), data.clone());

        // Check unique constraints before adding to transaction
        let doc_value = doc.to_value();
        self.check_unique_constraints(&key, &doc_value)?;

        // Extract database name from collection name (format: "db:collection")
        let (database, collection) = if let Some((db, coll)) = self.name.split_once(':') {
            (db.to_string(), coll.to_string())
        } else {
            ("_system".to_string(), self.name.clone())
        };

        // Add operation to transaction
        let operation = crate::transaction::Operation::Insert {
            database,
            collection,
            key: key.clone(),
            data: data.clone(),
        };

        // Write to WAL immediately for durability
        wal.write_operation(tx.id, operation.clone())?;

        // Track operation in transaction for commit/rollback
        tx.add_operation(operation);

        Ok(doc)
    }

    /// Update a document within a transaction (deferred until commit)
    pub fn update_tx(
        &self,
        tx: &mut crate::transaction::Transaction,
        wal: &crate::transaction::wal::WalWriter,
        key: &str,
        data: Value,
    ) -> DbResult<Document> {
        // Get old document
        let old_doc = self.get(key)?;
        let old_value = old_doc.to_value();

        // Create updated document
        let mut doc = old_doc;
        doc.update(data.clone());
        let new_value = doc.to_value();

        // Extract database name from collection name
        let (database, collection) = if let Some((db, coll)) = self.name.split_once(':') {
            (db.to_string(), coll.to_string())
        } else {
            ("_system".to_string(), self.name.clone())
        };

        // Add operation to transaction
        let operation = crate::transaction::Operation::Update {
            database,
            collection,
            key: key.to_string(),
            old_data: old_value,
            new_data: new_value,
        };

        // Write to WAL immediately
        wal.write_operation(tx.id, operation.clone())?;

        // Track operation in transaction
        tx.add_operation(operation);

        Ok(doc)
    }

    /// Delete a document within a transaction (deferred until commit)
    pub fn delete_tx(
        &self,
        tx: &mut crate::transaction::Transaction,
        wal: &crate::transaction::wal::WalWriter,
        key: &str,
    ) -> DbResult<()> {
        // Get document before deleting
        let doc = self.get(key)?;
        let doc_value = doc.to_value();

        // Extract database name from collection name
        let (database, collection) = if let Some((db, coll)) = self.name.split_once(':') {
            (db.to_string(), coll.to_string())
        } else {
            ("_system".to_string(), self.name.clone())
        };

        // Add operation to transaction
        let operation = crate::transaction::Operation::Delete {
            database,
            collection,
            key: key.to_string(),
            old_data: doc_value,
        };

        // Write to WAL immediately
        wal.write_operation(tx.id, operation.clone())?;

        // Track operation in transaction
        tx.add_operation(operation);

        Ok(())
    }

    // ==================== Transaction Commit/Rollback Helpers ====================

    /// Apply operations from a committed transaction (called during commit)
    pub fn apply_transaction_operations(
        &self,
        operations: &[crate::transaction::Operation],
    ) -> DbResult<()> {
        use rocksdb::WriteBatch;

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .ok_or_else(|| DbError::CollectionNotFound(self.name.clone()))?;

        let mut batch = WriteBatch::default();
        let mut insert_count = 0;
        let mut delete_count = 0;

        for op in operations {
            match op {
                crate::transaction::Operation::Insert { key, data, .. } => {
                    // Check if document already exists (defensive against double-recovery)
                    if self.get(key).is_ok() {
                        tracing::warn!("Skipping duplicate insert of key {} during transaction recovery", key);
                        continue;
                    }
                    
                    let doc = Document::with_key(&self.name, key.clone(), data.clone());
                    let doc_bytes = serde_json::to_vec(&doc)?;
                    batch.put_cf(cf, Self::doc_key(key), &doc_bytes);
                    insert_count += 1;

                    // Update indexes for insert
                    let doc_value = doc.to_value();
                    self.update_indexes_on_insert(key, &doc_value)?;
                    self.update_fulltext_on_insert(key, &doc_value)?;

                    // Broadcast change event
                    let _ = self.change_sender.send(ChangeEvent {
                        type_: ChangeType::Insert,
                        key: key.clone(),
                        data: Some(doc_value),
                        old_data: None,
                    });
                }
                crate::transaction::Operation::Update {
                    key,
                    old_data,
                    new_data,
                    ..
                } => {
                    let doc = Document::with_key(&self.name, key.clone(), new_data.clone());
                    let doc_bytes = serde_json::to_vec(&doc)?;
                    batch.put_cf(cf, Self::doc_key(key), &doc_bytes);

                    // Update indexes for update
                    self.update_indexes_on_update(key, old_data, new_data)?;
                    self.update_fulltext_on_delete(key, old_data)?;
                    self.update_fulltext_on_insert(key, new_data)?;

                    // Broadcast change event
                    let _ = self.change_sender.send(ChangeEvent {
                        type_: ChangeType::Update,
                        key: key.clone(),
                        data: Some(new_data.clone()),
                        old_data: Some(old_data.clone()),
                    });
                }
                crate::transaction::Operation::Delete { key, old_data, .. } => {
                    batch.delete_cf(cf, Self::doc_key(key));
                    delete_count += 1;

                    // Update indexes for delete
                    self.update_indexes_on_delete(key, old_data)?;
                    self.update_fulltext_on_delete(key, old_data)?;

                    // Broadcast change event
                    let _ = self.change_sender.send(ChangeEvent {
                        type_: ChangeType::Delete,
                        key: key.clone(),
                        data: None,
                        old_data: None, // We don't have old data in transaction op
                    });
                }
                crate::transaction::Operation::PutBlobChunk { key, chunk_index, data, .. } => {
                    let blob_key = Self::blo_chunk_key(key, *chunk_index);
                    batch.put_cf(cf, blob_key, data);
                }
                crate::transaction::Operation::DeleteBlob { key, .. } => {
                    // Iterate and delete all chunks
                    let prefix = format!("{}{}:", crate::storage::collection::BLO_PREFIX, key);
                    let prefix_bytes = prefix.as_bytes();
                    
                    // We need to collect keys first to avoid borrowing issues with iterator?
                    // Actually RocksDB iterator needs DB, batch is separate.
                    // But usually we iterate then delete.
                    // Since we are adding to batch, it's fine.
                    
                    let iter = db.iterator_cf(cf, rocksdb::IteratorMode::From(prefix_bytes, rocksdb::Direction::Forward));
                    for item in iter {
                         if let Ok((k, _)) = item {
                             if k.starts_with(prefix_bytes) {
                                 batch.delete_cf(cf, k);
                             } else {
                                 break;
                             }
                         }
                    }
                }
            }
        }

        // Atomic batch write
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to apply transaction: {}", e)))?;

        // Update document count
        if insert_count > 0 {
            self.doc_count
                .fetch_add(insert_count, std::sync::atomic::Ordering::Relaxed);
            self.count_dirty
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if delete_count > 0 {
            self.doc_count
                .fetch_sub(delete_count, std::sync::atomic::Ordering::Relaxed);
            self.count_dirty
                .store(true, std::sync::atomic::Ordering::Relaxed);
        }

        Ok(())
    }

    /// Get all documents
    pub fn all(&self) -> Vec<Document> {
        self.scan(None)
    }

    /// Scan documents with an optional limit
    pub fn scan(&self, limit: Option<usize>) -> Vec<Document> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let prefix = DOC_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        let iter = iter.filter_map(|result| {
            result.ok().and_then(|(key, value)| {
                // Check if key starts with doc prefix
                if key.starts_with(prefix) {
                    serde_json::from_slice(&value).ok()
                } else {
                    None
                }
            })
        });

        if let Some(n) = limit {
            iter.take(n).collect()
        } else {
            iter.collect()
        }
    }

    /// Get the number of documents
    pub fn count(&self) -> usize {
        // Fast atomic read - no disk I/O
        self.doc_count.load(Ordering::Relaxed)
    }

    /// Recount documents from actual RocksDB data (slow but accurate)
    /// Returns the actual count and updates the cached count
    pub fn recount_documents(&self) -> usize {
        let db_guard = self.db.read().unwrap();
        if let Some(cf) = db_guard.cf_handle(&self.name) {
            let prefix = DOC_PREFIX.as_bytes();
            let actual_count = db_guard
                .prefix_iterator_cf(cf, prefix)
                .take_while(|r| r.as_ref().map_or(false, |(k, _)| k.starts_with(prefix)))
                .count();
            
            // Update the cached count to match reality
            self.doc_count.store(actual_count, Ordering::Relaxed);
            self.count_dirty.store(true, Ordering::Relaxed);
            
            actual_count
        } else {
            0
        }
    }

    /// Increment document count (called on insert) - atomic, no disk I/O
    fn increment_count(&self) {
        self.doc_count.fetch_add(1, Ordering::Relaxed);
        self.count_dirty.store(true, Ordering::Relaxed);
    }

    /// Decrement document count (called on delete) - atomic, no disk I/O
    fn decrement_count(&self) {
        self.doc_count.fetch_sub(1, Ordering::Relaxed);
        self.count_dirty.store(true, Ordering::Relaxed);
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
    /// This is useful after bulk deletes or truncation
    pub fn compact(&self) {
        let db = self.db.read().unwrap();
        if let Some(cf) = db.cf_handle(&self.name) {
            db.compact_range_cf(cf, None::<&[u8]>, None::<&[u8]>);
        }
    }

    /// Recalculate and store document count (used for repair)
    pub fn recalculate_count(&self) -> usize {
        let db = self.db.read().unwrap();
        let cf = match db.cf_handle(&self.name) {
            Some(cf) => cf,
            None => return 0,
        };

        // Count document keys
        let prefix = DOC_PREFIX.as_bytes();
        let count = db
            .prefix_iterator_cf(cf, prefix)
            .take_while(|r| r.as_ref().map_or(false, |(k, _)| k.starts_with(prefix)))
            .count();

        // Update atomic counter and persist
        self.doc_count.store(count, Ordering::Relaxed);
        let _ = db.put_cf(cf, STATS_COUNT_KEY.as_bytes(), count.to_string().as_bytes());
        self.count_dirty.store(false, Ordering::Relaxed);
        count
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
        let num_sst_files = db
            .property_int_value_cf(cf, "rocksdb.num-files-at-level0")
            .ok()
            .flatten()
            .unwrap_or(0)
            + db.property_int_value_cf(cf, "rocksdb.num-files-at-level1")
                .ok()
                .flatten()
                .unwrap_or(0)
            + db.property_int_value_cf(cf, "rocksdb.num-files-at-level2")
                .ok()
                .flatten()
                .unwrap_or(0)
            + db.property_int_value_cf(cf, "rocksdb.num-files-at-level3")
                .ok()
                .flatten()
                .unwrap_or(0)
            + db.property_int_value_cf(cf, "rocksdb.num-files-at-level4")
                .ok()
                .flatten()
                .unwrap_or(0)
            + db.property_int_value_cf(cf, "rocksdb.num-files-at-level5")
                .ok()
                .flatten()
                .unwrap_or(0)
            + db.property_int_value_cf(cf, "rocksdb.num-files-at-level6")
                .ok()
                .flatten()
                .unwrap_or(0);

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
        
        tracing::info!("[SHARD_CONFIG] Saved config for {}: {:?}", self.name, config);

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

    /// Truncate collection - remove all documents but keep indexes
    pub fn truncate(&self) -> DbResult<usize> {
        let db = self.db.write().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Collect all document keys
        let prefix = DOC_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);
        let mut keys_to_delete = Vec::new();

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(prefix) {
                    keys_to_delete.push(key.to_vec());
                }
            }
        }

        let count = keys_to_delete.len();

        // Delete all documents
        for key in keys_to_delete {
            db.delete_cf(cf, &key)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        // Clear all index entries (but keep index metadata)
        let idx_prefix = IDX_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, idx_prefix);
        let mut idx_keys_to_delete = Vec::new();

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(idx_prefix) {
                    idx_keys_to_delete.push(key.to_vec());
                }
            }
        }

        for key in idx_keys_to_delete {
            db.delete_cf(cf, &key)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        // Clear fulltext index entries (but keep metadata)
        let ft_prefix = FT_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, ft_prefix);
        let mut ft_keys_to_delete = Vec::new();

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(ft_prefix) {
                    ft_keys_to_delete.push(key.to_vec());
                }
            }
        }

        for key in ft_keys_to_delete {
            db.delete_cf(cf, &key)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        // Clear fulltext term entries
        let ft_term_prefix = FT_TERM_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, ft_term_prefix);
        let mut ft_term_keys_to_delete = Vec::new();

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(ft_term_prefix) {
                    ft_term_keys_to_delete.push(key.to_vec());
                }
            }
        }

        for key in ft_term_keys_to_delete {
            db.delete_cf(cf, &key)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        // Clear geo index entries (but keep metadata)
        let geo_prefix = GEO_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, geo_prefix);
        let mut geo_keys_to_delete = Vec::new();

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(geo_prefix) {
                    geo_keys_to_delete.push(key.to_vec());
                }
            }
        }

        for key in geo_keys_to_delete {
            db.delete_cf(cf, &key)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        // Clear blob chunks (for blob collections)
        let blo_prefix = BLO_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, blo_prefix);
        let mut blo_keys_to_delete = Vec::new();

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(blo_prefix) {
                    blo_keys_to_delete.push(key.to_vec());
                }
            }
        }

        for key in blo_keys_to_delete {
            db.delete_cf(cf, &key)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        // Reset document count to 0 (both in-memory and on disk)
        self.doc_count.store(0, Ordering::Relaxed);
        self.count_dirty.store(false, Ordering::Relaxed);
        db.put_cf(cf, STATS_COUNT_KEY.as_bytes(), "0".as_bytes())
            .map_err(|e| DbError::InternalError(e.to_string()))?;

        // Trigger asynchronous compaction to reclaim space
        // We clone self to move it into the background thread.
        // Note: The background thread will acquire a read lock on the DB,
        // so it will wait until this function returns and drops the write lock.
        let collection = self.clone();
        std::thread::spawn(move || {
            tracing::info!(
                "[COMPACTION] Starting background compaction for collection '{}' after truncate",
                collection.name
            );
            let start = std::time::Instant::now();
            collection.compact();
            tracing::info!(
                "[COMPACTION] Background compaction for '{}' completed in {:?}",
                collection.name,
                start.elapsed()
            );
        });

        Ok(count)
    }

    // ==================== Index Operations ====================

    /// Get all index metadata
    pub fn get_all_indexes(&self) -> Vec<Index> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let prefix = IDX_META_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        iter.filter_map(|result| {
            result.ok().and_then(|(key, value)| {
                if key.starts_with(prefix) {
                    serde_json::from_slice(&value).ok()
                } else {
                    None
                }
            })
        })
        .collect()
    }

    /// Get an index by name
    fn get_index(&self, name: &str) -> Option<Index> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        db.get_cf(cf, Self::idx_meta_key(name))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Check unique constraints before inserting/updating a document
    fn check_unique_constraints(&self, doc_key: &str, doc_value: &Value) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for index in indexes {
            if index.unique {
                // For compound indexes, extract all field values
                let field_values: Vec<Value> = index.fields.iter()
                    .map(|f| extract_field_value(doc_value, f))
                    .collect();
                
                // Skip if all values are null
                if field_values.iter().all(|v| v.is_null()) {
                    continue;
                }
                
                let value_str = serde_json::to_string(&field_values).unwrap_or_default();
                let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_str);
                let mut iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

                // Check if any OTHER document already has this value
                if let Some(Ok((key, value))) = iter.next() {
                    if key.starts_with(prefix.as_bytes()) {
                        let existing_key = String::from_utf8_lossy(&value);
                        // Allow update of the same document
                        if existing_key != doc_key {
                            return Err(DbError::InvalidDocument(format!(
                                "Unique constraint violated: fields '{:?}' with value {} already exists in index '{}'",
                                index.fields, value_str, index.name
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Update indexes on document insert
    fn update_indexes_on_insert(&self, doc_key: &str, doc_value: &Value) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for index in indexes {
            let field_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(doc_value, f))
                .collect();

            if !field_values.iter().all(|v| v.is_null()) {
                let entry_key = Self::idx_entry_key(&index.name, &field_values, doc_key);
                db.put_cf(cf, entry_key, doc_key.as_bytes()).map_err(|e| {
                    DbError::InternalError(format!("Failed to update index: {}", e))
                })?;
            }
        }
        drop(db);

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for geo_index in geo_indexes {
            let field_value = extract_field_value(doc_value, &geo_index.field);
            if !field_value.is_null() {
                let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
                let geo_data = serde_json::to_vec(&field_value)?;
                db.put_cf(cf, entry_key, &geo_data).map_err(|e| {
                    DbError::InternalError(format!("Failed to update geo index: {}", e))
                })?;
            }
        }

        Ok(())
    }

    /// Update indexes on document update
    fn update_indexes_on_update(
        &self,
        doc_key: &str,
        old_value: &Value,
        new_value: &Value,
    ) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for index in indexes {
            let old_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(old_value, f))
                .collect();
            let new_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(new_value, f))
                .collect();

            // Remove old entry
            if !old_values.iter().all(|v| v.is_null()) {
                let old_entry_key = Self::idx_entry_key(&index.name, &old_values, doc_key);
                db.delete_cf(cf, old_entry_key).map_err(|e| {
                    DbError::InternalError(format!("Failed to update index: {}", e))
                })?;
            }

            // Add new entry
            if !new_values.iter().all(|v| v.is_null()) {
                let new_entry_key = Self::idx_entry_key(&index.name, &new_values, doc_key);
                db.put_cf(cf, new_entry_key, doc_key.as_bytes())
                    .map_err(|e| {
                        DbError::InternalError(format!("Failed to update index: {}", e))
                    })?;
            }
        }
        drop(db);

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for geo_index in geo_indexes {
            let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
            let new_field = extract_field_value(new_value, &geo_index.field);

            if !new_field.is_null() {
                let geo_data = serde_json::to_vec(&new_field)?;
                db.put_cf(cf, entry_key, &geo_data).map_err(|e| {
                    DbError::InternalError(format!("Failed to update geo index: {}", e))
                })?;
            } else {
                db.delete_cf(cf, entry_key).map_err(|e| {
                    DbError::InternalError(format!("Failed to update geo index: {}", e))
                })?;
            }
        }

        Ok(())
    }

    /// Update indexes on document delete
    fn update_indexes_on_delete(&self, doc_key: &str, doc_value: &Value) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for index in indexes {
            let field_values: Vec<Value> = index
                .fields
                .iter()
                .map(|f| extract_field_value(doc_value, f))
                .collect();
                
            if !field_values.iter().all(|v| v.is_null()) {
                let entry_key = Self::idx_entry_key(&index.name, &field_values, doc_key);
                db.delete_cf(cf, entry_key).map_err(|e| {
                    DbError::InternalError(format!("Failed to update index: {}", e))
                })?;
            }
        }
        drop(db);

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for geo_index in geo_indexes {
            let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
            db.delete_cf(cf, entry_key).map_err(|e| {
                DbError::InternalError(format!("Failed to update geo index: {}", e))
            })?;
        }

        Ok(())
    }

    /// Create an index on a field
    pub fn create_index(
        &self,
        name: String,
        fields: Vec<String>,
        index_type: IndexType,
        unique: bool,
    ) -> DbResult<IndexStats> {
        // Check if index already exists
        if self.get_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!(
                "Index '{}' already exists",
                name
            )));
        }

        // Create index metadata
        let index = Index::new(name.clone(), fields.clone(), index_type.clone(), unique);
        let index_bytes = serde_json::to_vec(&index)?;

        // Store index metadata and build index
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::idx_meta_key(&name), &index_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to create index: {}", e)))?;
        }

        // Build index from existing documents
        let docs = self.all();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for doc in &docs {
            let doc_value = doc.to_value();
            let field_values: Vec<Value> = fields
                .iter()
                .map(|f| extract_field_value(&doc_value, f))
                .collect();
                
            if !field_values.iter().all(|v| v.is_null()) {
                let entry_key = Self::idx_entry_key(&name, &field_values, &doc.key);
                db.put_cf(cf, entry_key, doc.key.as_bytes())
                    .map_err(|e| DbError::InternalError(format!("Failed to build index: {}", e)))?;
            }
        }

        Ok(IndexStats {
            name,
            field: fields.first().cloned().unwrap_or_default(),
            fields,
            index_type,
            unique,
            unique_values: docs.len(),
            indexed_documents: docs.len(),
        })
    }

    /// Drop an index
    pub fn drop_index(&self, name: &str) -> DbResult<()> {
        if self.get_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!(
                "Index '{}' not found",
                name
            )));
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Delete index metadata
        db.delete_cf(cf, Self::idx_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop index: {}", e)))?;

        // Delete all index entries
        let prefix = format!("{}{}:", IDX_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(prefix.as_bytes()) {
                    db.delete_cf(cf, &key).map_err(|e| {
                        DbError::InternalError(format!("Failed to drop index entry: {}", e))
                    })?;
                } else {
                    break;
                }
            }
        }

        Ok(())
    }

    /// List all indexes
    pub fn list_indexes(&self) -> Vec<IndexStats> {
        let mut stats: Vec<IndexStats> = self.get_all_indexes()
            .iter()
            .filter_map(|idx| self.get_index_stats(&idx.name))
            .collect();

        // Include fulltext indexes
        for idx in self.get_all_fulltext_indexes() {
            stats.push(IndexStats {
                name: idx.name,
                fields: idx.fields.clone(),
                field: idx.fields.first().cloned().unwrap_or_default(),
                index_type: IndexType::Fulltext,
                unique: false,
                unique_values: 0, // Not calculated for fulltext
                indexed_documents: 0, // Not calculated for fulltext
            });
        }

        stats
    }

    /// Rebuild all indexes from existing documents
    /// Call this after bulk imports using insert_no_index()
    pub fn rebuild_all_indexes(&self) -> DbResult<usize> {
        use rocksdb::WriteBatch;

        let total_start = std::time::Instant::now();

        let indexes = self.get_all_indexes();
        let geo_indexes = self.get_all_geo_indexes();
        let ft_indexes = self.get_all_fulltext_indexes();

        tracing::info!(
            "rebuild_all_indexes: {} regular, {} geo, {} fulltext indexes",
            indexes.len(),
            geo_indexes.len(),
            ft_indexes.len()
        );

        if indexes.is_empty() && geo_indexes.is_empty() && ft_indexes.is_empty() {
            return Ok(0);
        }

        // Clear existing index entries using WriteBatch
        let clear_start = std::time::Instant::now();
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            let mut batch = WriteBatch::default();

            // Clear regular indexes
            for index in &indexes {
                let prefix = format!("{}{}:", IDX_PREFIX, index.name);
                let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());
                for result in iter {
                    if let Ok((key, _)) = result {
                        if key.starts_with(prefix.as_bytes()) {
                            batch.delete_cf(cf, &key);
                        } else {
                            break;
                        }
                    }
                }
            }

            // Clear geo indexes
            for geo_index in &geo_indexes {
                let prefix = format!("{}{}:", GEO_PREFIX, geo_index.name);
                let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());
                for result in iter {
                    if let Ok((key, _)) = result {
                        if key.starts_with(prefix.as_bytes()) {
                            batch.delete_cf(cf, &key);
                        } else {
                            break;
                        }
                    }
                }
            }

            // Clear fulltext indexes
            for ft_index in &ft_indexes {
                let ngram_prefix = format!("{}{}:", FT_PREFIX, ft_index.name);
                let iter = db.prefix_iterator_cf(cf, ngram_prefix.as_bytes());
                for result in iter {
                    if let Ok((key, _)) = result {
                        if key.starts_with(ngram_prefix.as_bytes()) {
                            batch.delete_cf(cf, &key);
                        } else {
                            break;
                        }
                    }
                }

                let term_prefix = format!("{}{}:", FT_TERM_PREFIX, ft_index.name);
                let iter = db.prefix_iterator_cf(cf, term_prefix.as_bytes());
                for result in iter {
                    if let Ok((key, _)) = result {
                        if key.starts_with(term_prefix.as_bytes()) {
                            batch.delete_cf(cf, &key);
                        } else {
                            break;
                        }
                    }
                }
            }

            let _ = db.write(batch);
        }
        tracing::info!(
            "rebuild_all_indexes: Clear phase took {:?}",
            clear_start.elapsed()
        );

        // Load all documents
        let load_start = std::time::Instant::now();
        let docs = self.all();
        let doc_count = docs.len();
        tracing::info!(
            "rebuild_all_indexes: Loaded {} docs in {:?}",
            doc_count,
            load_start.elapsed()
        );

        // Rebuild regular indexes using WriteBatch
        if !indexes.is_empty() {
            let idx_start = std::time::Instant::now();
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            let mut batch = WriteBatch::default();

            for doc in &docs {
                let doc_value = doc.to_value();
                for index in &indexes {
                    // Extract values for all fields in the compound index
                    let field_values: Vec<Value> = index.fields.iter()
                        .map(|f| extract_field_value(&doc_value, f))
                        .collect();
                    
                    // Index if at least one field is not null
                    if !field_values.iter().all(|v| v.is_null()) {
                        let entry_key = Self::idx_entry_key(&index.name, &field_values, &doc.key);
                        batch.put_cf(cf, entry_key, doc.key.as_bytes());
                    }
                }
            }

            let _ = db.write(batch);
            tracing::info!(
                "rebuild_all_indexes: Regular indexes took {:?}",
                idx_start.elapsed()
            );
        }

        // Rebuild geo indexes using WriteBatch
        if !geo_indexes.is_empty() {
            let geo_start = std::time::Instant::now();
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            let mut batch = WriteBatch::default();

            for doc in &docs {
                let doc_value = doc.to_value();
                for geo_index in &geo_indexes {
                    let field_value = extract_field_value(&doc_value, &geo_index.field);
                    if !field_value.is_null() {
                        let entry_key = Self::geo_entry_key(&geo_index.name, &doc.key);
                        if let Ok(geo_data) = serde_json::to_vec(&field_value) {
                            batch.put_cf(cf, entry_key, &geo_data);
                        }
                    }
                }
            }

            let _ = db.write(batch);
            tracing::info!(
                "rebuild_all_indexes: Geo indexes took {:?}",
                geo_start.elapsed()
            );
        }

        // Rebuild fulltext indexes (this is the slow part - n-gram generation)
        if !ft_indexes.is_empty() {
            let ft_start = std::time::Instant::now();
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            let mut batch = WriteBatch::default();

            for doc in &docs {
                let doc_value = doc.to_value();
                for ft_index in &ft_indexes {
                    for field in &ft_index.fields {
                        let field_value = extract_field_value(&doc_value, field);
                        if let Some(text) = field_value.as_str() {
                            // Index terms
                            let terms = tokenize(text);
                            for term in &terms {
                                if term.len() >= ft_index.min_length {
                                    let term_key = Self::ft_term_key(&ft_index.name, term, &doc.key);
                                    batch.put_cf(cf, term_key, doc.key.as_bytes());
                                }
                            }

                            // Index n-grams
                            let ngrams = generate_ngrams(text, NGRAM_SIZE);
                            for ngram in &ngrams {
                                let ngram_key = Self::ft_ngram_key(&ft_index.name, ngram, &doc.key);
                                batch.put_cf(cf, ngram_key, doc.key.as_bytes());
                            }
                        }
                    }
                }
            }

            let _ = db.write(batch);
            tracing::info!(
                "rebuild_all_indexes: Fulltext indexes took {:?}",
                ft_start.elapsed()
            );
        }

        tracing::info!(
            "rebuild_all_indexes: Total time {:?}",
            total_start.elapsed()
        );
        Ok(doc_count)
    }

    /// Get index statistics
    fn get_index_stats(&self, name: &str) -> Option<IndexStats> {
        let index = self.get_index(name)?;

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        // Count entries
        let prefix = format!("{}{}:", IDX_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());
        let count = iter
            .filter(|r| {
                r.as_ref()
                    .map(|(k, _)| k.starts_with(prefix.as_bytes()))
                    .unwrap_or(false)
            })
            .count();

        Some(IndexStats {
            name: index.name,
            fields: index.fields.clone(),
            field: index.fields.first().cloned().unwrap_or_default(),
            index_type: index.index_type,
            unique: index.unique,
            unique_values: count,
            indexed_documents: count,
        })
    }

    /// Get an index for a field
    pub fn get_index_for_field(&self, field: &str) -> Option<Index> {
        // Direct lookup using field-to-index mapping key
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        // Try to find index by checking idx_meta entries
        // Use a more targeted approach - look for index with matching field
        let prefix = IDX_META_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        for result in iter {
            if let Ok((key, value)) = result {
                if !key.starts_with(prefix) {
                    break;
                }
                if let Ok(index) = serde_json::from_slice::<Index>(&value) {
                    // Check if field is the first field in the index (prefix match)
                    if index.fields.first().map(|s| s.as_str()) == Some(field) {
                        return Some(index);
                    }
                }
            }
        }
        None
    }

    /// Lookup documents using index (equality) - optimized version
    pub fn index_lookup_eq(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        let index = self.get_index_for_field(field)?;
        let value_str = hex::encode(crate::storage::codec::encode_key(value));

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_str);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        // Collect document keys from index
        let doc_keys: Vec<Vec<u8>> = iter
            .filter_map(|r| r.ok())
            .take_while(|(k, _)| k.starts_with(prefix.as_bytes()))
            .map(|(_, v)| {
                // Build the document key directly
                let key_str = String::from_utf8_lossy(&v);
                Self::doc_key(&key_str)
            })
            .collect();

        if doc_keys.is_empty() {
            return Some(Vec::new());
        }

        // Use multi_get for batch retrieval (much faster than individual gets)
        let results = db.multi_get_cf(doc_keys.iter().map(|k| (cf, k.as_slice())));

        let docs: Vec<Document> = results
            .into_iter()
            .filter_map(|r| r.ok())
            .filter_map(|opt| opt)
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect();

        Some(docs)
    }

    /// Lookup documents using index (equality) with limit - for high-cardinality fields
    pub fn index_lookup_eq_limit(
        &self,
        field: &str,
        value: &Value,
        limit: usize,
    ) -> Option<Vec<Document>> {
        let index = self.get_index_for_field(field)?;
        let value_str = hex::encode(crate::storage::codec::encode_key(value));

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, value_str);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        // Collect document keys from index (limited)
        let doc_keys: Vec<Vec<u8>> = iter
            .filter_map(|r| r.ok())
            .take_while(|(k, _)| k.starts_with(prefix.as_bytes()))
            .take(limit) // Apply limit early
            .map(|(_, v)| {
                let key_str = String::from_utf8_lossy(&v);
                Self::doc_key(&key_str)
            })
            .collect();

        if doc_keys.is_empty() {
            return Some(Vec::new());
        }

        let results = db.multi_get_cf(doc_keys.iter().map(|k| (cf, k.as_slice())));

        let docs: Vec<Document> = results
            .into_iter()
            .filter_map(|r| r.ok())
            .filter_map(|opt| opt)
            .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
            .collect();

        Some(docs)
    }

    /// Lookup documents using index (greater than)
    pub fn index_lookup_gt(&self, _field: &str, _value: &Value) -> Option<Vec<Document>> {
        // For simplicity, fall back to scan for range queries
        None
    }

    /// Lookup documents using index (greater than or equal)
    pub fn index_lookup_gte(&self, _field: &str, _value: &Value) -> Option<Vec<Document>> {
        None
    }

    /// Lookup documents using index (less than)
    pub fn index_lookup_lt(&self, _field: &str, _value: &Value) -> Option<Vec<Document>> {
        None
    }

    /// Lookup documents using index (less than or equal)
    pub fn index_lookup_lte(&self, _field: &str, _value: &Value) -> Option<Vec<Document>> {
        None
    }

    /// Get documents sorted by indexed field with optional limit
    /// Returns documents in sorted order by the indexed field
    /// 
    /// OPTIMIZATION: For LIMIT 1, uses seek to find first/last entry directly.
    pub fn index_sorted(
        &self,
        field: &str,
        ascending: bool,
        limit: Option<usize>,
    ) -> Option<Vec<Document>> {
        use rocksdb::{IteratorMode, Direction};
        
        let index = self.get_index_for_field(field)?;
        let index_name = index.name.clone();
        let prefix = format!("{}{}:", IDX_PREFIX, index_name);

        // Optimized path for all limits using binary-comparable keys
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        let prefix_bytes = prefix.as_bytes();
        
        // Iterator over index entries
        // Since we use binary-comparable encoding (wrapped in hex), 
        // the lexicographical order of keys matches the logical order of values.
        let iter = if ascending {
            let mode = IteratorMode::From(prefix_bytes, Direction::Forward);
            db.iterator_cf(cf, mode)
        } else {
            // For descending, we seek past the end of the prefix
            // Prefix + 0xFF is theoretically after any key starting with prefix
            let mut seek_key = prefix.as_bytes().to_vec();
            seek_key.push(0xFF);
            let mode = IteratorMode::From(&seek_key, Direction::Reverse);
            db.iterator_cf(cf, mode)
        };

        // Collect document keys directly from iterator order
        // No sorting needed!
        let doc_keys: Vec<String> = iter
            .filter_map(|r| r.ok())
            .take_while(|(k, _)| k.starts_with(prefix_bytes))
            .map(|(_, v)| String::from_utf8_lossy(&v).to_string())
            .take(limit.unwrap_or(usize::MAX))
            .collect();

        drop(db); 

        // Fetch documents
        if doc_keys.is_empty() {
             return Some(Vec::new());
        }

        let docs = self.get_many(&doc_keys);
        
        // Re-order docs based on doc_keys order (get_many might return disordered)
        let doc_map: std::collections::HashMap<_, _> =
            docs.into_iter().map(|d| (d.key.clone(), d)).collect();
        
        let result: Vec<Document> = doc_keys
            .into_iter()
            .filter_map(|key| doc_map.get(&key).cloned())
            .collect();
        
        Some(result)
    }

    // ==================== Geo Index Operations ====================

    /// Get all geo index metadata
    fn get_all_geo_indexes(&self) -> Vec<GeoIndex> {
        let db = self.db.read().unwrap();
        let cf = match db.cf_handle(&self.name) {
            Some(cf) => cf,
            None => return vec![],
        };
        let prefix = GEO_META_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        iter.filter_map(|result| {
            result.ok().and_then(|(key, value)| {
                if key.starts_with(prefix) {
                    serde_json::from_slice(&value).ok()
                } else {
                    None
                }
            })
        })
        .collect()
    }

    /// Get a geo index by name
    fn get_geo_index(&self, name: &str) -> Option<GeoIndex> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        db.get_cf(cf, Self::geo_meta_key(name))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Create a geo index on a field
    pub fn create_geo_index(&self, name: String, field: String) -> DbResult<GeoIndexStats> {
        if self.get_geo_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!(
                "Geo index '{}' already exists",
                name
            )));
        }

        let geo_index = GeoIndex::new(name.clone(), field.clone());
        let index_bytes = serde_json::to_vec(&geo_index)?;

        // Store geo index metadata
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::geo_meta_key(&name), &index_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create geo index: {}", e))
                })?;
        }

        // Build index from existing documents
        let docs = self.all();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for doc in &docs {
            let doc_value = doc.to_value();
            let field_value = extract_field_value(&doc_value, &field);
            if !field_value.is_null() {
                let entry_key = Self::geo_entry_key(&name, &doc.key);
                let geo_data = serde_json::to_vec(&field_value)?;
                db.put_cf(cf, entry_key, &geo_data).map_err(|e| {
                    DbError::InternalError(format!("Failed to build geo index: {}", e))
                })?;
            }
        }

        Ok(geo_index.stats())
    }

    /// Drop a geo index
    pub fn drop_geo_index(&self, name: &str) -> DbResult<()> {
        if self.get_geo_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!(
                "Geo index '{}' not found",
                name
            )));
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Delete geo index metadata
        db.delete_cf(cf, Self::geo_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop geo index: {}", e)))?;

        // Delete all geo index entries
        let prefix = format!("{}{}:", GEO_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(prefix.as_bytes()) {
                    db.delete_cf(cf, &key).map_err(|e| {
                        DbError::InternalError(format!("Failed to drop geo index entry: {}", e))
                    })?;
                } else {
                    break;
                }
            }
        }

        Ok(())
    }

    /// List all geo indexes
    pub fn list_geo_indexes(&self) -> Vec<GeoIndexStats> {
        self.get_all_geo_indexes()
            .iter()
            .map(|idx| idx.stats())
            .collect()
    }

    /// Find documents near a point
    pub fn geo_near(
        &self,
        field: &str,
        lat: f64,
        lon: f64,
        limit: usize,
    ) -> Option<Vec<(Document, f64)>> {
        use super::geo::{haversine_distance, GeoPoint};

        let geo_index = self
            .get_all_geo_indexes()
            .into_iter()
            .find(|idx| idx.field == field)?;

        let target = GeoPoint::new(lat, lon);

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        let prefix = format!("{}{}:", GEO_PREFIX, geo_index.name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        let mut results: Vec<(String, f64)> = iter
            .filter_map(|r| r.ok())
            .filter(|(k, _)| k.starts_with(prefix.as_bytes()))
            .filter_map(|(key, value)| {
                let key_str = String::from_utf8(key.to_vec()).ok()?;
                let doc_key = key_str.strip_prefix(&prefix)?;
                let geo_value: Value = serde_json::from_slice(&value).ok()?;
                let point = GeoPoint::from_value(&geo_value)?;
                let dist = haversine_distance(&target, &point);
                Some((doc_key.to_string(), dist))
            })
            .collect();
        drop(db);

        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        let docs: Vec<(Document, f64)> = results
            .into_iter()
            .filter_map(|(key, dist)| self.get(&key).ok().map(|doc| (doc, dist)))
            .collect();

        Some(docs)
    }

    /// Find documents within a radius of a point
    pub fn geo_within(
        &self,
        field: &str,
        lat: f64,
        lon: f64,
        radius_meters: f64,
    ) -> Option<Vec<(Document, f64)>> {
        use super::geo::{haversine_distance, GeoPoint};

        let geo_index = self
            .get_all_geo_indexes()
            .into_iter()
            .find(|idx| idx.field == field)?;

        let target = GeoPoint::new(lat, lon);

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        let prefix = format!("{}{}:", GEO_PREFIX, geo_index.name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        let mut results: Vec<(String, f64)> = iter
            .filter_map(|r| r.ok())
            .filter(|(k, _)| k.starts_with(prefix.as_bytes()))
            .filter_map(|(key, value)| {
                let key_str = String::from_utf8(key.to_vec()).ok()?;
                let doc_key = key_str.strip_prefix(&prefix)?;
                let geo_value: Value = serde_json::from_slice(&value).ok()?;
                let point = GeoPoint::from_value(&geo_value)?;
                let dist = haversine_distance(&target, &point);
                if dist <= radius_meters {
                    Some((doc_key.to_string(), dist))
                } else {
                    None
                }
            })
            .collect();
        drop(db);

        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let docs: Vec<(Document, f64)> = results
            .into_iter()
            .filter_map(|(key, dist)| self.get(&key).ok().map(|doc| (doc, dist)))
            .collect();

        Some(docs)
    }

    // ==================== Fulltext Index Operations ====================

    /// Get all fulltext index metadata
    fn get_all_fulltext_indexes(&self) -> Vec<FulltextIndex> {
        let db = self.db.read().unwrap();
        let cf = match db.cf_handle(&self.name) {
            Some(cf) => cf,
            None => return vec![],
        };
        let prefix = FT_META_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        iter.filter_map(|result| {
            result.ok().and_then(|(key, value)| {
                if key.starts_with(prefix) {
                    serde_json::from_slice(&value).ok()
                } else {
                    None
                }
            })
        })
        .collect()
    }

    /// Get a fulltext index by name
    fn get_fulltext_index(&self, name: &str) -> Option<FulltextIndex> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        db.get_cf(cf, Self::ft_meta_key(name))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Get fulltext index for a field
    pub fn get_fulltext_index_for_field(&self, field: &str) -> Option<String> {
        self.get_all_fulltext_indexes()
            .into_iter()
            .find(|idx| idx.fields.contains(&field.to_string()))
            .map(|idx| idx.name)
    }

    /// Create a fulltext index on a field
    pub fn create_fulltext_index(
        &self,
        name: String,
        fields: Vec<String>,
        min_length: Option<usize>,
    ) -> DbResult<IndexStats> {
        if self.get_fulltext_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!(
                "Fulltext index '{}' already exists",
                name
            )));
        }

        let min_len = min_length.unwrap_or(3);
        let ft_index = FulltextIndex {
            name: name.clone(),
            fields: fields.clone(),
            min_length: min_len,
        };
        let index_bytes = serde_json::to_vec(&ft_index)?;

        // Store fulltext index metadata
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::ft_meta_key(&name), &index_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create fulltext index: {}", e))
                })?;
        }

        // Build index from existing documents
        let docs = self.all();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for doc in &docs {
            let doc_value = doc.to_value();
            for field in &fields {
                let field_value = extract_field_value(&doc_value, field);
                if let Some(text) = field_value.as_str() {
                    // Index terms
                    let terms = tokenize(text);
                    for term in &terms {
                        if term.len() >= min_len {
                            let term_key = Self::ft_term_key(&name, term, &doc.key);
                            db.put_cf(cf, term_key, doc.key.as_bytes()).map_err(|e| {
                                DbError::InternalError(format!("Failed to build fulltext index: {}", e))
                            })?;
                        }
                    }

                    // Index n-grams for fuzzy matching
                    let ngrams = generate_ngrams(text, NGRAM_SIZE);
                    for ngram in &ngrams {
                        let ngram_key = Self::ft_ngram_key(&name, ngram, &doc.key);
                        db.put_cf(cf, ngram_key, doc.key.as_bytes()).map_err(|e| {
                            DbError::InternalError(format!("Failed to build fulltext index: {}", e))
                        })?;
                    }
                }
            }
        }

        Ok(IndexStats {
            name,
            fields: fields.clone(),
            field: fields.first().cloned().unwrap_or_default(),
            index_type: IndexType::Fulltext,
            unique: false,
            unique_values: docs.len(),
            indexed_documents: docs.len(),
        })
    }

    /// Drop a fulltext index
    pub fn drop_fulltext_index(&self, name: &str) -> DbResult<()> {
        if self.get_fulltext_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!(
                "Fulltext index '{}' not found",
                name
            )));
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Delete fulltext index metadata
        db.delete_cf(cf, Self::ft_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop fulltext index: {}", e)))?;

        // Delete all n-gram entries
        let ngram_prefix = format!("{}{}:", FT_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, ngram_prefix.as_bytes());
        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(ngram_prefix.as_bytes()) {
                    db.delete_cf(cf, &key).map_err(|e| {
                        DbError::InternalError(format!("Failed to drop fulltext index: {}", e))
                    })?;
                } else {
                    break;
                }
            }
        }

        // Delete all term entries
        let term_prefix = format!("{}{}:", FT_TERM_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, term_prefix.as_bytes());
        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(term_prefix.as_bytes()) {
                    db.delete_cf(cf, &key).map_err(|e| {
                        DbError::InternalError(format!("Failed to drop fulltext index: {}", e))
                    })?;
                } else {
                    break;
                }
            }
        }

        Ok(())
    }

    /// Update fulltext indexes on document insert
    fn update_fulltext_on_insert(&self, doc_key: &str, doc_value: &Value) -> DbResult<()> {
        let ft_indexes = self.get_all_fulltext_indexes();
        if ft_indexes.is_empty() {
            return Ok(());
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for ft_index in ft_indexes {
            for field in &ft_index.fields {
                let field_value = extract_field_value(doc_value, field);
                if let Some(text) = field_value.as_str() {
                    // Index terms
                    let terms = tokenize(text);
                    for term in &terms {
                        if term.len() >= ft_index.min_length {
                            let term_key = Self::ft_term_key(&ft_index.name, term, doc_key);
                            db.put_cf(cf, term_key, doc_key.as_bytes()).map_err(|e| {
                                DbError::InternalError(format!(
                                    "Failed to update fulltext index: {}",
                                    e
                                ))
                            })?;
                        }
                    }

                    // Index n-grams
                    let ngrams = generate_ngrams(text, NGRAM_SIZE);
                    for ngram in &ngrams {
                        let ngram_key = Self::ft_ngram_key(&ft_index.name, ngram, doc_key);
                        db.put_cf(cf, ngram_key, doc_key.as_bytes()).map_err(|e| {
                            DbError::InternalError(format!("Failed to update fulltext index: {}", e))
                        })?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Update fulltext indexes on document delete
    fn update_fulltext_on_delete(&self, doc_key: &str, doc_value: &Value) -> DbResult<()> {
        let ft_indexes = self.get_all_fulltext_indexes();
        if ft_indexes.is_empty() {
            return Ok(());
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for ft_index in ft_indexes {
            for field in &ft_index.fields {
                let field_value = extract_field_value(doc_value, field);
                if let Some(text) = field_value.as_str() {
                    // Remove terms
                    let terms = tokenize(text);
                    for term in &terms {
                        if term.len() >= ft_index.min_length {
                            let term_key = Self::ft_term_key(&ft_index.name, term, doc_key);
                            let _ = db.delete_cf(cf, term_key);
                        }
                    }

                    // Remove n-grams
                    let ngrams = generate_ngrams(text, NGRAM_SIZE);
                    for ngram in &ngrams {
                        let ngram_key = Self::ft_ngram_key(&ft_index.name, ngram, doc_key);
                        let _ = db.delete_cf(cf, ngram_key);
                    }
                }
            }
        }

        Ok(())
    }

    /// Fulltext search with fuzzy matching
    /// Returns documents matching the query with relevance scores
    pub fn fulltext_search(
        &self,
        field: &str,
        query: &str,
        max_distance: usize,
    ) -> Option<Vec<FulltextMatch>> {
        let ft_index = self
            .get_all_fulltext_indexes()
            .into_iter()
            .find(|idx| idx.fields.contains(&field.to_string()))?;

        let query_terms = tokenize(query);
        let query_ngrams = generate_ngrams(query, NGRAM_SIZE);

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        // Step 1: Find candidate documents using n-gram matching
        let mut candidate_scores: HashMap<String, (usize, HashSet<String>)> = HashMap::new();

        // Search for exact term matches first
        for term in &query_terms {
            let term_prefix = format!("{}{}:{}:", FT_TERM_PREFIX, ft_index.name, term);
            let iter = db.prefix_iterator_cf(cf, term_prefix.as_bytes());

            for result in iter {
                if let Ok((key, _)) = result {
                    if key.starts_with(term_prefix.as_bytes()) {
                        let key_str = String::from_utf8(key.to_vec()).ok()?;
                        if let Some(doc_key) = key_str.strip_prefix(&term_prefix) {
                            let entry = candidate_scores
                                .entry(doc_key.to_string())
                                .or_insert((0, HashSet::new()));
                            entry.0 += 10; // High score for exact match
                            entry.1.insert(term.clone());
                        }
                    } else {
                        break;
                    }
                }
            }
        }

        // Search using n-grams for fuzzy matching
        for ngram in &query_ngrams {
            let ngram_prefix = format!("{}{}:{}:", FT_PREFIX, ft_index.name, ngram);
            let iter = db.prefix_iterator_cf(cf, ngram_prefix.as_bytes());

            for result in iter {
                if let Ok((key, _)) = result {
                    if key.starts_with(ngram_prefix.as_bytes()) {
                        let key_str = String::from_utf8(key.to_vec()).ok()?;
                        if let Some(doc_key) = key_str.strip_prefix(&ngram_prefix) {
                            let entry = candidate_scores
                                .entry(doc_key.to_string())
                                .or_insert((0, HashSet::new()));
                            entry.0 += 1; // Lower score for n-gram match
                        }
                    } else {
                        break;
                    }
                }
            }
        }
        drop(db);

        // Step 2: Verify candidates with Levenshtein distance and compute final scores
        let mut results: Vec<FulltextMatch> = Vec::new();

        for (doc_key, (ngram_score, matched_terms)) in candidate_scores.into_iter() {
            if let Ok(doc) = self.get(&doc_key) {
                let doc_value = doc.to_value();
                let field_value = extract_field_value(&doc_value, field);

                if let Some(doc_text) = field_value.as_str() {
                    let doc_terms = tokenize(doc_text);
                    let mut total_score = ngram_score as f64;
                    let mut all_matched: HashSet<String> = matched_terms;

                    // Check fuzzy matches
                    for query_term in &query_terms {
                        for doc_term in &doc_terms {
                            let distance = levenshtein_distance(query_term, doc_term);
                            if distance <= max_distance {
                                // Score based on distance (closer = higher score)
                                let match_score = ((max_distance - distance + 1) as f64) * 5.0;
                                total_score += match_score;
                                all_matched.insert(doc_term.clone());
                            }
                        }
                    }

                    if !all_matched.is_empty() || total_score > 0.0 {
                        // Normalize score
                        let final_score = total_score / (query_terms.len().max(1) as f64);

                        results.push(FulltextMatch {
                            doc_key: doc_key.clone(),
                            score: final_score,
                            matched_terms: all_matched.into_iter().collect(),
                        });
                    }
                }
            }
        }

        // Sort by score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Some(results)
    }

    /// List all fulltext indexes
    pub fn list_fulltext_indexes(&self) -> Vec<IndexStats> {
        self.get_all_fulltext_indexes()
            .iter()
            .map(|idx| IndexStats {
                name: idx.name.clone(),
                fields: idx.fields.clone(),
                field: idx.fields.first().cloned().unwrap_or_default(),
                index_type: IndexType::Fulltext,
                unique: false,
                unique_values: 0,
                indexed_documents: 0,
            })
            .collect()
    }

    // ==================== TTL Index Operations ====================

    /// Build a TTL index metadata key
    fn ttl_meta_key(index_name: &str) -> Vec<u8> {
        format!("{}{}", TTL_META_PREFIX, index_name).into_bytes()
    }

    /// Get all TTL index metadata
    fn get_all_ttl_indexes(&self) -> Vec<TtlIndex> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let prefix = TTL_META_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        iter.filter_map(|result| {
            result.ok().and_then(|(key, value)| {
                if key.starts_with(prefix) {
                    serde_json::from_slice(&value).ok()
                } else {
                    None
                }
            })
        })
        .collect()
    }

    /// Get a TTL index by name
    fn get_ttl_index(&self, name: &str) -> Option<TtlIndex> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        db.get_cf(cf, Self::ttl_meta_key(name))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Create a TTL index on a timestamp field
    pub fn create_ttl_index(
        &self,
        name: String,
        field: String,
        expire_after_seconds: u64,
    ) -> DbResult<TtlIndexStats> {
        // Check if TTL index already exists
        if self.get_ttl_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!(
                "TTL index '{}' already exists",
                name
            )));
        }

        // Create TTL index metadata
        let index = TtlIndex::new(name.clone(), field.clone(), expire_after_seconds);
        let index_bytes = serde_json::to_vec(&index)?;

        // Store TTL index metadata
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::ttl_meta_key(&name), &index_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to create TTL index: {}", e)))?;
        }

        Ok(TtlIndexStats {
            name,
            field,
            expire_after_seconds,
        })
    }

    /// Drop a TTL index
    pub fn drop_ttl_index(&self, name: &str) -> DbResult<()> {
        if self.get_ttl_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!(
                "TTL index '{}' not found",
                name
            )));
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Delete TTL index metadata
        db.delete_cf(cf, Self::ttl_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop TTL index: {}", e)))?;

        Ok(())
    }

    /// List all TTL indexes
    pub fn list_ttl_indexes(&self) -> Vec<TtlIndexStats> {
        self.get_all_ttl_indexes()
            .iter()
            .map(|idx| TtlIndexStats {
                name: idx.name.clone(),
                field: idx.field.clone(),
                expire_after_seconds: idx.expire_after_seconds,
            })
            .collect()
    }

    /// Cleanup expired documents for a specific TTL index
    /// Returns the number of documents deleted
    pub fn cleanup_expired_documents_for_ttl_index(&self, index: &TtlIndex) -> DbResult<usize> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let docs = self.all();
        let mut deleted_count = 0;

        for doc in docs {
            let doc_value = doc.to_value();
            let field_value = extract_field_value(&doc_value, &index.field);

            // Check if field is a valid timestamp
            if let Some(timestamp) = field_value.as_u64().or_else(|| field_value.as_i64().map(|v| v as u64)) {
                // Calculate expiration time
                let expiration_time = timestamp.saturating_add(index.expire_after_seconds);

                // Delete if expired
                if now >= expiration_time {
                    if self.delete(&doc.key).is_ok() {
                        deleted_count += 1;
                    }
                }
            }
        }

        Ok(deleted_count)
    }

    /// Cleanup all expired documents for all TTL indexes on this collection
    /// Returns the total number of documents deleted
    pub fn cleanup_all_expired_documents(&self) -> DbResult<usize> {
        let ttl_indexes = self.get_all_ttl_indexes();
        let mut total_deleted = 0;

        for index in ttl_indexes {
            match self.cleanup_expired_documents_for_ttl_index(&index) {
                Ok(count) => total_deleted += count,
                Err(e) => {
                    tracing::warn!(
                        "Failed to cleanup expired documents for TTL index '{}': {}",
                        index.name,
                        e
                    );
                }
            }
        }

        Ok(total_deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_doc_key() {
        let key = Collection::doc_key("user123");
        let key_str = String::from_utf8(key).unwrap();
        assert!(key_str.starts_with(DOC_PREFIX));
        assert!(key_str.contains("user123"));
    }

    #[test]
    fn test_blo_chunk_key() {
        let key = Collection::blo_chunk_key("file1", 5);
        let key_str = String::from_utf8(key.clone()).unwrap();
        assert!(key_str.starts_with(BLO_PREFIX));
        assert!(key_str.contains("file1"));
    }

    #[test]
    fn test_idx_meta_key() {
        let key = Collection::idx_meta_key("idx_name");
        let key_str = String::from_utf8(key).unwrap();
        assert!(key_str.starts_with(IDX_META_PREFIX));
        assert!(key_str.contains("idx_name"));
    }

    #[test]
    fn test_idx_entry_key() {
        let values = vec![json!("value1"), json!(42)];
        let key = Collection::idx_entry_key("myindex", &values, "doc1");
        let key_str = String::from_utf8(key).unwrap();
        assert!(key_str.starts_with(IDX_PREFIX));
        assert!(key_str.contains("myindex"));
    }

    #[test]
    fn test_geo_meta_key() {
        let key = Collection::geo_meta_key("geo_idx");
        let key_str = String::from_utf8(key).unwrap();
        assert!(key_str.starts_with(GEO_META_PREFIX));
    }

    #[test]
    fn test_geo_entry_key() {
        let key = Collection::geo_entry_key("geo_idx", "doc1");
        let key_str = String::from_utf8(key).unwrap();
        assert!(key_str.starts_with(GEO_PREFIX));
    }

    #[test]
    fn test_ft_meta_key() {
        let key = Collection::ft_meta_key("fulltext_idx");
        let key_str = String::from_utf8(key).unwrap();
        assert!(key_str.starts_with(FT_META_PREFIX));
    }

    #[test]
    fn test_ft_ngram_key() {
        let key = Collection::ft_ngram_key("ft_idx", "hel", "doc1");
        let key_str = String::from_utf8(key).unwrap();
        assert!(key_str.starts_with(FT_PREFIX));
    }

    #[test]
    fn test_ft_term_key() {
        let key = Collection::ft_term_key("ft_idx", "hello", "doc1");
        let key_str = String::from_utf8(key).unwrap();
        assert!(key_str.starts_with(FT_TERM_PREFIX));
    }

    #[test]
    fn test_change_type_serialization() {
        let insert = ChangeType::Insert;
        let json = serde_json::to_string(&insert).unwrap();
        assert_eq!(json, "\"insert\"");

        let update = ChangeType::Update;
        let json = serde_json::to_string(&update).unwrap();
        assert_eq!(json, "\"update\"");

        let delete = ChangeType::Delete;
        let json = serde_json::to_string(&delete).unwrap();
        assert_eq!(json, "\"delete\"");
    }

    #[test]
    fn test_change_event_creation() {
        let event = ChangeEvent {
            type_: ChangeType::Insert,
            key: "doc1".to_string(),
            data: Some(json!({"name": "Alice"})),
            old_data: None,
        };

        assert_eq!(event.key, "doc1");
        assert!(event.data.is_some());
        assert!(event.old_data.is_none());
    }

    #[test]
    fn test_change_event_serialization() {
        let event = ChangeEvent {
            type_: ChangeType::Update,
            key: "doc1".to_string(),
            data: Some(json!({"a": 1})),
            old_data: Some(json!({"a": 0})),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"type\":\"update\""));
        assert!(json.contains("\"key\":\"doc1\""));
    }

    #[test]
    fn test_collection_stats() {
        let stats = CollectionStats {
            name: "users".to_string(),
            document_count: 1000,
            chunk_count: 50,
            disk_usage: DiskUsage {
                sst_files_size: 1024 * 1024,
                live_data_size: 512 * 1024,
                num_sst_files: 5,
                memtable_size: 64 * 1024,
            },
        };

        assert_eq!(stats.name, "users");
        assert_eq!(stats.document_count, 1000);
        assert_eq!(stats.disk_usage.num_sst_files, 5);
    }

    #[test]
    fn test_disk_usage_serialization() {
        let usage = DiskUsage {
            sst_files_size: 100,
            live_data_size: 50,
            num_sst_files: 2,
            memtable_size: 10,
        };

        let json = serde_json::to_string(&usage).unwrap();
        let deserialized: DiskUsage = serde_json::from_str(&json).unwrap();
        
        assert_eq!(usage.sst_files_size, deserialized.sst_files_size);
        assert_eq!(usage.num_sst_files, deserialized.num_sst_files);
    }

    #[test]
    fn test_default_min_length() {
        assert_eq!(default_min_length(), 3);
    }

    #[test]
    fn test_fulltext_index_deserialization() {
        let json = r#"{"name": "ft_idx", "field": "content", "min_length": 2}"#;
        let idx: FulltextIndex = serde_json::from_str(json).unwrap();
        
        assert_eq!(idx.name, "ft_idx");
        assert_eq!(idx.fields.len(), 1);
        assert_eq!(idx.min_length, 2);
    }

    #[test]
    fn test_fulltext_index_default_min_length() {
        let json = r#"{"name": "ft_idx", "field": "content"}"#;
        let idx: FulltextIndex = serde_json::from_str(json).unwrap();
        
        assert_eq!(idx.min_length, 3); // default
    }
}


