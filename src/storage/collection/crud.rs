use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::serializer::{deserialize_doc, serialize_doc};
use rocksdb::WriteBatch;
use serde_json::Value;
use std::sync::atomic::Ordering;

impl Collection {
    // ==================== Basic CRUD ====================

    /// Get a document by key
    pub fn get(&self, key: &str) -> DbResult<Document> {
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let bytes = db
            .get_cf(cf, Self::doc_key(key))
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
        if *self.collection_type.read().unwrap() == "edge" {
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
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let mut batch = WriteBatch::default();

        // Add document to batch
        batch.put_cf(cf, Self::doc_key(&key), &doc_bytes);

        // Add index entries to batch (if enabled)
        if update_indexes {
            // Compute and add regular + geo index entries
            let (regular_entries, geo_entries) =
                self.compute_index_entries_for_insert(&key, &doc_value)?;
            for (entry_key, entry_value) in regular_entries {
                batch.put_cf(cf, entry_key, entry_value);
            }
            for (entry_key, entry_value) in geo_entries {
                batch.put_cf(cf, entry_key, entry_value);
            }

            // Compute and add fulltext entries
            let fulltext_entries = self.compute_fulltext_entries_for_insert(&key, &doc_value);
            for (entry_key, entry_value) in fulltext_entries {
                batch.put_cf(cf, entry_key, entry_value);
            }

            // Compute and add TTL expiry entries
            let ttl_expiry_entries = self.compute_ttl_expiry_entries_for_insert(&key, &doc_value);
            for (entry_key, _entry_value) in ttl_expiry_entries {
                batch.put_cf(cf, entry_key, Vec::new());
            }
        }

        // Atomic write: document + indexes together
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to insert document: {}", e)))?;

        // Update vector indexes in-memory (separate from WriteBatch)
        if update_indexes {
            self.update_vector_indexes_on_upsert(&key, &doc_value);
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
        if *self.collection_type.read().unwrap() == "timeseries" {
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
        if *self.collection_type.read().unwrap() == "edge" {
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
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let mut batch = WriteBatch::default();

        // Update document in batch
        batch.put_cf(cf, Self::doc_key(key), &doc_bytes);

        // Compute and apply index updates atomically
        let (entries_to_add, keys_to_remove, geo_entries_to_add, geo_keys_to_remove) =
            self.compute_index_entries_for_update(key, &old_value, &new_value)?;

        // Remove old index entries
        for key_to_remove in keys_to_remove {
            batch.delete_cf(cf, key_to_remove);
        }
        for geo_key in geo_keys_to_remove {
            batch.delete_cf(cf, geo_key);
        }

        // Add new index entries
        for (entry_key, entry_value) in entries_to_add {
            batch.put_cf(cf, entry_key, entry_value);
        }
        for (entry_key, entry_value) in geo_entries_to_add {
            batch.put_cf(cf, entry_key, entry_value);
        }

        // Compute and apply fulltext updates
        let fulltext_keys_to_remove = self.compute_fulltext_entries_for_delete(key, &old_value);
        for key_to_remove in fulltext_keys_to_remove {
            batch.delete_cf(cf, key_to_remove);
        }

        let fulltext_entries_to_add = self.compute_fulltext_entries_for_insert(key, &new_value);
        for (entry_key, entry_value) in fulltext_entries_to_add {
            batch.put_cf(cf, entry_key, entry_value);
        }

        // Compute and apply TTL expiry updates
        let (ttl_entries_to_add, ttl_keys_to_remove) =
            self.compute_ttl_expiry_entries_for_update(key, &old_value, &new_value);
        for key_to_remove in ttl_keys_to_remove {
            batch.delete_cf(cf, key_to_remove);
        }
        for (entry_key, _entry_value) in ttl_entries_to_add {
            batch.put_cf(cf, entry_key, Vec::new());
        }

        // Atomic write: document + all index updates together
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to update document: {}", e)))?;

        // Update vector indexes in-memory (separate from WriteBatch)
        self.update_vector_indexes_on_delete(key);
        self.update_vector_indexes_on_upsert(key, &new_value);

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
        if *self.collection_type.read().unwrap() == "timeseries" {
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
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let mut batch = WriteBatch::default();

        // Update document in batch
        batch.put_cf(cf, Self::doc_key(key), &doc_bytes);

        // Compute and apply index updates atomically
        let (entries_to_add, keys_to_remove, geo_entries_to_add, geo_keys_to_remove) =
            self.compute_index_entries_for_update(key, &old_value, &new_value)?;

        // Remove old index entries
        for key_to_remove in keys_to_remove {
            batch.delete_cf(cf, key_to_remove);
        }
        for geo_key in geo_keys_to_remove {
            batch.delete_cf(cf, geo_key);
        }

        // Add new index entries
        for (entry_key, entry_value) in entries_to_add {
            batch.put_cf(cf, entry_key, entry_value);
        }
        for (entry_key, entry_value) in geo_entries_to_add {
            batch.put_cf(cf, entry_key, entry_value);
        }

        // Compute and apply fulltext updates
        let fulltext_keys_to_remove = self.compute_fulltext_entries_for_delete(key, &old_value);
        for key_to_remove in fulltext_keys_to_remove {
            batch.delete_cf(cf, key_to_remove);
        }

        let fulltext_entries_to_add = self.compute_fulltext_entries_for_insert(key, &new_value);
        for (entry_key, entry_value) in fulltext_entries_to_add {
            batch.put_cf(cf, entry_key, entry_value);
        }

        // Compute and apply TTL expiry updates
        let (ttl_entries_to_add, ttl_keys_to_remove) =
            self.compute_ttl_expiry_entries_for_update(key, &old_value, &new_value);
        for key_to_remove in ttl_keys_to_remove {
            batch.delete_cf(cf, key_to_remove);
        }
        for (entry_key, _entry_value) in ttl_entries_to_add {
            batch.put_cf(cf, entry_key, Vec::new());
        }

        // Atomic write: document + all index updates together
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to update document: {}", e)))?;

        // Update vector indexes in-memory (separate from WriteBatch)
        self.update_vector_indexes_on_delete(key);
        self.update_vector_indexes_on_upsert(key, &new_value);

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
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let mut batch = WriteBatch::default();

        // Delete document from batch
        batch.delete_cf(cf, Self::doc_key(key));

        // Compute and remove regular + geo index entries
        let (regular_keys, geo_keys) = self.compute_index_entries_for_delete(key, &doc_value)?;
        for key_to_remove in regular_keys {
            batch.delete_cf(cf, key_to_remove);
        }
        for key_to_remove in geo_keys {
            batch.delete_cf(cf, key_to_remove);
        }

        // Compute and remove fulltext entries
        let fulltext_keys = self.compute_fulltext_entries_for_delete(key, &doc_value);
        for key_to_remove in fulltext_keys {
            batch.delete_cf(cf, key_to_remove);
        }

        // Compute and remove TTL expiry entries
        let ttl_keys = self.compute_ttl_expiry_entries_for_delete(key, &doc_value);
        for key_to_remove in ttl_keys {
            batch.delete_cf(cf, key_to_remove);
        }

        // Atomic write: document deletion + index removals together
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to delete document: {}", e)))?;

        // If blob collection, delete chunks (separate from WriteBatch)
        if *self.collection_type.read().unwrap() == "blob" {
            self.delete_blob_data(key)?;
        }

        // Update vector indexes in-memory (separate from WriteBatch)
        self.update_vector_indexes_on_delete(key);

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

        if *self.collection_type.read().unwrap() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Upsert (update) operations are not allowed on timeseries collections. Use insert_batch instead.".to_string(),
            ));
        }

        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut batch = WriteBatch::default();
        let mut insert_count = 0;
        let mut upserted_docs: Vec<(String, Value)> = Vec::new();

        for (key, mut data) in documents {
            // Ensure _key is set
            if let Some(obj) = data.as_object_mut() {
                obj.insert("_key".to_string(), Value::String(key.clone()));
            }

            // Check if document exists to determine insert vs update
            let exists = db.get_cf(cf, Self::doc_key(&key)).ok().flatten().is_some();

            let doc = if exists {
                if let Ok(Some(bytes)) = db.get_cf(cf, Self::doc_key(&key)) {
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
                batch.put_cf(cf, Self::doc_key(&key), &doc_bytes);
                upserted_docs.push((key.clone(), doc.to_value()));
                if !exists {
                    insert_count += 1;
                }
            }
        }

        let count = batch.len();

        // Write all documents in one batch operation
        db.write(batch)
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
        if let Err(e) = self.persist_vector_indexes() {
            tracing::warn!("Failed to persist vector indexes: {}", e);
        }

        Ok(count)
    }

    /// Batch delete documents with atomic document + index removal
    pub fn delete_batch(&self, keys: Vec<String>) -> DbResult<usize> {
        if keys.is_empty() {
            return Ok(0);
        }

        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut batch = WriteBatch::default();
        let mut deleted_count = 0;
        let mut deleted_docs = Vec::new(); // To store doc_value for change events

        // Prepare batch with document deletions and index removals atomically
        for key in &keys {
            // Get document first (needed for index cleanup and change events)
            if let Ok(Some(bytes)) = db.get_cf(cf, Self::doc_key(key)) {
                if let Ok(doc) = deserialize_doc(&bytes) {
                    let doc_value = doc.to_value();

                    // Add document deletion to batch
                    batch.delete_cf(cf, Self::doc_key(key));

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
                        batch.delete_cf(cf, key_to_remove);
                    }
                    for key_to_remove in geo_keys {
                        batch.delete_cf(cf, key_to_remove);
                    }

                    // Compute and add fulltext key removals to batch
                    let fulltext_keys = self.compute_fulltext_entries_for_delete(key, &doc_value);
                    for key_to_remove in fulltext_keys {
                        batch.delete_cf(cf, key_to_remove);
                    }

                    // Compute and add TTL expiry key removals to batch
                    let ttl_keys = self.compute_ttl_expiry_entries_for_delete(key, &doc_value);
                    for key_to_remove in ttl_keys {
                        batch.delete_cf(cf, key_to_remove);
                    }

                    // Handle blobs (separate from WriteBatch)
                    if *self.collection_type.read().unwrap() == "blob" {
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
        if let Err(e) = self.persist_vector_indexes() {
            tracing::warn!("Failed to persist vector indexes: {}", e);
        }

        // Commit batch atomically: all document deletions + index removals together
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch delete: {}", e)))?;

        // Update count
        self.doc_count.fetch_sub(deleted_count, Ordering::Relaxed);
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
        if *self.collection_type.read().unwrap() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Update operations are not allowed on timeseries collections".to_string(),
            ));
        }

        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

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
                if *self.collection_type.read().unwrap() == "edge" {
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
                    batch.put_cf(cf, Self::doc_key(key), &doc_bytes);

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
                        batch.delete_cf(cf, key_to_remove);
                    }
                    for geo_key in geo_keys_to_remove {
                        batch.delete_cf(cf, geo_key);
                    }

                    // Add new index entries
                    for (entry_key, entry_value) in entries_to_add {
                        batch.put_cf(cf, entry_key, entry_value);
                    }
                    for (entry_key, entry_value) in geo_entries_to_add {
                        batch.put_cf(cf, entry_key, entry_value);
                    }

                    // Compute and add fulltext updates to batch
                    let fulltext_keys_to_remove =
                        self.compute_fulltext_entries_for_delete(key, &old_value);
                    for key_to_remove in fulltext_keys_to_remove {
                        batch.delete_cf(cf, key_to_remove);
                    }

                    let fulltext_entries_to_add =
                        self.compute_fulltext_entries_for_insert(key, &new_value);
                    for (entry_key, entry_value) in fulltext_entries_to_add {
                        batch.put_cf(cf, entry_key, entry_value);
                    }

                    // Compute and apply TTL expiry updates
                    let (ttl_entries_to_add, ttl_keys_to_remove) =
                        self.compute_ttl_expiry_entries_for_update(key, &old_value, &new_value);
                    for key_to_remove in ttl_keys_to_remove {
                        batch.delete_cf(cf, key_to_remove);
                    }
                    for (entry_key, _entry_value) in ttl_entries_to_add {
                        batch.put_cf(cf, entry_key, Vec::new());
                    }

                    // Update vector indexes in-memory (separate from WriteBatch)
                    self.update_vector_indexes_on_delete(key);
                    self.update_vector_indexes_on_upsert(key, &new_value);

                    change_events.push((key.clone(), old_value, new_value));
                    updated_docs.push(doc);
                }
            }
        }

        if updated_docs.is_empty() {
            return Ok(Vec::new());
        }

        // Persist vector indexes after batch update
        if let Err(e) = self.persist_vector_indexes() {
            tracing::warn!("Failed to persist vector indexes: {}", e);
        }

        // Commit batch atomically: all document updates + index updates together
        db.write(batch)
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

        let is_edge = *self.collection_type.read().unwrap() == "edge";
        let schema_validator = self.get_cached_schema_validator()?;

        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

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

            let doc = Document::with_key(&self.name, key.clone(), data);
            let doc_value = doc.to_value();

            // Check unique constraints
            self.check_unique_constraints(&key, &doc_value)?;

            let doc_bytes = serialize_doc(&doc)?;

            // Add document to batch
            batch.put_cf(cf, Self::doc_key(&key), &doc_bytes);

            // Compute and add regular + geo index entries
            let (regular_entries, geo_entries) =
                self.compute_index_entries_for_insert(&key, &doc_value)?;
            for (entry_key, entry_value) in regular_entries {
                batch.put_cf(cf, entry_key, entry_value);
            }
            for (entry_key, entry_value) in geo_entries {
                batch.put_cf(cf, entry_key, entry_value);
            }

            // Compute and add fulltext entries
            let fulltext_entries = self.compute_fulltext_entries_for_insert(&key, &doc_value);
            for (entry_key, entry_value) in fulltext_entries {
                batch.put_cf(cf, entry_key, entry_value);
            }

            // Compute and add TTL expiry entries
            let ttl_expiry_entries = self.compute_ttl_expiry_entries_for_insert(&key, &doc_value);
            for (entry_key, _entry_value) in ttl_expiry_entries {
                batch.put_cf(cf, entry_key, Vec::new());
            }

            doc_values.push((key, doc_value));
            inserted_docs.push(doc);
        }

