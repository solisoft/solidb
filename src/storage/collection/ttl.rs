use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::index::{extract_field_value, TtlIndex, TtlIndexStats};
use rocksdb::WriteBatch;
use std::time::{SystemTime, UNIX_EPOCH};

type TtlExpiryEntry = (Vec<u8>, Vec<u8>);
type TtlExpiryEntries = Vec<TtlExpiryEntry>;
type TtlExpiryKeys = Vec<Vec<u8>>;

impl Collection {
    // ==================== TTL Index Operations ====================

    /// Get all TTL indexes
    pub fn get_all_ttl_indexes(&self) -> Vec<TtlIndex> {
        let db = &self.db;
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
        let db = &self.db;
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
            let db = &self.db;
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

        let db = &self.db;
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

    // ==================== TTL Expiry Index Management (for efficient cleanup) ====================

    /// Compute TTL expiry index entries for a document insert
    /// Returns Vec<(key, value)> where value is empty - suitable for WriteBatch
    pub(crate) fn compute_ttl_expiry_entries_for_insert(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> TtlExpiryEntries {
        let ttl_indexes = self.get_all_ttl_indexes();
        let mut entries = Vec::new();

        for ttl_index in &ttl_indexes {
            if let Some(expiry_time) = Self::extract_expiry_time(doc_value, &ttl_index.field) {
                // Calculate when this document will expire
                let expires_at = expiry_time + ttl_index.expire_after_seconds;
                let entry_key = Self::ttl_expiry_key(&ttl_index.name, expires_at, doc_key);
                entries.push((entry_key, Vec::new()));
            }
        }

        entries
    }

    /// Compute TTL expiry index entries for a document update
    /// Returns (entries_to_add, keys_to_remove)
    pub(crate) fn compute_ttl_expiry_entries_for_update(
        &self,
        doc_key: &str,
        old_value: &Value,
        new_value: &Value,
    ) -> (TtlExpiryEntries, TtlExpiryKeys) {
        let ttl_indexes = self.get_all_ttl_indexes();
        let mut entries_to_add = Vec::new();
        let mut keys_to_remove = Vec::new();

        // Get old expiry entries for removal
        for ttl_index in &ttl_indexes {
            if let Some(old_expiry) = Self::extract_expiry_time(old_value, &ttl_index.field) {
                let old_expires_at = old_expiry + ttl_index.expire_after_seconds;
                let old_key = Self::ttl_expiry_key(&ttl_index.name, old_expires_at, doc_key);
                keys_to_remove.push(old_key);
            }
        }

        // Get new expiry entries for addition
        for ttl_index in &ttl_indexes {
            if let Some(new_expiry) = Self::extract_expiry_time(new_value, &ttl_index.field) {
                let new_expires_at = new_expiry + ttl_index.expire_after_seconds;
                let new_key = Self::ttl_expiry_key(&ttl_index.name, new_expires_at, doc_key);
                entries_to_add.push((new_key, Vec::new()));
            }
        }

        (entries_to_add, keys_to_remove)
    }

    /// Compute TTL expiry index entries for a document delete
    /// Returns keys to remove
    pub(crate) fn compute_ttl_expiry_entries_for_delete(
        &self,
        doc_key: &str,
        doc_value: &Value,
    ) -> TtlExpiryKeys {
        let ttl_indexes = self.get_all_ttl_indexes();
        let mut keys_to_remove = Vec::new();

        for ttl_index in &ttl_indexes {
            if let Some(expiry_time) = Self::extract_expiry_time(doc_value, &ttl_index.field) {
                let expires_at = expiry_time + ttl_index.expire_after_seconds;
                let key = Self::ttl_expiry_key(&ttl_index.name, expires_at, doc_key);
                keys_to_remove.push(key);
            }
        }

        keys_to_remove
    }

    /// Extract expiry timestamp from document field
    fn extract_expiry_time(doc_value: &Value, field: &str) -> Option<u64> {
        let field_value = extract_field_value(doc_value, field);

        if let Some(n) = field_value.as_u64() {
            Some(n)
        } else if let Some(s) = field_value.as_str() {
            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                Some(dt.timestamp() as u64)
            } else {
                None
            }
        } else {
            None
        }
    }

    /// Cleanup expired documents for a specific TTL index using expiry index
    /// This is O(n) where n = expired documents (not all documents)
    pub fn cleanup_expired_documents_for_ttl_index(&self, index: &TtlIndex) -> DbResult<usize> {
        const BATCH_SIZE: usize = 1000;

        // Use SystemTime for consistency with test timestamps
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Iterate through expiry index up to the threshold
        // Key format: "doc:ttl_exp:<ttl_index_name>:<expiry_ts>:<doc_key>"
        // We want all entries where expiry_ts <= expire_threshold
        let prefix = Self::ttl_expiry_prefix(&index.name);

        let mut expired_doc_keys: Vec<String> = Vec::new();
        let mut expired_expiry_keys: Vec<Vec<u8>> = Vec::new();

        let iter = db.prefix_iterator_cf(cf, prefix.as_slice());

        for result in iter.flatten() {
            let (key_bytes, _value) = result;
            if !key_bytes.starts_with(prefix.as_slice()) {
                break;
            }

            // Parse expiry timestamp from key
            // Format: doc:ttl_exp:<ttl_index_name>:<expiry_ts>:<doc_key>
            // Skip "doc:ttl_exp:<ttl_index_name>:" to get to expiry_ts
            let prefix_len = prefix.len();
            let after_prefix = &key_bytes[prefix_len..];

            // Find the next colon which separates expiry_ts from doc_key
            let colon_pos = after_prefix.iter().position(|&b| b == b':').unwrap_or(0);

            if colon_pos > 0 {
                let ts_str = String::from_utf8_lossy(&after_prefix[..colon_pos]);
                if let Ok(expiry_ts) = ts_str.parse::<u64>() {
                    if expiry_ts <= now {
                        // Parse doc_key from remaining bytes
                        let doc_key =
                            String::from_utf8_lossy(&after_prefix[colon_pos + 1..]).to_string();
                        expired_doc_keys.push(doc_key);
                        expired_expiry_keys.push(key_bytes.to_vec());
                    } else {
                        // Entries are sorted by expiry timestamp, so we can stop
                        break;
                    }
                }
            }
        }

        drop(db);

        if expired_doc_keys.is_empty() {
            return Ok(0);
        }

        // Delete documents and expiry entries in batches
        let mut deleted_count: usize = 0;
        for chunk in expired_doc_keys.chunks(BATCH_SIZE) {
            let db = &self.db;
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            let mut batch = WriteBatch::default();

            let base_idx = deleted_count;
            for (i, key) in chunk.iter().enumerate() {
                // Delete document
                batch.delete_cf(cf, Self::doc_key(key));

                // Delete expiry index entry
                let expiry_idx = base_idx.saturating_add(i);
                if let Some(expiry_key) = expired_expiry_keys.get(expiry_idx) {
                    batch.delete_cf(cf, expiry_key);
                }

                deleted_count += 1;
            }

            let db = &self.db;
            db.write(batch)?;
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
