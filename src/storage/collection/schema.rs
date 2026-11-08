use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::keyspace::KsId;
use crate::storage::schema::CollectionSchema;
use crate::storage::RocksDb as DB;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use seahash::SeaHasher;
use std::hash::Hasher;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

/// A collection's stored schema with its hash, as last read from disk.
pub(super) type StoredSchema = Option<Arc<(u64, CollectionSchema)>>;

struct SchemaEntry {
    /// Pins the DB allocation, so an equal pointer is the same live instance.
    db: Weak<DB>,
    /// [`SCHEMA_EPOCH`] when the schema was read.
    epoch: u64,
    schema: StoredSchema,
}

/// Every write checks the collection's schema, and reading it was a point
/// lookup (plus a parse, a re-serialization and a hash when one is set) per
/// document. It is cached per shared keyspace — ids are never reused, so a
/// recreated collection cannot see its predecessor's schema — and an entry is
/// trusted only while [`SCHEMA_EPOCH`] has not moved since it was read. Setting
/// or removing any schema moves it, after the write: a read that raced the
/// change is stored under the old epoch and never served. Legacy (per-CF)
/// collections, whose id is a reusable name, are not cached.
static SCHEMAS: Lazy<DashMap<(usize, KsId), SchemaEntry>> = Lazy::new(DashMap::new);
static SCHEMA_EPOCH: AtomicU64 = AtomicU64::new(0);

impl Collection {
    // ==================== Schema Management ====================

    /// Compute hash of schema for cache invalidation
    fn compute_schema_hash(schema: &CollectionSchema) -> u64 {
        let serialized = serde_json::to_vec(schema).unwrap_or_default();
        let mut hasher = SeaHasher::new();
        hasher.write(&serialized);
        hasher.finish()
    }

    /// The stored schema and its hash, from the cache when it is current.
    pub(super) fn stored_schema(&self) -> StoredSchema {
        let cacheable = matches!(self.ks.id(), KsId::Shared(_));
        let epoch = SCHEMA_EPOCH.load(Ordering::Acquire);
        let key = (Arc::as_ptr(&self.db) as usize, self.ks.id().clone());
        if cacheable {
            if let Some(entry) = SCHEMAS.get(&key) {
                if entry.epoch == epoch && Weak::as_ptr(&entry.db) == Arc::as_ptr(&self.db) {
                    return entry.schema.clone();
                }
            }
        }
        let schema = self
            .get_json_schema()
            .map(|s| Arc::new((Self::compute_schema_hash(&s), s)));
        if cacheable {
            SCHEMAS.insert(
                key,
                SchemaEntry {
                    db: Arc::downgrade(&self.db),
                    epoch,
                    schema: schema.clone(),
                },
            );
        }
        schema
    }

    /// Get or create cached schema validator
    pub fn get_cached_schema_validator(&self) -> Result<Option<SchemaValidator>, DbError> {
        if let Some(stored) = self.stored_schema() {
            let (schema_hash, ref schema) = *stored;

            // Fast path: read guards only (parking_lot's RwLock never
            // poisons, so no .unwrap() is needed).
            {
                let cached_hash = self.schema_hash.read();
                let cached_validator = self.schema_validator.read();
                if let Some(ref current_hash) = *cached_hash {
                    if *current_hash == schema_hash {
                        if let Some(ref validator) = *cached_validator {
                            return Ok(Some(validator.clone()));
                        }
                    }
                }
            }

            let validator = SchemaValidator::new(schema.clone()).map_err(|e| {
                DbError::InvalidDocument(format!("Schema compilation error: {}", e))
            })?;

            // Hold BOTH write guards while setting the pair so the hash and
            // validator can never be observed (or written) mismatched when
            // two threads race with different schema versions. Lock order is
            // always hash -> validator.
            {
                let mut cached_hash = self.schema_hash.write();
                let mut cached_validator = self.schema_validator.write();
                *cached_hash = Some(schema_hash);
                *cached_validator = Some(validator.clone());
            }

            Ok(Some(validator))
        } else {
            Ok(None)
        }
    }

    /// Invalidate schema cache (called when schema changes).
    /// Same lock order as the setter (hash -> validator), both held together.
    fn invalidate_schema_cache(&self) {
        let mut cached_hash = self.schema_hash.write();
        let mut cached_validator = self.schema_validator.write();
        *cached_hash = None;
        *cached_validator = None;
    }

    /// Set JSON schema for collection validation
    /// Set JSON schema for collection validation
    pub fn set_json_schema(&self, schema: CollectionSchema) -> DbResult<()> {
        self.invalidate_schema_cache();

        // Validate the schema itself if enabled
        if schema.is_enabled() {
            jsonschema::validator_for(&schema.schema)
                .map_err(|e| DbError::InvalidDocument(format!("Invalid JSON Schema: {}", e)))?;
        }

        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        let schema_bytes = serde_json::to_vec(&schema)?;
        let written = db
            .put_ks(&cf, SCHEMA_KEY.as_bytes(), &schema_bytes)
            .map_err(|e| DbError::InternalError(format!("Failed to set schema: {}", e)));
        SCHEMA_EPOCH.fetch_add(1, Ordering::AcqRel);
        written?;

        Ok(())
    }

    /// Get JSON schema
    pub fn get_json_schema(&self) -> Option<CollectionSchema> {
        let db = &self.db;
        let cf = self.ks.handle(&self.db)?;

        db.get_ks(&cf, SCHEMA_KEY.as_bytes())
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Remove JSON schema
    pub fn remove_json_schema(&self) -> DbResult<()> {
        self.invalidate_schema_cache();

        let db = &self.db;
        let cf = self
            .ks
            .handle(&self.db)
            .expect("Column family should exist");

        let removed = db
            .delete_ks(&cf, SCHEMA_KEY.as_bytes())
            .map_err(|e| DbError::InternalError(format!("Failed to remove schema: {}", e)));
        SCHEMA_EPOCH.fetch_add(1, Ordering::AcqRel);
        removed?;

        Ok(())
    }
}
