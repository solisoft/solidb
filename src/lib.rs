pub mod cli;
pub mod cluster;
pub mod driver;
pub mod error;
pub mod queue;
pub mod scripting;
pub mod sdbql;
pub mod server;
pub mod sharding;
pub mod sql;
pub mod storage;
pub mod transaction;
pub mod triggers;
pub mod ttl;

// Synchronization module (new architecture)
pub mod sync;

// AI-augmented database module
pub mod ai;

// Stream Processing module
pub mod stream;

// Schema validation module
pub use storage::schema::{
    CollectionSchema as JsonSchema, SchemaCompilationError, SchemaValidationError,
    SchemaValidationMode, SchemaValidator, ValidationResult, ValidationViolation,
};

pub use error::{DbError, DbResult};
pub use sdbql::{parse, BindVars, QueryExecutor, QueryExplain};
pub use server::create_router;
pub use storage::{
    distance_meters,
    // Columnar storage types
    AggregateOp,
    Collection,
    ColumnDef,
    ColumnFilter,
    ColumnType,
    ColumnarCollection,
    ColumnarCollectionMeta,
    ColumnarStats,
    CompressionType,
    Document,
    GeoIndex,
    GeoIndexStats,
    GeoPoint,
    Index,
    IndexStats,
    IndexType,
    StorageEngine,
    TtlIndex,
    TtlIndexStats,
};
pub use transaction::{
    manager::TransactionManager, IsolationLevel, Operation, Transaction, TransactionId,
    TransactionState,
};