        // Atomic write: all documents + indexes together
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch insert: {}", e)))?;

        // Update vector indexes in-memory (separate from WriteBatch)
        for (key, doc_value) in &doc_values {
            self.update_vector_indexes_on_upsert(key, doc_value);
        }
        // Persist vector indexes after batch
        if let Err(e) = self.persist_vector_indexes() {
            tracing::warn!("Failed to persist vector indexes: {}", e);
        }

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
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let prefix = DOC_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

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

    // ==================== Counters ====================

    /// Recalculate document count from storage
    pub fn recalculate_count(&self) -> usize {
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        if let Some(cf) = db.cf_handle(&self.name) {
            let prefix = DOC_PREFIX.as_bytes();
            let count = db
                .prefix_iterator_cf(cf, prefix)
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

    /// Recount documents from actual RocksDB data (slow but accurate)
    pub fn recount_documents(&self) -> usize {
        if let Some(cf) = self.db.cf_handle(&self.name) {
            let prefix = DOC_PREFIX.as_bytes();
            let actual_count = self
                .db
                .prefix_iterator_cf(cf, prefix)
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

    /// Decrement document count (called on delete) - atomic, no disk I/O
    pub(crate) fn decrement_count(&self) {
        self.doc_count.fetch_sub(1, Ordering::Relaxed);
        self.count_dirty.store(true, Ordering::Relaxed);
    }

    // ==================== Maintenance ====================

    /// Truncate collection (delete all documents)
    pub fn truncate(&self) -> DbResult<usize> {
        let docs = self.all();
        let count = docs.len();
        if count == 0 {
            return Ok(0);
        }

        let keys: Vec<String> = docs.iter().map(|d| d.key.clone()).collect();
        self.delete_batch(keys)
    }

    /// Prune documents older than timestamp (for timeseries)
    /// The timestamp is in milliseconds since Unix epoch.
    /// This extracts the timestamp from UUIDv7 keys and deletes matching documents.
    pub fn prune_older_than(&self, timestamp_ms: u64) -> DbResult<usize> {
        // Collect keys to delete
        // Lock-free: RocksDB is thread-safe for reads
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let prefix = DOC_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

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
