use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::serializer::{deserialize_doc, deserialize_doc_as_value, serialize_doc};
use rust_rocksdb::{
    AsColumnFamilyRef, BoundColumnFamily, Direction, IteratorMode, ReadOptions, WriteBatch,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// The error for an insert whose `_key` is already taken. Single-document and
/// batch inserts return the same conflict (audit D1: the single path used to
/// overwrite silently, leaving the old document's index entries behind).
fn key_exists_error(key: &str) -> DbError {
    DbError::ConflictError(format!("Document with _key '{}' already exists", key))
}

/// Remove and return `_key` from `data`, or generate a UUIDv7 key.
fn take_key(data: &mut Value) -> DbResult<String> {
    if let Some(obj) = data.as_object_mut() {
        if let Some(key_value) = obj.remove("_key") {
            return match key_value.as_str() {
                Some(key_str) => Ok(key_str.to_string()),
                None => Err(DbError::InvalidDocument(
                    "_key must be a string".to_string(),
                )),
            };
        }
    }
    Ok(uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string())
}

impl Collection {
    // ==================== Basic CRUD ====================

    fn live_cf(&self) -> DbResult<Arc<BoundColumnFamily<'_>>> {
        self.db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })
    }

    /// Get a document by key
    pub fn get(&self, key: &str) -> DbResult<Document> {
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = self.live_cf()?;

        let bytes = db
            .get_cf(&cf, Self::doc_key(key))
            .map_err(|e| DbError::InternalError(format!("Failed to get document: {}", e)))?
            .ok_or_else(|| DbError::DocumentNotFound(key.to_string()))?;

        let doc = deserialize_doc(&bytes)?;
        Ok(doc)
    }

    /// Get multiple documents by keys
    pub fn get_many(&self, keys: &[String]) -> Vec<Document> {
        keys.iter().filter_map(|k| self.get(k).ok()).collect()
    }

    // ---------- per-document derived entries (idx / geo / ft / ttl) ----------

    fn add_insert_entries<C: AsColumnFamilyRef>(
        &self,
        batch: &mut WriteBatch,
        cf: &C,
        key: &str,
        doc_value: &Value,
    ) -> DbResult<()> {
        let (regular, geo) = self.compute_index_entries_for_insert(key, doc_value)?;
        for (entry_key, entry_value) in regular.into_iter().chain(geo) {
            batch.put_cf(cf, entry_key, entry_value);
        }
        for (entry_key, entry_value) in self.compute_fulltext_entries_for_insert(key, doc_value) {
            batch.put_cf(cf, entry_key, entry_value);
        }
        for (entry_key, _) in self.compute_ttl_expiry_entries_for_insert(key, doc_value) {
            batch.put_cf(cf, entry_key, b"");
        }
        Ok(())
    }

    fn add_update_entries<C: AsColumnFamilyRef>(
        &self,
        batch: &mut WriteBatch,
        cf: &C,
        key: &str,
        old_value: &Value,
        new_value: &Value,
    ) -> DbResult<()> {
        // Removals strictly before additions: an unchanged entry appears in
        // both lists and must end up present.
        let (entries_to_add, keys_to_remove, geo_entries_to_add, geo_keys_to_remove) =
            self.compute_index_entries_for_update(key, old_value, new_value)?;
        for key_to_remove in keys_to_remove.into_iter().chain(geo_keys_to_remove) {
            batch.delete_cf(cf, key_to_remove);
        }
        for (entry_key, entry_value) in entries_to_add.into_iter().chain(geo_entries_to_add) {
            batch.put_cf(cf, entry_key, entry_value);
        }

        let (ft_to_add, ft_to_remove) =
            self.compute_fulltext_entries_for_update(key, old_value, new_value);
        for key_to_remove in ft_to_remove {
            batch.delete_cf(cf, key_to_remove);
        }
        for (entry_key, entry_value) in ft_to_add {
            batch.put_cf(cf, entry_key, entry_value);
        }

        let (ttl_to_add, ttl_to_remove) =
            self.compute_ttl_expiry_entries_for_update(key, old_value, new_value);
        for key_to_remove in ttl_to_remove {
            batch.delete_cf(cf, key_to_remove);
        }
        for (entry_key, _) in ttl_to_add {
            batch.put_cf(cf, entry_key, b"");
        }
        Ok(())
    }

    fn add_delete_entries<C: AsColumnFamilyRef>(
        &self,
        batch: &mut WriteBatch,
        cf: &C,
        key: &str,
        doc_value: &Value,
    ) -> DbResult<()> {
        let (regular_keys, geo_keys) = self.compute_index_entries_for_delete(key, doc_value)?;
        for key_to_remove in regular_keys.into_iter().chain(geo_keys) {
            batch.delete_cf(cf, key_to_remove);
        }
        for key_to_remove in self.compute_fulltext_entries_for_delete(key, doc_value) {
            batch.delete_cf(cf, key_to_remove);
        }
        for key_to_remove in self.compute_ttl_expiry_entries_for_delete(key, doc_value) {
            batch.delete_cf(cf, key_to_remove);
        }
        Ok(())
    }

    /// Run a single-document write under its key stripe, then give the
    /// vector indexes their throttled persist once the stripe is released
    /// (audit D9: only the batch paths used to persist, so single-document
    /// vector changes waited for shutdown). The persist can serialize a whole
    /// index; holding the stripe through it would stall that key's writers.
    fn with_key_locked<T>(&self, key: &str, f: impl FnOnce() -> DbResult<T>) -> DbResult<T> {
        let result = {
            let _key_guard = self.lock_keys([key]);
            f()
        };
        if result.is_ok() {
            self.persist_vector_indexes_throttled();
        }
        result
    }

    // ---------- insert ----------

    /// Insert a new document. Fails with a conflict if `_key` exists; use
    /// [`Collection::insert_or_replace`] where overwriting is intended.
    pub fn insert(&self, data: Value) -> DbResult<Document> {
        self.insert_internal(data, true)
    }

    /// Insert a new document without updating indexes (for bulk loads)
    pub fn insert_no_index(&self, data: Value) -> DbResult<Document> {
        self.insert_internal(data, false)
    }

    /// Internal insert implementation with atomic document + index writes
    pub(crate) fn insert_internal(
        &self,
        mut data: Value,
        update_indexes: bool,
    ) -> DbResult<Document> {
        // Validate edge documents
        if self.collection_type.read().as_str() == "edge" {
            self.validate_edge_document(&data)?;
        }

        // Validate against JSON schema if defined
        if let Some(validator) = self.get_cached_schema_validator()? {
            validator.validate(&data).map_err(|e| {
                DbError::InvalidDocument(format!("Schema validation failed: {}", e))
            })?;
        }

        let key = take_key(&mut data)?;
        let doc = Document::with_key(&self.name, key.clone(), data);

        self.with_key_locked(&key, || self.insert_locked(doc, update_indexes))
    }

    /// Insert `doc`; the caller holds its key stripe (audit D4).
    fn insert_locked(&self, doc: Document, update_indexes: bool) -> DbResult<Document> {
        let key = doc.key.clone();
        let doc_value = doc.to_value();

        let tokens = if update_indexes {
            self.unique_tokens(&doc_value)
        } else {
            Vec::new()
        };
        let _unique_guard = self.lock_unique_tokens(&tokens);

        let db = &self.db;
        let cf = self.live_cf()?;

        // Audit D1: an existing key is a conflict, never an overwrite.
        if db
            .get_pinned_cf(&cf, Self::doc_key(&key))
            .map_err(|e| DbError::InternalError(format!("Failed to check existing key: {}", e)))?
            .is_some()
        {
            return Err(key_exists_error(&key));
        }

        if !tokens.is_empty() {
            self.check_unique_constraints(&key, &doc_value)?;
        }

        let doc_bytes = serialize_doc(&doc)?;

        // Build WriteBatch with document and all index entries atomically
        let mut batch = WriteBatch::default();
        batch.put_cf(&cf, Self::doc_key(&key), &doc_bytes);

        // Record a version in the same atomic batch (if versioning is enabled).
        let versioned = self.is_versioned();
        if versioned {
            self.append_version_to_batch(&mut batch, &cf, &key, Some(&doc_value));
        }

        if update_indexes {
            self.add_insert_entries(&mut batch, &cf, &key, &doc_value)?;
        }

        // Atomic write: document + indexes together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to insert document: {}", e)))?;

        // Update vector indexes in-memory (separate from WriteBatch)
        if update_indexes {
            self.update_vector_indexes_on_upsert(&key, &doc_value);
        }

        // Enforce version retention (best-effort, off the atomic write).
        if versioned {
            self.prune_versions(&key);
        }

        self.increment_count();

        let _ = self.emit_change(ChangeEvent {
            type_: ChangeType::Insert,
            key,
            data: Some(doc_value),
            old_data: None,
        });

        Ok(doc)
    }

    /// Insert `data`, or replace the stored document wholesale if its `_key`
    /// already exists, with full index maintenance.
    ///
    /// For callers that relied on `insert` overwriting (audit D1). Unlike
    /// `update` / `upsert_batch`, which merge, fields absent from `data` are
    /// removed; `_created_at` is kept, `_rev` and `_updated_at` renewed.
    pub fn insert_or_replace(&self, mut data: Value) -> DbResult<Document> {
        if self.collection_type.read().as_str() == "edge" {
            self.validate_edge_document(&data)?;
        }
        if let Some(validator) = self.get_cached_schema_validator()? {
            validator.validate(&data).map_err(|e| {
                DbError::InvalidDocument(format!("Schema validation failed: {}", e))
            })?;
        }

        let key = take_key(&mut data)?;
        let mut doc = Document::with_key(&self.name, key.clone(), data);

        self.with_key_locked(&key, || {
            let old_doc = match self.get(&key) {
                Ok(old) => old,
                Err(DbError::DocumentNotFound(_)) => return self.insert_locked(doc, true),
                Err(e) => return Err(e),
            };
            if self.collection_type.read().as_str() == "timeseries" {
                return Err(DbError::OperationNotSupported(
                    "Update operations are not allowed on timeseries collections".to_string(),
                ));
            }
            doc.created_at = old_doc.created_at;
            let old_value = old_doc.to_value();
            let new_value = doc.to_value();
            self.write_update_locked(&key, old_value, doc, new_value)
        })
    }

    // ---------- update ----------

    /// Update a document with atomic document + index writes
    pub fn update(&self, key: &str, data: Value) -> DbResult<Document> {
        if self.collection_type.read().as_str() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Update operations are not allowed on timeseries collections".to_string(),
            ));
        }

        // Audit D4: read, diff and write under the key's stripe, so two
        // concurrent updates cannot both remove the same old index entry
        // and leave one of their new ones orphaned.
        self.with_key_locked(key, || {
            // Get old document for index updates
            let old_doc = self.get(key)?;
            let old_value = old_doc.to_value();

            // Create updated document
            let mut doc = old_doc;
            doc.update(data);
            let new_value = doc.to_value();

            // Validate edge documents after update
            if self.collection_type.read().as_str() == "edge" {
                self.validate_edge_document(&new_value)?;
            }

            // Validate against JSON schema if defined
            if let Some(validator) = self.get_cached_schema_validator()? {
                validator.validate(&new_value).map_err(|e| {
                    DbError::InvalidDocument(format!("Schema validation failed: {}", e))
                })?;
            }

            self.write_update_locked(key, old_value, doc, new_value)
        })
    }

    /// Update a document with revision check (optimistic concurrency control)
    pub fn update_with_rev(
        &self,
        key: &str,
        expected_rev: &str,
        data: Value,
    ) -> DbResult<Document> {
        if self.collection_type.read().as_str() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Update operations are not allowed on timeseries collections".to_string(),
            ));
        }

        // Held across the revision check too, so check-and-write is atomic.
        self.with_key_locked(key, || {
            // Get old document for index updates
            let old_doc = self.get(key)?;

            // Check revision matches
            if old_doc.revision() != expected_rev {
                return Err(DbError::ConflictError(format!(
                    "Document '{}' has been modified. Expected revision '{}', but current is '{}'",
                    key,
                    expected_rev,
                    old_doc.revision()
                )));
            }

            let old_value = old_doc.to_value();

            // Create updated document
            let mut doc = old_doc;
            doc.update(data);
            let new_value = doc.to_value();

            self.write_update_locked(key, old_value, doc, new_value)
        })
    }

    /// Write an updated document and its index diff; the caller holds the
    /// key stripe and has read `old_value` under it.
    fn write_update_locked(
        &self,
        key: &str,
        old_value: Value,
        doc: Document,
        new_value: Value,
    ) -> DbResult<Document> {
        // A unique value the update claims is checked and written under that
        // value's stripe (audit D4; updates used not to check at all).
        let tokens = self.unique_tokens(&new_value);
        let _unique_guard = self.lock_unique_tokens(&tokens);
        if !tokens.is_empty() {
            self.check_unique_constraints(key, &new_value)?;
        }

        let doc_bytes = serialize_doc(&doc)?;

        let db = &self.db;
        let cf = self.live_cf()?;
        let mut batch = WriteBatch::default();

        // Update document in batch
        batch.put_cf(&cf, Self::doc_key(key), &doc_bytes);

        // Record a version in the same atomic batch (if versioning is enabled).
        let versioned = self.is_versioned();
        if versioned {
            self.append_version_to_batch(&mut batch, &cf, key, Some(&new_value));
        }

        self.add_update_entries(&mut batch, &cf, key, &old_value, &new_value)?;

        // Atomic write: document + all index updates together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to update document: {}", e)))?;

        // Update vector indexes in-memory (separate from WriteBatch). Skip the
        // delete+reinsert entirely when the embedding is unchanged — a document
        // rewritten for non-embedding fields must not pay HNSW churn or dirty
        // the index (which would trigger a full re-serialize on persist).
        if !self.vector_index_unchanged(&old_value, &new_value) {
            self.update_vector_indexes_on_delete(key);
            self.update_vector_indexes_on_upsert(key, &new_value);
        }

        // Enforce version retention (best-effort, off the atomic write).
        if versioned {
            self.prune_versions(key);
        }

        // Broadcast change event
        let _ = self.emit_change(ChangeEvent {
            type_: ChangeType::Update,
            key: key.to_string(),
            data: Some(new_value),
            old_data: Some(old_value),
        });

        Ok(doc)
    }

    // ---------- delete ----------

    /// Delete a document with atomic document + index removal
    pub fn delete(&self, key: &str) -> DbResult<()> {
        self.with_key_locked(key, || {
            let doc_value = self.get(key)?.to_value();
            self.delete_docs_locked(vec![(key.to_string(), doc_value)], Vec::new())?;
            Ok(())
        })
    }

    /// Delete documents already read under their key stripes, together with
    /// everything derived from them: index / geo / fulltext / TTL entries,
    /// a version tombstone, blob chunks, the vector-index entries, the
    /// document count and a change event. Every delete path goes through
    /// here — including the TTL reaper, which used to drop the bare `doc:`
    /// key and nothing else (audit H8). `extra_deletes` ride in the same
    /// batch (the reaper's consumed expiry entries).
    pub(crate) fn delete_docs_locked(
        &self,
        docs: Vec<(String, Value)>,
        extra_deletes: Vec<Vec<u8>>,
    ) -> DbResult<usize> {
        if docs.is_empty() && extra_deletes.is_empty() {
            return Ok(0);
        }

        let db = &self.db;
        let cf = self.live_cf()?;
        let versioned = self.is_versioned();
        let mut batch = WriteBatch::default();

        for (key, doc_value) in &docs {
            batch.delete_cf(&cf, Self::doc_key(key));
            if versioned {
                self.append_version_to_batch(&mut batch, &cf, key, None);
            }
            self.add_delete_entries(&mut batch, &cf, key, doc_value)?;
        }
        for extra in extra_deletes {
            batch.delete_cf(&cf, extra);
        }

        // Atomic write: document deletions + index removals together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to delete document: {}", e)))?;

        let deleted = docs.len();
        if deleted == 0 {
            return Ok(0);
        }

        // Blob chunks, vector entries and version pruning are separate from
        // the WriteBatch, and only happen once the documents are really gone.
        let is_blob = self.collection_type.read().as_str() == "blob";
        for (key, _) in &docs {
            if is_blob {
                if let Err(e) = self.delete_blob_data(key) {
                    tracing::warn!("Failed to delete blob chunks for {}: {}", key, e);
                }
            }
            self.update_vector_indexes_on_delete(key);
            if versioned {
                self.prune_versions(key);
            }
        }

        // Update count. Saturating: several Collection instances can exist
        // for the same CF (engine cache, Database cache, fresh handles), each
        // with its own counter — a plain fetch_sub on an instance that didn't
        // see the inserts wraps to u64::MAX and the UI shows
        // 18446744073709551615 documents.
        let _ = self
            .doc_count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(deleted))
            });
        self.count_dirty.store(true, Ordering::Relaxed);

        for (key, old_data) in docs {
            let _ = self.emit_change(ChangeEvent {
                type_: ChangeType::Delete,
                key,
                data: None,
                old_data: Some(old_data),
            });
        }

        Ok(deleted)
    }

    // ==================== Batch Operations ====================

    /// Batch upsert (insert or update) multiple documents - optimized for replication
    ///
    /// Existing documents are merged (`Document::update`), new ones created.
    /// Returns the number of documents written.
    pub fn upsert_batch(&self, documents: Vec<(String, Value)>) -> DbResult<usize> {
        if documents.is_empty() {
            return Ok(0);
        }

        if self.collection_type.read().as_str() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Upsert (update) operations are not allowed on timeseries collections. Use insert_batch instead.".to_string(),
            ));
        }

        let _key_guard = self.lock_keys(documents.iter().map(|(k, _)| k.as_str()));

        let db = &self.db;
        let cf = self.live_cf()?;

        // (key, value on disk before this batch, merged document, existed)
        let mut pending: Vec<(String, Option<Value>, Document, bool)> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();

        for (key, mut data) in documents {
            // Ensure _key is set
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_key".to_string(), Value::String(key.clone()));
            }

            // The same key twice in one batch builds on the pending version,
            // so the index diff below is still taken against what is on disk.
            if let Some(&i) = positions.get(&key) {
                pending[i].2.update(data);
                continue;
            }

            // The bool is "the document already existed", which decides the
            // count and whether the change event is an Update or an Insert.
            let stored = db.get_cf(&cf, Self::doc_key(&key)).ok().flatten();
            let existed = stored.is_some();
            let (old_value, doc) = match stored.and_then(|bytes| deserialize_doc(&bytes).ok()) {
                Some(mut existing) => {
                    let old_value = existing.to_value();
                    existing.update(data);
                    (Some(old_value), existing)
                }
                None => (None, Document::with_key(&self.name, key.clone(), data)),
            };
            positions.insert(key.clone(), pending.len());
            pending.push((key, old_value, doc, existed));
        }

        let versioned = self.is_versioned();
        let mut batch = WriteBatch::default();
        let mut insert_count = 0;
        let mut written: Vec<(String, Value, Option<Value>, bool)> =
            Vec::with_capacity(pending.len());

        for (key, old_value, doc, existed) in pending {
            let Ok(doc_bytes) = serialize_doc(&doc) else {
                continue;
            };
            let new_value = doc.to_value();
            batch.put_cf(&cf, Self::doc_key(&key), &doc_bytes);
            if versioned {
                self.append_version_to_batch(&mut batch, &cf, &key, Some(&new_value));
            }
            // Index maintenance: this path used to write the document alone,
            // so every replicated or bulk-upserted write left the regular,
            // geo, fulltext and TTL entries stale.
            match &old_value {
                Some(old) => self.add_update_entries(&mut batch, &cf, &key, old, &new_value)?,
                None => self.add_insert_entries(&mut batch, &cf, &key, &new_value)?,
            }
            if !existed {
                insert_count += 1;
            }
            written.push((key, new_value, old_value, existed));
        }

        let count = written.len();

        // Write all documents in one batch operation
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch upsert: {}", e)))?;

        // Update document count (only for new inserts)
        if insert_count > 0 {
            self.doc_count.fetch_add(insert_count, Ordering::Relaxed);
            self.count_dirty.store(true, Ordering::Relaxed);
        }

        // Update vector indexes for all upserted documents
        for (key, new_value, old_value, _) in &written {
            match old_value {
                Some(old) if self.vector_index_unchanged(old, new_value) => {}
                Some(_) => {
                    self.update_vector_indexes_on_delete(key);
                    self.update_vector_indexes_on_upsert(key, new_value);
                }
                None => self.update_vector_indexes_on_upsert(key, new_value),
            }
            if versioned {
                self.prune_versions(key);
            }
        }
        // Persist vector indexes after batch, outside the key stripes.
        drop(_key_guard);
        self.persist_vector_indexes_throttled();

        // Broadcast change events. `insert`, `insert_batch` and `delete` all do
        // this; this path did not, and it is the one replication applies through
        // (`sync::worker`) and the one shard replicas receive on
        // (`insert_documents_replica`) — so a changefeed subscriber saw deletes
        // propagate but never the inserts that preceded them.
        for (key, new_value, old_value, existed) in written {
            let _ = self.emit_change(ChangeEvent {
                type_: if existed {
                    ChangeType::Update
                } else {
                    ChangeType::Insert
                },
                key,
                data: Some(new_value),
                old_data: old_value,
            });
        }

        Ok(count)
    }

    /// Batch delete documents with atomic document + index removal
    pub fn delete_batch(&self, keys: Vec<String>) -> DbResult<usize> {
        if keys.is_empty() {
            return Ok(0);
        }

        let _key_guard = self.lock_keys(keys.iter().map(String::as_str));

        let db = &self.db;
        let cf = self.live_cf()?;

        let mut seen: HashSet<String> = HashSet::with_capacity(keys.len());
        let mut docs = Vec::new();
        for key in keys {
            // A repeated key would be counted and announced twice.
            if !seen.insert(key.clone()) {
                continue;
            }
            // Get document first (needed for index cleanup and change events)
            if let Ok(Some(bytes)) = db.get_cf(&cf, Self::doc_key(&key)) {
                if let Ok(doc) = deserialize_doc(&bytes) {
                    docs.push((key, doc.to_value()));
                }
            }
        }

        if docs.is_empty() {
            return Ok(0);
        }

        let deleted = self.delete_docs_locked(docs, Vec::new())?;
        drop(_key_guard);
        self.persist_vector_indexes_throttled();
        Ok(deleted)
    }

    /// Batch update multiple documents with atomic document + index writes
    pub fn update_batch(&self, updates: &[(String, Value)]) -> DbResult<Vec<Document>> {
        if updates.is_empty() {
            return Ok(Vec::new());
        }

        // Check timeseries restriction
        if self.collection_type.read().as_str() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Update operations are not allowed on timeseries collections".to_string(),
            ));
        }

        let _key_guard = self.lock_keys(updates.iter().map(|(k, _)| k.as_str()));

        let db = &self.db;
        let cf = self.live_cf()?;
        let is_edge = self.collection_type.read().as_str() == "edge";
        let schema_validator = self.get_cached_schema_validator()?;

        // Pass 1: merge every update. A key updated twice in the batch builds
        // on its pending version; its index diff is still against the disk.
        // (key, value on disk, updated document)
        let mut pending: Vec<(String, Value, Document)> = Vec::new();
        let mut positions: HashMap<String, usize> = HashMap::new();

        for (key, changes) in updates {
            let existing = positions.get(key).copied();
            let mut doc = match existing {
                Some(i) => pending[i].2.clone(),
                None => match self.get(key) {
                    Ok(old_doc) => old_doc,
                    Err(_) => continue,
                },
            };
            let old_value = if existing.is_none() {
                Some(doc.to_value())
            } else {
                None
            };

            doc.update(changes.clone());
            let new_value = doc.to_value();

            // Validate edge documents after update
            if is_edge {
                if let Err(e) = self.validate_edge_document(&new_value) {
                    tracing::warn!("Failed to validate edge for {}: {}", key, e);
                    continue;
                }
            }

            // Validate against JSON schema if defined
            if let Some(ref validator) = schema_validator {
                if let Err(e) = validator.validate(&new_value) {
                    tracing::warn!("Schema validation failed for {}: {}", key, e);
                    continue;
                }
            }

            match (existing, old_value) {
                (Some(i), _) => pending[i].2 = doc,
                (None, Some(old_value)) => {
                    positions.insert(key.clone(), pending.len());
                    pending.push((key.clone(), old_value, doc));
                }
                (None, None) => unreachable!("old_value is set whenever existing is None"),
            }
        }

        if pending.is_empty() {
            return Ok(Vec::new());
        }

        // Pass 2: claim unique values under their stripes (audit D4).
        let new_values: Vec<Value> = pending.iter().map(|(_, _, doc)| doc.to_value()).collect();
        let tokens: Vec<String> = new_values
            .iter()
            .flat_map(|v| self.unique_tokens(v))
            .collect();
        let _unique_guard = self.lock_unique_tokens(&tokens);
        if !tokens.is_empty() {
            let mut claimed: HashMap<String, &str> = HashMap::new();
            for ((key, _, _), new_value) in pending.iter().zip(&new_values) {
                self.check_unique_constraints(key, new_value)?;
                for token in self.unique_tokens(new_value) {
                    if let Some(other) = claimed.insert(token, key.as_str()) {
                        return Err(DbError::InvalidDocument(format!(
                            "Unique constraint violated: documents '{}' and '{}' in the same batch share a unique value",
                            other, key
                        )));
                    }
                }
            }
        }

        let versioned = self.is_versioned();
        let mut batch = WriteBatch::default();
        let mut updated_docs = Vec::with_capacity(pending.len());
        let mut change_events = Vec::with_capacity(pending.len());

        for ((key, old_value, doc), new_value) in pending.into_iter().zip(new_values) {
            let doc_bytes = match serialize_doc(&doc) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!("Failed to serialize {}: {}", key, e);
                    continue;
                }
            };
            batch.put_cf(&cf, Self::doc_key(&key), &doc_bytes);
            if versioned {
                self.append_version_to_batch(&mut batch, &cf, &key, Some(&new_value));
            }
            self.add_update_entries(&mut batch, &cf, &key, &old_value, &new_value)?;

            change_events.push((key, old_value, new_value));
            updated_docs.push(doc);
        }

        if updated_docs.is_empty() {
            return Ok(Vec::new());
        }

        // Commit batch atomically: all document updates + index updates together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch update: {}", e)))?;

        // Update vector indexes in-memory (separate from WriteBatch), after
        // the commit. Skip the delete+reinsert when the embedding is
        // unchanged so a bulk update that only rewrites metadata (e.g. an
        // incremental graph sync) pays no HNSW churn and doesn't dirty the
        // index into a full re-serialize.
        for (key, old_value, new_value) in &change_events {
            if !self.vector_index_unchanged(old_value, new_value) {
                self.update_vector_indexes_on_delete(key);
                self.update_vector_indexes_on_upsert(key, new_value);
            }
            if versioned {
                self.prune_versions(key);
            }
        }
        // Persist vector indexes after batch update, outside the stripes.
        drop(_unique_guard);
        drop(_key_guard);
        self.persist_vector_indexes_throttled();

        // Send Change Events
        for (key, old_data, new_data) in change_events {
            let _ = self.emit_change(ChangeEvent {
                type_: ChangeType::Update,
                key,
                data: Some(new_data),
                old_data: Some(old_data),
            });
        }

        Ok(updated_docs)
    }

    /// Insert multiple documents with atomic batched write
    pub fn insert_batch(&self, documents: Vec<Value>) -> DbResult<Vec<Document>> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }

        let is_edge = self.collection_type.read().as_str() == "edge";
        let schema_validator = self.get_cached_schema_validator()?;

        // Pass 1: validate and assign keys (all keys are needed up front to
        // take their stripes).
        let mut inserted_docs: Vec<Document> = Vec::with_capacity(documents.len());
        let mut batch_keys: HashSet<String> = HashSet::with_capacity(documents.len());
        for mut data in documents {
            // Validate edge documents
            if is_edge {
                self.validate_edge_document(&data)?;
            }

            // Validate against JSON schema if defined
            if let Some(ref validator) = schema_validator {
                validator.validate(&data).map_err(|e| {
                    DbError::InvalidDocument(format!("Schema validation failed: {}", e))
                })?;
            }

            let key = take_key(&mut data)?;

            // Check for duplicate _key within the batch
            if !batch_keys.insert(key.clone()) {
                return Err(DbError::InvalidDocument(format!(
                    "Duplicate _key '{}' within batch",
                    key
                )));
            }

            inserted_docs.push(Document::with_key(&self.name, key, data));
        }

        let _key_guard = self.lock_keys(inserted_docs.iter().map(|d| d.key.as_str()));
        let doc_values: Vec<Value> = inserted_docs.iter().map(|d| d.to_value()).collect();
        let tokens: Vec<String> = doc_values
            .iter()
            .flat_map(|v| self.unique_tokens(v))
            .collect();
        let _unique_guard = self.lock_unique_tokens(&tokens);

        let db = &self.db;
        let cf = self.live_cf()?;
        let versioned = self.is_versioned();
        let mut batch = WriteBatch::default();
        let mut claimed: HashSet<String> = HashSet::new();

        for (doc, doc_value) in inserted_docs.iter().zip(&doc_values) {
            let key = &doc.key;

            // Check if document with this key already exists in the DB
            if db
                .get_pinned_cf(&cf, Self::doc_key(key))
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to check existing key: {}", e))
                })?
                .is_some()
            {
                return Err(key_exists_error(key));
            }

            // Check unique constraints, against the DB and within the batch
            if !tokens.is_empty() {
                self.check_unique_constraints(key, doc_value)?;
                for token in self.unique_tokens(doc_value) {
                    if !claimed.insert(token) {
                        return Err(DbError::InvalidDocument(format!(
                            "Unique constraint violated: document '{}' repeats a unique value used earlier in the same batch",
                            key
                        )));
                    }
                }
            }

            let doc_bytes = serialize_doc(doc)?;

            // Add document to batch
            batch.put_cf(&cf, Self::doc_key(key), &doc_bytes);
            if versioned {
                self.append_version_to_batch(&mut batch, &cf, key, Some(doc_value));
            }

            self.add_insert_entries(&mut batch, &cf, key, doc_value)?;
        }

        // Atomic write: all documents + indexes together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch insert: {}", e)))?;

        // Update vector indexes in-memory (separate from WriteBatch)
        for (doc, doc_value) in inserted_docs.iter().zip(&doc_values) {
            self.update_vector_indexes_on_upsert(&doc.key, doc_value);
        }
        // Persist vector indexes after batch, outside the stripes.
        drop(_unique_guard);
        drop(_key_guard);
        self.persist_vector_indexes_throttled();

        // Update document count
        let count = inserted_docs.len();
        self.doc_count.fetch_add(count, Ordering::Relaxed);
        self.count_dirty.store(true, Ordering::Relaxed);

        // Broadcast change events
        for (doc, doc_value) in inserted_docs.iter().zip(doc_values) {
            let _ = self.emit_change(ChangeEvent {
                type_: ChangeType::Insert,
                key: doc.key.clone(),
                data: Some(doc_value),
                old_data: None,
            });
        }

        Ok(inserted_docs)
    }

    // ==================== Scanning ====================

    /// Get all documents
    pub fn all(&self) -> Vec<Document> {
        self.scan(None)
    }

    /// Scan documents with an optional limit
    pub fn scan(&self, limit: Option<usize>) -> Vec<Document> {
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = match db.cf_handle(&self.name) {
            Some(cf) => cf,
            // CF dropped mid-operation (concurrent database delete): an
            // empty scan is the graceful answer.
            None => return Vec::new(),
        };
        let prefix = DOC_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(&cf, prefix);

        let iter = iter.filter_map(|result| {
            result.ok().and_then(|(key, value)| {
                if key.starts_with(prefix) {
                    deserialize_doc(&value).ok()
                } else {
                    None
                }
            })
        });

        if let Some(n) = limit {
            iter.take(n).collect()
        } else {
            iter.collect()
        }
    }

    /// Scan documents and return directly as serde_json::Value, skipping intermediate Document.
    /// Faster for queries that just need the JSON value (e.g., FOR doc IN coll RETURN doc).
    pub fn scan_values(&self, limit: Option<usize>) -> Vec<Value> {
        self.scan_values_range(0, limit)
    }

    /// Scan a range of documents as serde_json::Value with offset + optional limit.
    /// Skips `offset` documents and then returns up to `limit` documents.
    pub fn scan_values_range(&self, offset: usize, limit: Option<usize>) -> Vec<Value> {
        if matches!(limit, Some(0)) {
            return Vec::new();
        }

        let db = &self.db;
        let cf = match db.cf_handle(&self.name) {
            Some(cf) => cf,
            // CF dropped mid-operation (concurrent database delete): an
            // empty scan is the graceful answer.
            None => return Vec::new(),
        };
        let prefix = DOC_PREFIX.as_bytes();

        // Scale readahead to limit: small reads get minimal readahead
        let requested = limit.map(|n| n.saturating_add(offset));
        let mut read_opts = ReadOptions::default();
        read_opts.set_prefix_same_as_start(true);
        let readahead = match requested {
            Some(n) if n <= 10 => 0,          // Small reads: no readahead
            Some(n) if n <= 100 => 16 * 1024, // Medium: 16KB
            _ => 256 * 1024,                  // Large/unlimited: 256KB
        };
        if readahead > 0 {
            read_opts.set_readahead_size(readahead);
        }

        let iter = db.iterator_cf_opt(
            &cf,
            read_opts,
            IteratorMode::From(prefix, Direction::Forward),
        );

        let capacity = limit.unwrap_or(128);
        let mut results = Vec::with_capacity(capacity);
        let mut skipped = 0usize;

        for (key, value) in iter.flatten() {
            if !key.starts_with(prefix) {
                break;
            }
            if let Ok(val) = deserialize_doc_as_value(&value) {
                if skipped < offset {
                    skipped += 1;
                    continue;
                }
                results.push(val);
                if let Some(n) = limit {
                    if results.len() >= n {
                        break;
                    }
                }
            }
        }

        results
    }

    // ==================== Counters ====================

    /// Recalculate document count from storage
    pub fn recalculate_count(&self) -> usize {
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        if let Some(cf) = db.cf_handle(&self.name) {
            let count = Self::count_doc_entries(db, &cf);

            self.doc_count
                .store(count, std::sync::atomic::Ordering::Relaxed);
            self.count_dirty
                .store(true, std::sync::atomic::Ordering::Relaxed);

            count
        } else {
            0
        }
    }

    /// Count documents in the collection
    pub fn count(&self) -> usize {
        self.doc_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Count blob chunks in the collection (0 for non-blob collections).
    ///
    /// Like [`Self::count`] this is a cached atomic, not a scan — cheap enough
    /// for background sweeps to poll on every pass.
    pub fn chunk_count(&self) -> usize {
        self.ensure_chunk_count();
        self.chunk_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Recount documents from actual RocksDB data (slow but accurate)
    pub fn recount_documents(&self) -> usize {
        if let Some(cf) = self.db.cf_handle(&self.name) {
            let actual_count = Self::count_doc_entries(&self.db, &cf);

            // Update the cached count to match reality
            self.doc_count.store(actual_count, Ordering::Relaxed);
            self.count_dirty.store(true, Ordering::Relaxed);

            actual_count
        } else {
            0
        }
    }

    /// Increment document count (called on insert) - atomic, no disk I/O
    pub(crate) fn increment_count(&self) {
        self.doc_count.fetch_add(1, Ordering::Relaxed);
        self.count_dirty.store(true, Ordering::Relaxed);
    }

    /// Decrement document count (called on delete) - atomic, no disk I/O.
    /// Saturating: a counter that didn't observe the matching insert (another
    /// Collection instance did) must floor at 0, not wrap to u64::MAX.
    pub(crate) fn decrement_count(&self) {
        let _ = self
            .doc_count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(1))
            });
        self.count_dirty.store(true, Ordering::Relaxed);
    }

    // ==================== Maintenance ====================

    /// Truncate collection (delete all documents)
    /// Remove every document plus all of its per-document index / fulltext /
    /// TTL / blob entries, while preserving the collection's definitions
    /// (schema, index metadata, shard config, type).
    ///
    /// This is O(number of key prefixes), not O(number of documents). The old
    /// path read every document into memory (`all()`), then in `delete_batch`
    /// re-read each one, recomputed its index keys, queued a per-doc point
    /// delete, and emitted a change event per row — multi-second on large
    /// collections. Instead we write a handful of RocksDB range tombstones in a
    /// single atomic batch, which is effectively constant-time regardless of
    /// document count.
    pub fn truncate(&self) -> DbResult<usize> {
        // Cached count is the maintained document count; report it as "deleted"
        // without paying for a full scan.
        let count = self.count();

        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;

        // Range tombstones over every per-document DATA prefix. Each prefix ends
        // in ':' (0x3A), so the exclusive upper bound is the same prefix with
        // ':' bumped to ';' (0x3B). That bound stops before the matching
        // `*_meta:` definitions and the `_stats:*` config keys (whose next byte
        // '_' is 0x5F > 0x3B), so index definitions, schema, collection type and
        // shard config all survive the truncate. `doc:` additionally covers the
        // legacy nested `doc:ttl_exp:` expiry-index entries; `ttl_exp;` sorts
        // before `ttl_meta:`, so TTL index definitions survive too.
        let data_ranges: [(&[u8], &[u8]); 10] = [
            (b"doc:", b"doc;"),         // documents + legacy TTL expiry entries
            (b"ttl_exp:", b"ttl_exp;"), // TTL expiry entries
            (b"idx:", b"idx;"),         // persistent / hash index entries
            (b"geo:", b"geo;"),         // geo index entries
            (b"ft:", b"ft;"),           // fulltext n-gram entries
            (b"ft_term:", b"ft_term;"), // fulltext term -> doc entries
            (b"blo:", b"blo;"),         // blob chunks
            (b"blo_tmp:", b"blo_tmp;"), // resumable-upload temp chunks
            (b"blo_idx:", b"blo_idx;"), // blob bloom-filter index
            (b"cfo_idx:", b"cfo_idx;"), // cuckoo-filter index
        ];

        let mut batch = WriteBatch::default();
        for (start, end) in data_ranges {
            batch.delete_range_cf(&cf, start, end);
        }
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to truncate: {}", e)))?;

        // Vector indexes answer searches from their in-memory structure, not via
        // the `idx:` entries we just dropped — leaving stale vectors would keep
        // surfacing truncated documents. Empty each defined index in place
        // (loading it first if needed) and persist the empty state to
        // `vec_data:`.
        for config in self.get_all_vector_index_configs() {
            if let Ok(index) = self.get_vector_index(&config.name) {
                index.clear();
            }
        }
        if let Err(e) = self.persist_vector_indexes() {
            tracing::warn!("Failed to persist vector indexes after truncate: {}", e);
        }

        // Reset counters and flush the zeroed count to disk immediately.
        self.doc_count.store(0, Ordering::Relaxed);
        // Every `blo:` key was just range-deleted, so the count is known
        // without a walk — publish it and mark it resolved.
        self.chunk_count.store(0, Ordering::Relaxed);
        self.chunk_count_ready.store(true, Ordering::Release);
        self.count_dirty.store(true, Ordering::Relaxed);
        self.flush_stats();

        // One broadcast event instead of one-per-document: subscribers
        // (LiveQuery, stream processors) learn the collection was cleared
        // without being flooded.
        if count > 0 {
            let _ = self.emit_change(ChangeEvent {
                type_: ChangeType::Truncate,
                key: String::new(),
                data: None,
                old_data: None,
            });
        }

        Ok(count)
    }

    /// Prune documents older than timestamp (for timeseries)
    /// The timestamp is in milliseconds since Unix epoch.
    /// This extracts the timestamp from UUIDv7 keys and deletes matching documents.
    pub fn prune_older_than(&self, timestamp_ms: u64) -> DbResult<usize> {
        // Collect keys to delete
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;
        let prefix = DOC_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(&cf, prefix);

        let mut keys_to_delete = Vec::new();

        for result in iter.flatten() {
            let (key_bytes, _value) = result;
            if !key_bytes.starts_with(prefix) {
                break;
            }

            // Extract the document key (without prefix)
            let doc_key = String::from_utf8_lossy(&key_bytes[prefix.len()..]).to_string();

            // Try to parse as UUID and extract timestamp
            if let Ok(uuid) = uuid::Uuid::parse_str(&doc_key) {
                // UUIDv7: timestamp is in the upper 48 bits (milliseconds)
                // uuid.as_u128() returns: timestamp_ms (48 bits) | version (4 bits) | rand_a (12 bits) | variant (2 bits) | rand_b (62 bits)
                let uuid_int = uuid.as_u128();
                let uuid_timestamp_ms = (uuid_int >> 80) as u64;

                if uuid_timestamp_ms < timestamp_ms {
                    keys_to_delete.push(doc_key);
                }
            }
        }

        let _ = db; // Keep reference alive until this point

        if keys_to_delete.is_empty() {
            return Ok(0);
        }

        self.delete_batch(keys_to_delete)
    }

    // ==================== Validation ====================

    /// Validate edge document has required _from and _to fields
    pub(crate) fn validate_edge_document(&self, data: &Value) -> DbResult<()> {
        let obj = data.as_object().ok_or_else(|| {
            DbError::InvalidDocument("Edge document must be a JSON object".to_string())
        })?;

        // Check _from field
        match obj.get("_from") {
            Some(Value::String(s)) if !s.is_empty() => {}
            Some(Value::String(_)) => {
                return Err(DbError::InvalidDocument(
                    "Edge document _from field must be a non-empty string".to_string(),
                ));
            }
            Some(_) => {
                return Err(DbError::InvalidDocument(
                    "Edge document _from field must be a string".to_string(),
                ));
            }
            None => {
                return Err(DbError::InvalidDocument(
                    "Edge document must have a _from field".to_string(),
                ));
            }
        }

        // Check _to field
        match obj.get("_to") {
            Some(Value::String(s)) if !s.is_empty() => {}
            Some(Value::String(_)) => {
                return Err(DbError::InvalidDocument(
                    "Edge document _to field must be a non-empty string".to_string(),
                ));
            }
            Some(_) => {
                return Err(DbError::InvalidDocument(
                    "Edge document _to field must be a string".to_string(),
                ));
            }
            None => {
                return Err(DbError::InvalidDocument(
                    "Edge document must have a _to field".to_string(),
                ));
            }
        }

        Ok(())
    }
}
