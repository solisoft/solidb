pub mod sdbql;
pub mod cluster;
pub mod error;
pub mod server;
pub mod storage;
pub mod transaction;
pub mod scripting;
pub mod sharding;
pub mod queue;
pub mod ttl;
pub mod driver;

// Synchronization module (new architecture)
pub mod sync;

// Schema validation module
pub use storage::schema::{CollectionSchema as JsonSchema, SchemaValidationMode, SchemaValidator, SchemaCompilationError, SchemaValidationError, ValidationViolation, ValidationResult};

pub use sdbql::{parse, BindVars, QueryExecutor, QueryExplain};
pub use error::{DbError, DbResult};
pub use server::create_router;
pub use storage::{
    distance_meters, Collection, Document, GeoIndex, GeoIndexStats, GeoPoint, Index, IndexStats,
    IndexType, StorageEngine, TtlIndex, TtlIndexStats,
};
pub use transaction::{
    manager::TransactionManager, IsolationLevel, Operation, Transaction, TransactionId,
    TransactionState,
};
