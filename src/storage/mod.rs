/// RocksDB handle type used throughout the codebase.
///
/// `MultiThreaded` mode synchronizes the column-family handle map internally
/// (`create_cf`/`drop_cf` take `&self`), so CF creation/drop no longer needs
/// `unsafe` mutable casts of `Arc<DB>` — which raced against the lock-free
/// `cf_handle()` reads done all over the codebase.
pub type RocksDb = rust_rocksdb::DBWithThreadMode<rust_rocksdb::MultiThreaded>;

pub mod codec;
pub mod collection;
pub mod columnar;
pub mod database;
pub mod document;
pub mod document_cache;
pub mod engine;
pub mod geo;
pub mod http_client;
pub mod index;
pub mod query_cache;
pub mod schema;
pub mod serializer;
pub mod vector;

pub use collection::{Collection, CollectionStats, DiskUsage};
pub use columnar::{
    AggregateOp, ColumnDef, ColumnFilter, ColumnType, ColumnarCollection, ColumnarCollectionMeta,
    ColumnarStats, CompressionType,
};
pub use database::Database;
pub use document::Document;
pub use engine::StorageEngine;
pub use geo::{distance_meters, GeoIndex, GeoIndexStats, GeoPoint};
pub use index::{
    bm25_score, calculate_idf, deserialize_fields, extract_field_value, generate_ngrams,
    levenshtein_distance, ngram_similarity, normalize_text, tokenize, FulltextMatch, Index,
    IndexStats, IndexType, TtlIndex, TtlIndexStats, VectorIndexConfig, VectorIndexStats,
    VectorMetric, BM25_B, BM25_K1, NGRAM_SIZE,
};
pub use schema::{
    CollectionSchema, SchemaCompilationError, SchemaValidationError, SchemaValidationMode,
    SchemaValidator, ValidationResult, ValidationViolation,
};
pub use vector::{VectorIndex, VectorSearchResult};
