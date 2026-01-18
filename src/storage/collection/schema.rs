use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::schema::CollectionSchema;

impl Collection {
    // ==================== Schema Management ====================

    /// Set JSON schema for collection validation
    /// Set JSON schema for collection validation
    pub fn set_json_schema(&self, schema: CollectionSchema) -> DbResult<()> {
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
        let db = self.db.write().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        db.delete_cf(cf, SCHEMA_KEY.as_bytes())
            .map_err(|e| DbError::InternalError(format!("Failed to remove schema: {}", e)))?;

        Ok(())
    }
}
