use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::document_cache::get_document_cache;
use crate::storage::serializer::{deserialize_doc, deserialize_doc_as_value, serialize_doc};
use rust_rocksdb::{Direction, IteratorMode, ReadOptions, WriteBatch};
use serde_json::Value;
use std::sync::atomic::Ordering;

impl Collection {
    // ==================== Basic CRUD ====================

    /// Get a document by key
    pub fn get(&self, key: &str) -> DbResult<Document> {
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;

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

    /// Insert a new document
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

        // Extract or generate key
        let key = if let Some(obj) = data.as_object_mut() {
            if let Some(key_value) = obj.remove("_key") {
                if let Some(key_str) = key_value.as_str() {
                    key_str.to_string()
                } else {
                    return Err(DbError::InvalidDocument(
                        "_key must be a string".to_string(),
                    ));
                }
            } else {
                uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
            }
        } else {
            uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
        };

        let doc = Document::with_key(&self.name, key.clone(), data);
        let doc_value = doc.to_value();

        if update_indexes {
            self.check_unique_constraints(&key, &doc_value)?;
        }

        let doc_bytes = serialize_doc(&doc)?;

        // Build WriteBatch with document and all index entries atomically
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;
        let mut batch = WriteBatch::default();

        // Add document to batch
        batch.put_cf(&cf, Self::doc_key(&key), &doc_bytes);

        // Record a version in the same atomic batch (if versioning is enabled).
        let versioned = self.is_versioned();
        if versioned {
            self.append_version_to_batch(&mut batch, &cf, &key, Some(&doc_value));
        }

        // Add index entries to batch (if enabled)
        if update_indexes {
            // Compute and add regular + geo index entries
            let (regular_entries, geo_entries) =
                self.compute_index_entries_for_insert(&key, &doc_value)?;
            for (entry_key, entry_value) in regular_entries {
                batch.put_cf(&cf, entry_key, entry_value);
            }
            for (entry_key, entry_value) in geo_entries {
                batch.put_cf(&cf, entry_key, entry_value);
            }

            // Compute and add fulltext entries
            let fulltext_entries = self.compute_fulltext_entries_for_insert(&key, &doc_value);
            for (entry_key, entry_value) in fulltext_entries {
                batch.put_cf(&cf, entry_key, entry_value);
            }

            // Compute and add TTL expiry entries
            let ttl_expiry_entries = self.compute_ttl_expiry_entries_for_insert(&key, &doc_value);
            for (entry_key, _entry_value) in ttl_expiry_entries {
                batch.put_cf(&cf, entry_key, Vec::new());
            }
        }

        // Atomic write: document + indexes together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to insert document: {}", e)))?;

        // Invalidate document cache for this key
        let cache_key = format!("{}:{}", self.name, key);
        let cache = get_document_cache();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                cache.invalidate(&cache_key).await;
            });
        }

        // Update vector indexes in-memory (separate from WriteBatch)
        if update_indexes {
            self.update_vector_indexes_on_upsert(&key, &doc_value);
        }

        // Enforce version retention (best-effort, off the atomic write).
        if versioned {
            self.prune_versions(&key);
        }

        // Update document count
        self.increment_count();

        // Broadcast change event
        let _ = self.change_sender.send(ChangeEvent {
            type_: ChangeType::Insert,
            key: key.clone(),
            data: Some(doc_value),
            old_data: None,
        });

        Ok(doc)
    }

    /// Update a document with atomic document + index writes
    pub fn update(&self, key: &str, data: Value) -> DbResult<Document> {
        if self.collection_type.read().as_str() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Update operations are not allowed on timeseries collections".to_string(),
            ));
        }
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

        let doc_bytes = serialize_doc(&doc)?;

        // Build WriteBatch with document and all index updates atomically
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;
        let mut batch = WriteBatch::default();

        // Update document in batch
        batch.put_cf(&cf, Self::doc_key(key), &doc_bytes);

        // Record a version in the same atomic batch (if versioning is enabled).
        let versioned = self.is_versioned();
        if versioned {
            self.append_version_to_batch(&mut batch, &cf, key, Some(&new_value));
        }

        // Compute and apply index updates atomically
        let (entries_to_add, keys_to_remove, geo_entries_to_add, geo_keys_to_remove) =
            self.compute_index_entries_for_update(key, &old_value, &new_value)?;

        // Remove old index entries
        for key_to_remove in keys_to_remove {
            batch.delete_cf(&cf, key_to_remove);
        }
        for geo_key in geo_keys_to_remove {
            batch.delete_cf(&cf, geo_key);
        }

        // Add new index entries
        for (entry_key, entry_value) in entries_to_add {
            batch.put_cf(&cf, entry_key, entry_value);
        }
        for (entry_key, entry_value) in geo_entries_to_add {
            batch.put_cf(&cf, entry_key, entry_value);
        }

        // Compute and apply fulltext updates
        let fulltext_keys_to_remove = self.compute_fulltext_entries_for_delete(key, &old_value);
        for key_to_remove in fulltext_keys_to_remove {
            batch.delete_cf(&cf, key_to_remove);
        }

        let fulltext_entries_to_add = self.compute_fulltext_entries_for_insert(key, &new_value);
        for (entry_key, entry_value) in fulltext_entries_to_add {
            batch.put_cf(&cf, entry_key, entry_value);
        }

        // Compute and apply TTL expiry updates
        let (ttl_entries_to_add, ttl_keys_to_remove) =
            self.compute_ttl_expiry_entries_for_update(key, &old_value, &new_value);
        for key_to_remove in ttl_keys_to_remove {
            batch.delete_cf(&cf, key_to_remove);
        }
        for (entry_key, _entry_value) in ttl_entries_to_add {
            batch.put_cf(&cf, entry_key, Vec::new());
        }

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
        let _ = self.change_sender.send(ChangeEvent {
            type_: ChangeType::Update,
            key: key.to_string(),
            data: Some(new_value),
            old_data: Some(old_value),
        });

        Ok(doc)
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
        let doc_bytes = serialize_doc(&doc)?;

        // Build WriteBatch with document and all index updates atomically
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;
        let mut batch = WriteBatch::default();

        // Update document in batch
        batch.put_cf(&cf, Self::doc_key(key), &doc_bytes);

        // Record a version in the same atomic batch (if versioning is enabled).
        let versioned = self.is_versioned();
        if versioned {
            self.append_version_to_batch(&mut batch, &cf, key, Some(&new_value));
        }

        // Compute and apply index updates atomically
        let (entries_to_add, keys_to_remove, geo_entries_to_add, geo_keys_to_remove) =
            self.compute_index_entries_for_update(key, &old_value, &new_value)?;

        // Remove old index entries
        for key_to_remove in keys_to_remove {
            batch.delete_cf(&cf, key_to_remove);
        }
        for geo_key in geo_keys_to_remove {
            batch.delete_cf(&cf, geo_key);
        }

        // Add new index entries
        for (entry_key, entry_value) in entries_to_add {
            batch.put_cf(&cf, entry_key, entry_value);
        }
        for (entry_key, entry_value) in geo_entries_to_add {
            batch.put_cf(&cf, entry_key, entry_value);
        }

        // Compute and apply fulltext updates
        let fulltext_keys_to_remove = self.compute_fulltext_entries_for_delete(key, &old_value);
        for key_to_remove in fulltext_keys_to_remove {
            batch.delete_cf(&cf, key_to_remove);
        }

        let fulltext_entries_to_add = self.compute_fulltext_entries_for_insert(key, &new_value);
        for (entry_key, entry_value) in fulltext_entries_to_add {
            batch.put_cf(&cf, entry_key, entry_value);
        }

        // Compute and apply TTL expiry updates
        let (ttl_entries_to_add, ttl_keys_to_remove) =
            self.compute_ttl_expiry_entries_for_update(key, &old_value, &new_value);
        for key_to_remove in ttl_keys_to_remove {
            batch.delete_cf(&cf, key_to_remove);
        }
        for (entry_key, _entry_value) in ttl_entries_to_add {
            batch.put_cf(&cf, entry_key, Vec::new());
        }

        // Atomic write: document + all index updates together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to update document: {}", e)))?;

        // Invalidate document cache for this key
        let cache_key = format!("{}:{}", self.name, key);
        let cache = get_document_cache();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                cache.invalidate(&cache_key).await;
            });
        }

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
        let _ = self.change_sender.send(ChangeEvent {
            type_: ChangeType::Update,
            key: key.to_string(),
            data: Some(new_value),
            old_data: Some(old_value),
        });

        Ok(doc)
    }

    /// Delete a document with atomic document + index removal
    pub fn delete(&self, key: &str) -> DbResult<()> {
        // Get document for index cleanup
        let doc = self.get(key)?;
        let doc_value = doc.to_value();

        // Build WriteBatch with document deletion and all index removals atomically
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;
        let mut batch = WriteBatch::default();

        // Delete document from batch
        batch.delete_cf(&cf, Self::doc_key(key));

        // Record a delete tombstone version in the same atomic batch.
        let versioned = self.is_versioned();
        if versioned {
            self.append_version_to_batch(&mut batch, &cf, key, None);
        }

        // Compute and remove regular + geo index entries
        let (regular_keys, geo_keys) = self.compute_index_entries_for_delete(key, &doc_value)?;
        for key_to_remove in regular_keys {
            batch.delete_cf(&cf, key_to_remove);
        }
        for key_to_remove in geo_keys {
            batch.delete_cf(&cf, key_to_remove);
        }

        // Compute and remove fulltext entries
        let fulltext_keys = self.compute_fulltext_entries_for_delete(key, &doc_value);
        for key_to_remove in fulltext_keys {
            batch.delete_cf(&cf, key_to_remove);
        }

        // Compute and remove TTL expiry entries
        let ttl_keys = self.compute_ttl_expiry_entries_for_delete(key, &doc_value);
        for key_to_remove in ttl_keys {
            batch.delete_cf(&cf, key_to_remove);
        }

        // Atomic write: document deletion + index removals together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to delete document: {}", e)))?;

        // Invalidate document cache for this key
        let cache_key = format!("{}:{}", self.name, key);
        let cache = get_document_cache();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                cache.invalidate(&cache_key).await;
            });
        }

        // If blob collection, delete chunks (separate from WriteBatch)
        if self.collection_type.read().as_str() == "blob" {
            self.delete_blob_data(key)?;
        }

        // Update vector indexes in-memory (separate from WriteBatch)
        self.update_vector_indexes_on_delete(key);

        // Enforce version retention (best-effort, off the atomic write).
        if versioned {
            self.prune_versions(key);
        }

        // Update document count
        self.decrement_count();

        // Broadcast change event
        let _ = self.change_sender.send(ChangeEvent {
            type_: ChangeType::Delete,
            key: key.to_string(),
            data: None,
            old_data: Some(doc_value),
        });

        Ok(())
    }

    // ==================== Batch Operations ====================

    /// Batch upsert (insert or update) multiple documents - optimized for replication
    pub fn upsert_batch(&self, documents: Vec<(String, Value)>) -> DbResult<usize> {
        if documents.is_empty() {
            return Ok(0);
        }

        if self.collection_type.read().as_str() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Upsert (update) operations are not allowed on timeseries collections. Use insert_batch instead.".to_string(),
            ));
        }

        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;

        let mut batch = WriteBatch::default();
        let mut insert_count = 0;
        let mut upserted_docs: Vec<(String, Value)> = Vec::new();

        for (key, mut data) in documents {
            // Ensure _key is set
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_key".to_string(), Value::String(key.clone()));
            }

            // Check if document exists to determine insert vs update
            let exists = db.get_cf(&cf, Self::doc_key(&key)).ok().flatten().is_some();

            let doc = if exists {
                if let Ok(Some(bytes)) = db.get_cf(&cf, Self::doc_key(&key)) {
                    if let Ok(mut existing) = deserialize_doc(&bytes) {
                        existing.update(data);
                        existing
                    } else {
                        Document::with_key(&self.name, key.clone(), data)
                    }
                } else {
                    Document::with_key(&self.name, key.clone(), data)
                }
            } else {
                Document::with_key(&self.name, key.clone(), data)
            };

            if let Ok(doc_bytes) = serialize_doc(&doc) {
                batch.put_cf(&cf, Self::doc_key(&key), &doc_bytes);
                upserted_docs.push((key.clone(), doc.to_value()));
                if !exists {
                    insert_count += 1;
                }
            }
        }

        let count = batch.len();

        // Write all documents in one batch operation
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch upsert: {}", e)))?;

        // Update document count (only for new inserts)
        if insert_count > 0 {
            self.doc_count.fetch_add(insert_count, Ordering::Relaxed);
            self.count_dirty.store(true, Ordering::Relaxed);
        }

        // Update vector indexes for all upserted documents
        for (key, doc_value) in &upserted_docs {
            self.update_vector_indexes_on_upsert(key, doc_value);
        }
        // Persist vector indexes after batch
        self.persist_vector_indexes_throttled();

        Ok(count)
    }

    /// Batch delete documents with atomic document + index removal
    pub fn delete_batch(&self, keys: Vec<String>) -> DbResult<usize> {
        if keys.is_empty() {
            return Ok(0);
        }

        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;

        let mut batch = WriteBatch::default();
        let mut deleted_count = 0;
        let mut deleted_docs = Vec::new(); // To store doc_value for change events

        // Prepare batch with document deletions and index removals atomically
        for key in &keys {
            // Get document first (needed for index cleanup and change events)
            if let Ok(Some(bytes)) = db.get_cf(&cf, Self::doc_key(key)) {
                if let Ok(doc) = deserialize_doc(&bytes) {
                    let doc_value = doc.to_value();

                    // Add document deletion to batch
                    batch.delete_cf(&cf, Self::doc_key(key));

                    // Compute and add index key removals to batch
                    let (regular_keys, geo_keys) =
                        match self.compute_index_entries_for_delete(key, &doc_value) {
                            Ok(keys) => keys,
                            Err(e) => {
                                tracing::warn!("Failed to compute index keys for {}: {}", key, e);
                                (Vec::new(), Vec::new())
                            }
                        };
                    for key_to_remove in regular_keys {
                        batch.delete_cf(&cf, key_to_remove);
                    }
                    for key_to_remove in geo_keys {
                        batch.delete_cf(&cf, key_to_remove);
                    }

                    // Compute and add fulltext key removals to batch
                    let fulltext_keys = self.compute_fulltext_entries_for_delete(key, &doc_value);
                    for key_to_remove in fulltext_keys {
                        batch.delete_cf(&cf, key_to_remove);
                    }

                    // Compute and add TTL expiry key removals to batch
                    let ttl_keys = self.compute_ttl_expiry_entries_for_delete(key, &doc_value);
                    for key_to_remove in ttl_keys {
                        batch.delete_cf(&cf, key_to_remove);
                    }

                    // Handle blobs (separate from WriteBatch)
                    if self.collection_type.read().as_str() == "blob" {
                        let _ = self.delete_blob_data(key);
                    }

                    // Update vector indexes in-memory (separate from WriteBatch)
                    self.update_vector_indexes_on_delete(key);

                    deleted_docs.push((key.clone(), doc_value));
                    deleted_count += 1;
                }
            }
        }

        if deleted_count == 0 {
            return Ok(0);
        }

        // Persist vector indexes after batch delete
        self.persist_vector_indexes_throttled();

        // Commit batch atomically: all document deletions + index removals together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch delete: {}", e)))?;

        // Update count. Saturating: several Collection instances can exist
        // for the same CF (engine cache, Database cache, fresh handles), each
        // with its own counter — a plain fetch_sub on an instance that didn't
        // see the inserts wraps to u64::MAX and the UI shows
        // 18446744073709551615 documents.
        let _ = self
            .doc_count
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(deleted_count))
            });
        self.count_dirty.store(true, Ordering::Relaxed);

        // Send Change Events
        for (key, old_data) in deleted_docs {
            let _ = self.change_sender.send(ChangeEvent {
                type_: ChangeType::Delete,
                key,
                data: None,
                old_data: Some(old_data),
            });
        }

        Ok(deleted_count)
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

        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;

        let mut batch = WriteBatch::default();
        let mut updated_docs = Vec::new();
        let mut change_events = Vec::new();

        // Prepare batch with document updates and index updates atomically
        for (key, changes) in updates {
            // Get old document
            if let Ok(old_doc) = self.get(key) {
                let old_value = old_doc.to_value();

                // Create updated document
                let mut doc = old_doc;
                doc.update(changes.clone());
                let new_value = doc.to_value();

                // Validate edge documents after update
                if self.collection_type.read().as_str() == "edge" {
                    if let Err(e) = self.validate_edge_document(&new_value) {
                        tracing::warn!("Failed to validate edge for {}: {}", key, e);
                        continue;
                    }
                }

                // Validate against JSON schema if defined
                if let Some(validator) = self.get_cached_schema_validator()? {
                    if let Err(e) = validator.validate(&new_value) {
                        tracing::warn!("Schema validation failed for {}: {}", key, e);
                        continue;
                    }
                }

                // Serialize document
                if let Ok(doc_bytes) = serialize_doc(&doc) {
                    // Add document to batch
                    batch.put_cf(&cf, Self::doc_key(key), &doc_bytes);

                    // Compute and add index updates to batch atomically
                    let (entries_to_add, keys_to_remove, geo_entries_to_add, geo_keys_to_remove) =
                        match self.compute_index_entries_for_update(key, &old_value, &new_value) {
                            Ok(result) => result,
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to compute index updates for {}: {}",
                                    key,
                                    e
                                );
                                (Vec::new(), Vec::new(), Vec::new(), Vec::new())
                            }
                        };

                    // Remove old index entries
                    for key_to_remove in keys_to_remove {
                        batch.delete_cf(&cf, key_to_remove);
                    }
                    for geo_key in geo_keys_to_remove {
                        batch.delete_cf(&cf, geo_key);
                    }

                    // Add new index entries
                    for (entry_key, entry_value) in entries_to_add {
                        batch.put_cf(&cf, entry_key, entry_value);
                    }
                    for (entry_key, entry_value) in geo_entries_to_add {
                        batch.put_cf(&cf, entry_key, entry_value);
                    }

                    // Compute and add fulltext updates to batch
                    let fulltext_keys_to_remove =
                        self.compute_fulltext_entries_for_delete(key, &old_value);
                    for key_to_remove in fulltext_keys_to_remove {
                        batch.delete_cf(&cf, key_to_remove);
                    }

                    let fulltext_entries_to_add =
                        self.compute_fulltext_entries_for_insert(key, &new_value);
                    for (entry_key, entry_value) in fulltext_entries_to_add {
                        batch.put_cf(&cf, entry_key, entry_value);
                    }

                    // Compute and apply TTL expiry updates
                    let (ttl_entries_to_add, ttl_keys_to_remove) =
                        self.compute_ttl_expiry_entries_for_update(key, &old_value, &new_value);
                    for key_to_remove in ttl_keys_to_remove {
                        batch.delete_cf(&cf, key_to_remove);
                    }
                    for (entry_key, _entry_value) in ttl_entries_to_add {
                        batch.put_cf(&cf, entry_key, Vec::new());
                    }

                    // Update vector indexes in-memory (separate from
                    // WriteBatch). Skip the delete+reinsert when the embedding is
                    // unchanged so a bulk update that only rewrites metadata
                    // (e.g. an incremental graph sync) pays no HNSW churn and
                    // doesn't dirty the index into a full re-serialize.
                    if !self.vector_index_unchanged(&old_value, &new_value) {
                        self.update_vector_indexes_on_delete(key);
                        self.update_vector_indexes_on_upsert(key, &new_value);
                    }

                    change_events.push((key.clone(), old_value, new_value));
                    updated_docs.push(doc);
                }
            }
        }

        if updated_docs.is_empty() {
            return Ok(Vec::new());
        }

        // Persist vector indexes after batch update
        self.persist_vector_indexes_throttled();

        // Commit batch atomically: all document updates + index updates together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch update: {}", e)))?;

        // Send Change Events
        for (key, old_data, new_data) in change_events {
            let _ = self.change_sender.send(ChangeEvent {
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

        let db = &self.db;
        let cf = db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped mid-operation)",
                self.name
            ))
        })?;

        let mut batch = WriteBatch::default();
        let mut inserted_docs = Vec::with_capacity(documents.len());
        let mut doc_values: Vec<(String, Value)> = Vec::with_capacity(documents.len());

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

            // Extract or generate key
            let key = if let Some(obj) = data.as_object_mut() {
                if let Some(key_value) = obj.remove("_key") {
                    if let Some(key_str) = key_value.as_str() {
                        key_str.to_string()
                    } else {
                        return Err(DbError::InvalidDocument(
                            "_key must be a string".to_string(),
                        ));
                    }
                } else {
                    uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
                }
            } else {
                uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
            };

            // Check for duplicate _key within the batch
            if doc_values.iter().any(|(k, _)| k == &key) {
                return Err(DbError::InvalidDocument(format!(
                    "Duplicate _key '{}' within batch",
                    key
                )));
            }

            // Check if document with this key already exists in the DB
            if db
                .get_cf(&cf, Self::doc_key(&key))
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to check existing key: {}", e))
                })?
                .is_some()
            {
                return Err(DbError::InvalidDocument(format!(
                    "Document with _key '{}' already exists",
                    key
                )));
            }

            let doc = Document::with_key(&self.name, key.clone(), data);
            let doc_value = doc.to_value();

            // Check unique constraints
            self.check_unique_constraints(&key, &doc_value)?;

            let doc_bytes = serialize_doc(&doc)?;

            // Add document to batch
            batch.put_cf(&cf, Self::doc_key(&key), &doc_bytes);

            // Compute and add regular + geo index entries
            let (regular_entries, geo_entries) =
                self.compute_index_entries_for_insert(&key, &doc_value)?;
            for (entry_key, entry_value) in regular_entries {
                batch.put_cf(&cf, entry_key, entry_value);
            }
            for (entry_key, entry_value) in geo_entries {
                batch.put_cf(&cf, entry_key, entry_value);
            }

            // Compute and add fulltext entries
            let fulltext_entries = self.compute_fulltext_entries_for_insert(&key, &doc_value);
            for (entry_key, entry_value) in fulltext_entries {
                batch.put_cf(&cf, entry_key, entry_value);
            }

            // Compute and add TTL expiry entries
            let ttl_expiry_entries = self.compute_ttl_expiry_entries_for_insert(&key, &doc_value);
            for (entry_key, _entry_value) in ttl_expiry_entries {
                batch.put_cf(&cf, entry_key, Vec::new());
            }

            doc_values.push((key, doc_value));
            inserted_docs.push(doc);
        }

        // Atomic write: all documents + indexes together
        db.write(&batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch insert: {}", e)))?;

        // Update vector indexes in-memory (separate from WriteBatch)
        for (key, doc_value) in &doc_values {
            self.update_vector_indexes_on_upsert(key, doc_value);
        }
        // Persist vector indexes after batch
        self.persist_vector_indexes_throttled();

        // Update document count
        let count = inserted_docs.len();
        self.doc_count.fetch_add(count, Ordering::Relaxed);
        self.count_dirty.store(true, Ordering::Relaxed);

        // Broadcast change events
        for (key, doc_value) in doc_values {
            let _ = self.change_sender.send(ChangeEvent {
                type_: ChangeType::Insert,
                key,
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
            let prefix = DOC_PREFIX.as_bytes();
            let count = db
                .prefix_iterator_cf(&cf, prefix)
                .take_while(|r| r.as_ref().is_ok_and(|(k, _)| k.starts_with(prefix)))
                .count();

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
        self.chunk_count.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Recount documents from actual RocksDB data (slow but accurate)
    pub fn recount_documents(&self) -> usize {
        if let Some(cf) = self.db.cf_handle(&self.name) {
            let prefix = DOC_PREFIX.as_bytes();
            let actual_count = self
                .db
                .prefix_iterator_cf(&cf, prefix)
                .take_while(|r| r.as_ref().is_ok_and(|(k, _)| k.starts_with(prefix)))
                .count();

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
        // nested `doc:ttl_exp:` expiry-index entries.
        let data_ranges: [(&[u8], &[u8]); 9] = [
            (b"doc:", b"doc;"),         // documents + TTL expiry entries
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
        self.chunk_count.store(0, Ordering::Relaxed);
        self.count_dirty.store(true, Ordering::Relaxed);
        self.flush_stats();

        // One broadcast event instead of one-per-document: subscribers
        // (LiveQuery, stream processors) learn the collection was cleared
        // without being flooded.
        if count > 0 {
            let _ = self.change_sender.send(ChangeEvent {
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
