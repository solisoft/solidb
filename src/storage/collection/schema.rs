use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::schema::CollectionSchema;
use seahash::SeaHasher;
use std::hash::Hasher;

impl Collection {
    // ==================== Schema Management ====================

    /// Compute hash of schema for cache invalidation
    fn compute_schema_hash(schema: &CollectionSchema) -> u64 {
        let serialized = serde_json::to_vec(schema).unwrap_or_default();
        let mut hasher = SeaHasher::new();
        hasher.write(&serialized);
        hasher.finish()
    }

    /// Get or create cached schema validator
    pub fn get_cached_schema_validator(&self) -> Result<Option<SchemaValidator>, DbError> {
        if let Some(schema) = self.get_json_schema() {
            let schema_hash = Self::compute_schema_hash(&schema);

            let mut cached_hash = self.schema_hash.write().unwrap();
            let mut cached_validator = self.schema_validator.write().unwrap();

            if let Some(ref current_hash) = *cached_hash {
                if *current_hash == schema_hash {
                    if let Some(ref validator) = *cached_validator {
                        return Ok(Some(validator.clone()));
                    }
                }
            }

            let validator = SchemaValidator::new(schema).map_err(|e| {
                DbError::InvalidDocument(format!("Schema compilation error: {}", e))
            })?;

            *cached_hash = Some(schema_hash);
            *cached_validator = Some(validator.clone());

            Ok(Some(validator))
        } else {
            Ok(None)
        }
    }

    /// Invalidate schema cache (called when schema changes)
    fn invalidate_schema_cache(&self) {
        let mut cached_hash = self.schema_hash.write().unwrap();
        let mut cached_validator = self.schema_validator.write().unwrap();
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

        let db = self.db.write().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let schema_bytes = serde_json::to_vec(&schema)?;
        db.put_cf(cf, SCHEMA_KEY.as_bytes(), &schema_bytes)
            .map_err(|e| DbError::InternalError(format!("Failed to set schema: {}", e)))?;

        Ok(())
    }

    /// Get JSON schema
    pub fn get_json_schema(&self) -> Option<CollectionSchema> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        db.get_cf(cf, SCHEMA_KEY.as_bytes())
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Remove JSON schema
    pub fn remove_json_schema(&self) -> DbResult<()> {
        self.invalidate_schema_cache();

        let db = self.db.write().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        db.delete_cf(cf, SCHEMA_KEY.as_bytes())
            .map_err(|e| DbError::InternalError(format!("Failed to remove schema: {}", e)))?;

        Ok(())
    }
}
