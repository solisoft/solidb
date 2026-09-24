use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::index::{extract_field_value, TtlIndex, TtlIndexStats};
use rust_rocksdb::WriteBatch;
use std::time::{SystemTime, UNIX_EPOCH};

type TtlExpiryEntry = (Vec<u8>, Vec<u8>);
type TtlExpiryEntries = Vec<TtlExpiryEntry>;
type TtlExpiryKeys = Vec<Vec<u8>>;

impl Collection {
    // ==================== TTL Index Operations ====================

    /// Get all TTL indexes
    pub fn get_all_ttl_indexes(&self) -> Vec<TtlIndex> {
        // Empty when the column family is gone (dropped mid-operation): a
        // background caller such as the TTL worker must not panic (audit P11).
        self.index_meta().map(|m| m.ttl.clone()).unwrap_or_default()
    }

    /// Get a TTL index by name
    pub fn get_ttl_index(&self, name: &str) -> Option<TtlIndex> {
        self.index_meta()?
            .ttl
            .iter()
            .find(|i| i.name == name)
            .cloned()
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
            db.put_cf(&cf, Self::ttl_meta_key(&name), &index_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create TTL index: {}", e))
                })?;
        }
        self.invalidate_index_meta();

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

        db.delete_cf(&cf, Self::ttl_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop TTL index: {}", e)))?;
        self.invalidate_index_meta();

        // Drop the index's expiry entries, current and legacy, so a later
        // index of the same name does not inherit them.
        let mut batch = WriteBatch::default();
        let prefix = Self::ttl_expiry_prefix(name);
        for (key, _) in db.prefix_iterator_cf(&cf, prefix.as_slice()).flatten() {
            if !key.starts_with(prefix.as_slice()) {
                break;
            }
            if Self::parse_expiry_suffix(&key[prefix.len()..]).is_some() {
                batch.delete_cf(&cf, &key);
            }
        }
        let legacy_prefix = Self::legacy_ttl_expiry_prefix(name);
        for (key, value) in db
            .prefix_iterator_cf(&cf, legacy_prefix.as_slice())
            .flatten()
        {
            if !key.starts_with(legacy_prefix.as_slice()) {
                break;
            }
            // Empty value only: a non-empty one is a document.
            if value.is_empty() {
                batch.delete_cf(&cf, &key);
            }
        }
        if !batch.is_empty() {
            db.write(&batch).map_err(|e| {
                DbError::InternalError(format!("Failed to drop TTL expiry entries: {}", e))
            })?;
        }

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
                let expires_at = expiry_time.saturating_add(ttl_index.expire_after_seconds);
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
                let old_expires_at = old_expiry.saturating_add(ttl_index.expire_after_seconds);
                let old_key = Self::ttl_expiry_key(&ttl_index.name, old_expires_at, doc_key);
                keys_to_remove.push(old_key);
            }
        }

        // Get new expiry entries for addition
        for ttl_index in &ttl_indexes {
            if let Some(new_expiry) = Self::extract_expiry_time(new_value, &ttl_index.field) {
                let new_expires_at = new_expiry.saturating_add(ttl_index.expire_after_seconds);
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
                let expires_at = expiry_time.saturating_add(ttl_index.expire_after_seconds);
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

    /// Parse `<20-digit ts>:<doc_key>` (current expiry entries).
    fn parse_expiry_suffix(rest: &[u8]) -> Option<(u64, String)> {
        if rest.len() < 22 || rest[20] != b':' || !rest[..20].iter().all(u8::is_ascii_digit) {
            return None;
        }
        let ts = std::str::from_utf8(&rest[..20]).ok()?.parse().ok()?;
        Some((ts, String::from_utf8_lossy(&rest[21..]).into_owned()))
    }

    /// Parse `<ts>:<doc_key>` (legacy, unpadded expiry entries).
    fn parse_legacy_expiry_suffix(rest: &[u8]) -> Option<(u64, String)> {
        let colon = rest.iter().position(|&b| b == b':')?;
        if colon == 0 || !rest[..colon].iter().all(u8::is_ascii_digit) {
            return None;
        }
        let ts = std::str::from_utf8(&rest[..colon]).ok()?.parse().ok()?;
        Some((ts, String::from_utf8_lossy(&rest[colon + 1..]).into_owned()))
    }

    /// Cleanup expired documents for a specific TTL index using expiry index
    /// This is O(n) where n = expired documents (not all documents)
    ///
    /// Audit H8: an expiry entry is only a hint. Before deleting, the reaper
    /// re-reads the document under its key stripe and recomputes its expiry
    /// for this index — a stale entry (the document was updated or deleted
    /// since) or a forged one is dropped without touching any document.
    /// Deletion goes through `delete_docs_locked`, so index / fulltext / geo /
    /// vector entries, blobs, the count and change events are handled exactly
    /// as for a client delete. Pre-H8 entries under `doc:ttl_exp::` are
    /// reaped the same way and otherwise migrated to the new prefix.
    ///
    /// Replication and triggers are driven at the handler level, so reaper
    /// deletions are not logged there; each node reaps its own copy.
    pub fn cleanup_expired_documents_for_ttl_index(&self, index: &TtlIndex) -> DbResult<usize> {
        const BATCH_SIZE: usize = 1000;
        /// Legacy entries migrated per pass, bounding one pass's work.
        const LEGACY_PER_PASS: usize = 10_000;

        // Use SystemTime for consistency with test timestamps
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let db = &self.db;
        let Some(cf) = db.cf_handle(&self.name) else {
            return Ok(0); // collection dropped mid-pass
        };

        // (expiry entry key, doc_key) for entries whose timestamp has passed
        let mut expired: Vec<(Vec<u8>, String)> = Vec::new();

        // Current entries: "ttl_exp:<index>:<ts, 20 digits>:<doc_key>", sorted
        // by timestamp, so the scan stops at the first unexpired one.
        let prefix = Self::ttl_expiry_prefix(&index.name);
        for (key_bytes, _) in db.prefix_iterator_cf(&cf, prefix.as_slice()).flatten() {
            if !key_bytes.starts_with(prefix.as_slice()) {
                break;
            }
            // Entries of another index whose name extends this one fail to
            // parse and are skipped.
            let Some((expiry_ts, doc_key)) = Self::parse_expiry_suffix(&key_bytes[prefix.len()..])
            else {
                continue;
            };
            if expiry_ts > now {
                break;
            }
            expired.push((key_bytes.to_vec(), doc_key));
        }

        // Legacy entries: "doc:ttl_exp::<index>:<ts>:<doc_key>" with an empty
        // value. A non-empty value is a real document whose `_key` merely
        // looks like an entry (the H8 forgery) and is never acted on.
        let legacy_prefix = Self::legacy_ttl_expiry_prefix(&index.name);
        let mut migrate = WriteBatch::default();
        let mut legacy_seen = 0usize;
        for (key_bytes, value) in db
            .prefix_iterator_cf(&cf, legacy_prefix.as_slice())
            .flatten()
        {
            if !key_bytes.starts_with(legacy_prefix.as_slice()) || legacy_seen >= LEGACY_PER_PASS {
                break;
            }
            if !value.is_empty() {
                continue;
            }
            let Some((expiry_ts, doc_key)) =
                Self::parse_legacy_expiry_suffix(&key_bytes[legacy_prefix.len()..])
            else {
                continue;
            };
            legacy_seen += 1;
            if expiry_ts <= now {
                expired.push((key_bytes.to_vec(), doc_key));
            } else {
                migrate.put_cf(
                    &cf,
                    Self::ttl_expiry_key(&index.name, expiry_ts, &doc_key),
                    b"",
                );
                migrate.delete_cf(&cf, &key_bytes);
            }
        }
        if !migrate.is_empty() {
            db.write(&migrate)?;
        }
        drop(cf);

        let mut deleted_count: usize = 0;
        for chunk in expired.chunks(BATCH_SIZE) {
            let _key_guard = self.lock_keys(chunk.iter().map(|(_, k)| k.as_str()));

            let mut docs = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for (_, doc_key) in chunk {
                if !seen.insert(doc_key.as_str()) {
                    continue;
                }
                let Ok(doc) = self.get(doc_key) else {
                    continue; // already gone: only the entry is left to drop
                };
                let doc_value = doc.to_value();
                let still_expired = Self::extract_expiry_time(&doc_value, &index.field)
                    .is_some_and(|t| t.saturating_add(index.expire_after_seconds) <= now);
                if still_expired {
                    docs.push((doc_key.clone(), doc_value));
                }
            }

            // The consumed entries go in the same batch. A live document's
            // current entry (if any) is re-derived from its value by the
            // delete, or left alone when the document survives.
            let entries: Vec<Vec<u8>> = chunk.iter().map(|(k, _)| k.clone()).collect();
            deleted_count += self.delete_docs_locked(docs, entries)?;
        }
        if deleted_count > 0 {
            self.persist_vector_indexes_throttled();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_suffix_parsing() {
        let key = Collection::ttl_expiry_key("idx", 1_700_000_000, "a:b");
        let prefix = Collection::ttl_expiry_prefix("idx");
        assert_eq!(
            Collection::parse_expiry_suffix(&key[prefix.len()..]),
            Some((1_700_000_000, "a:b".to_string()))
        );
        // Zero padding keeps lexicographic order numeric.
        assert!(
            Collection::ttl_expiry_key("idx", 99, "k")
                < Collection::ttl_expiry_key("idx", 100, "k")
        );
        // Entries of an index named "idx:x" do not parse under "idx".
        let other = Collection::ttl_expiry_key("idx:x", 5, "k");
        assert_eq!(
            Collection::parse_expiry_suffix(&other[prefix.len()..]),
            None
        );
        // Expiry entries are outside the document namespace (audit H8).
        assert!(!key.starts_with(DOC_PREFIX.as_bytes()));

        assert_eq!(
            Collection::parse_legacy_expiry_suffix(b"12:victim"),
            Some((12, "victim".to_string()))
        );
        assert_eq!(Collection::parse_legacy_expiry_suffix(b"x:victim"), None);
    }

    #[test]
    fn legacy_expiry_entries_are_reaped_or_migrated_and_not_counted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = crate::storage::StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
        engine.create_collection("s".to_string(), None).unwrap();
        let coll = engine.get_collection("s").unwrap();
        coll.create_ttl_index("ttl".to_string(), "t".to_string(), 1)
            .unwrap();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        // Documents written without the index's entries, then pre-H8 entries
        // planted by hand, as an older version would have left them.
        coll.insert_no_index(serde_json::json!({"_key": "old", "t": 1000}))
            .unwrap();
        coll.insert_no_index(serde_json::json!({"_key": "fresh", "t": now + 3600}))
            .unwrap();
        let legacy = |ts: u64, key: &str| {
            format!("{}:ttl:{}:{}", LEGACY_TTL_EXPIRY_PREFIX, ts, key).into_bytes()
        };
        let old_entry = legacy(1001, "old");
        let fresh_entry = legacy(now + 3601, "fresh");
        {
            let cf = coll.db.cf_handle("s").unwrap();
            coll.db.put_cf(&cf, &old_entry, b"").unwrap();
            coll.db.put_cf(&cf, &fresh_entry, b"").unwrap();
        }

        // Legacy entries are not documents.
        assert_eq!(coll.recalculate_count(), 2);

        assert_eq!(coll.cleanup_all_expired_documents().unwrap(), 1);
        assert!(coll.get("old").is_err());
        assert!(coll.get("fresh").is_ok());

        let cf = coll.db.cf_handle("s").unwrap();
        assert!(coll.db.get_cf(&cf, &old_entry).unwrap().is_none());
        assert!(coll.db.get_cf(&cf, &fresh_entry).unwrap().is_none());
        let migrated = Collection::ttl_expiry_key("ttl", now + 3601, "fresh");
        assert!(coll.db.get_cf(&cf, &migrated).unwrap().is_some());
    }
}
