use dashmap::DashMap;
use rocksdb::DB;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, RwLock};

pub use super::document::Document;
pub use super::geo::{GeoIndex, GeoIndexStats};
pub use super::index::{
    FulltextMatch, Index, IndexStats, IndexType, TtlIndex, TtlIndexStats, VectorIndexConfig,
    VectorIndexStats, VectorQuantization,
};
pub use super::schema::SchemaValidator;
pub use super::vector::{VectorIndex, VectorSearchResult};
use cuckoofilter::CuckooFilter;
use fastbloom::BloomFilter;

pub mod blobs;
pub mod core;
pub mod crud;
pub mod fulltext;
pub mod geo;
pub mod indexes;
pub mod schema;
pub mod ttl;
pub mod txn;
pub mod vector;
pub use self::vector::QuantizationStats;

/// Key prefixes for different data types
pub const DOC_PREFIX: &str = "doc:";
pub const IDX_PREFIX: &str = "idx:";
pub const IDX_META_PREFIX: &str = "idx_meta:";
pub const GEO_PREFIX: &str = "geo:";
pub const GEO_META_PREFIX: &str = "geo_meta:";
pub const FT_PREFIX: &str = "ft:"; // Fulltext n-gram entries
pub const FT_META_PREFIX: &str = "ft_meta:"; // Fulltext index metadata
pub const FT_TERM_PREFIX: &str = "ft_term:"; // Fulltext term → doc mapping
pub const STATS_COUNT_KEY: &str = "_stats:count"; // Document count
pub const SHARD_CONFIG_KEY: &str = "_stats:shard_config"; // Sharding configuration
pub const SHARD_TABLE_KEY: &str = "_stats:shard_table"; // Sharding assignment table
pub const COLLECTION_TYPE_KEY: &str = "_stats:type"; // Collection type (document, edge)
pub const BLO_PREFIX: &str = "blo:"; // Blob chunk prefix
pub const TTL_META_PREFIX: &str = "ttl_meta:"; // TTL index metadata
pub const TTL_EXPIRY_PREFIX: &str = "ttl_exp:"; // TTL expiry index (expiry_timestamp -> doc_key)

pub const BLO_IDX_PREFIX: &str = "blo_idx:"; // Bloom filter index prefix
pub const CFO_IDX_PREFIX: &str = "cfo_idx:"; // Cuckoo filter index prefix
pub const SCHEMA_KEY: &str = "_stats:schema"; // JSON Schema for validation
pub const VEC_META_PREFIX: &str = "vec_meta:"; // Vector index metadata
pub const VEC_DATA_PREFIX: &str = "vec_data:"; // Vector index data (serialized VectorIndex)
pub const NGRAM_SIZE: usize = 3;

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
pub struct FulltextIndex {
    pub name: String,
    #[serde(
        alias = "field",
        deserialize_with = "crate::storage::index::deserialize_fields"
    )]
    pub fields: Vec<String>,
    #[serde(default = "default_min_length")]
    pub min_length: usize,
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
    /// RocksDB instance - thread-safe for reads and writes
    pub(crate) db: Arc<DB>,
    /// Cached document count (atomic for lock-free updates)
    pub(crate) doc_count: Arc<AtomicUsize>,
    /// Cached blob chunk count (atomic for lock-free updates)
    pub(crate) chunk_count: Arc<AtomicUsize>,
    /// Whether count needs to be persisted to disk
    pub(crate) count_dirty: Arc<AtomicBool>,
    /// Last flush time in seconds since UNIX epoch (for throttling)
    pub(crate) last_flush_time: Arc<std::sync::atomic::AtomicU64>,
    /// Broadcast channel for real-time change events
    pub change_sender: Arc<tokio::sync::broadcast::Sender<ChangeEvent>>,
    /// Collection type (document, edge, blob)
    pub(crate) collection_type: Arc<RwLock<String>>,
    /// In-memory Bloom filters for indexes (DashMap for lock-free concurrent access)
    pub(crate) bloom_filters: Arc<DashMap<String, BloomFilter>>,
    /// In-memory Cuckoo filters for indexes (DashMap for lock-free concurrent access)
    pub(crate) cuckoo_filters: Arc<DashMap<String, CuckooFilter<DefaultHasher>>>,
    /// In-memory vector indexes (DashMap for lock-free concurrent access)
    pub(crate) vector_indexes: Arc<DashMap<String, Arc<VectorIndex>>>,
    /// Cached compiled schema validator
    pub(crate) schema_validator: Arc<RwLock<Option<SchemaValidator>>>,
    /// Hash of cached schema for invalidation detection
    pub(crate) schema_hash: Arc<RwLock<Option<u64>>>,
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
            bloom_filters: self.bloom_filters.clone(),
            cuckoo_filters: self.cuckoo_filters.clone(),
            vector_indexes: self.vector_indexes.clone(),
            schema_validator: self.schema_validator.clone(),
            schema_hash: self.schema_hash.clone(),
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
