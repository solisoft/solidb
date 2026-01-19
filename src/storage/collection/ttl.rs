use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::index::{extract_field_value, TtlIndex, TtlIndexStats};

impl Collection {
    // ==================== TTL Index Operations ====================

    /// Get all TTL indexes
    pub fn get_all_ttl_indexes(&self) -> Vec<TtlIndex> {
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
    pub fn get_ttl_index(&self, name: &str) -> Option<TtlIndex> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        db.get_cf(cf, Self::ttl_meta_key(name))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Create a TTL index
    pub fn create_ttl_index(
        &self,
        name: String,
        field: String,
        expire_after_seconds: u64,
    ) -> DbResult<TtlIndexStats> {
        if self.get_ttl_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!(
                "TTL Index '{}' already exists",
                name
            )));
        }

        let index = TtlIndex {
            name: name.clone(),
            field: field.clone(),
            expire_after_seconds,
        };
        let index_bytes = serde_json::to_vec(&index)?;

        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::ttl_meta_key(&name), &index_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create TTL index: {}", e))
                })?;
        }

        // Trigger an initial cleanup?
        // self.cleanup_expired_documents_for_ttl_index(&index)?;
        // Better to let the user or background job trigger it.

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
                "TTL Index '{}' not found",
                name
            )));
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        db.delete_cf(cf, Self::ttl_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop TTL index: {}", e)))?;

        Ok(())
    }

    /// List all TTL indexes
    pub fn list_ttl_indexes(&self) -> Vec<TtlIndexStats> {
        self.get_all_ttl_indexes()
            .into_iter()
            .map(|idx| TtlIndexStats {
                name: idx.name,
                field: idx.field,
                expire_after_seconds: idx.expire_after_seconds,
            })
            .collect()
    }

    /// Cleanup expired documents for a specific TTL index
    pub fn cleanup_expired_documents_for_ttl_index(&self, index: &TtlIndex) -> DbResult<usize> {
        // Naive implementation: scan all docs.
        // Optimized: Secondary index on timestamp?
        // Current architecture: Scan all.

        let docs = self.all();
        let mut deleted_count = 0;
        let now = chrono::Utc::now().timestamp() as u64;

        for doc in docs {
            let doc_value = doc.to_value();
            let field_value = extract_field_value(&doc_value, &index.field);

            // Check format (number vs ISO string)
            let expiry_time = if let Some(n) = field_value.as_u64() {
                // Assuming timestamp in seconds? or milliseconds?
                // Usually seconds for unix timestamp.
                Some(n)
            } else if let Some(s) = field_value.as_str() {
                // Try parse ISO string
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                    Some(dt.timestamp() as u64)
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(ts) = expiry_time {
                if now > ts + index.expire_after_seconds && self.delete(&doc.key).is_ok() {
                    deleted_count += 1;
                }
            }
        }

        Ok(deleted_count)
    }

    /// Cleanup all expired documents across all TTL indexes
    pub fn cleanup_all_expired_documents(&self) -> DbResult<usize> {
        let indexes = self.get_all_ttl_indexes();
        let mut total_deleted = 0;
        for index in indexes {
            total_deleted += self.cleanup_expired_documents_for_ttl_index(&index)?;
        }
        Ok(total_deleted)
    }
}
