use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::schema::SchemaValidator;
use rocksdb::WriteBatch;
use serde_json::Value;
use std::sync::atomic::Ordering;

impl Collection {
    // ==================== Basic CRUD ====================

    /// Get a document by key
    pub fn get(&self, key: &str) -> DbResult<Document> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let bytes = db
            .get_cf(cf, Self::doc_key(key))
            .map_err(|e| DbError::InternalError(format!("Failed to get document: {}", e)))?
            .ok_or_else(|| DbError::DocumentNotFound(key.to_string()))?;

        let doc: Document = serde_json::from_slice(&bytes)?;
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

    /// Internal insert implementation
    pub(crate) fn insert_internal(&self, mut data: Value, update_indexes: bool) -> DbResult<Document> {
        // Validate edge documents
        if *self.collection_type.read().unwrap() == "edge" {
            self.validate_edge_document(&data)?;
        }

        // Validate against JSON schema if defined
        if let Some(schema) = self.get_json_schema() {
            let validator = SchemaValidator::new(schema).map_err(|e| {
                DbError::InvalidDocument(format!("Schema compilation error: {}", e))
            })?;
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

        // Check unique constraints BEFORE saving the document (only if indexes are enabled)
        if update_indexes {
            self.check_unique_constraints(&key, &doc_value)?;
        }

        // Store document
        let doc_bytes = serde_json::to_vec(&doc)?;
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::doc_key(&key), &doc_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to insert document: {}", e)))?;
        }

        // Update indexes (if enabled)
        if update_indexes {
            self.update_indexes_on_insert(&key, &doc_value)?;
            self.update_fulltext_on_insert(&key, &doc_value)?;
            // Update vector indexes
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

    /// Update a document
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
        if let Some(schema) = self.get_json_schema() {
            let validator = SchemaValidator::new(schema).map_err(|e| {
                DbError::InvalidDocument(format!("Schema compilation error: {}", e))
            })?;
            validator.validate(&new_value).map_err(|e| {
                DbError::InvalidDocument(format!("Schema validation failed: {}", e))
            })?;
        }

        let doc_bytes = serde_json::to_vec(&doc)?;

        // Store updated document
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::doc_key(key), &doc_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to update document: {}", e)))?;
        }

        // Update indexes
        self.update_indexes_on_update(key, &old_value, &new_value)?;

        // Update fulltext indexes (delete old, insert new)
        self.update_fulltext_on_delete(key, &old_value)?;
        self.update_fulltext_on_insert(key, &new_value)?;

        // Update vector indexes (remove old, add new)
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
        let doc_bytes = serde_json::to_vec(&doc)?;

        // Store updated document
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::doc_key(key), &doc_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to update document: {}", e)))?;
        }

        // Update indexes
        self.update_indexes_on_update(key, &old_value, &new_value)?;

        // Update fulltext indexes (delete old, insert new)
        self.update_fulltext_on_delete(key, &old_value)?;
        self.update_fulltext_on_insert(key, &new_value)?;

        // Update vector indexes (remove old, add new)
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

    /// Delete a document
    pub fn delete(&self, key: &str) -> DbResult<()> {
        // Get document for index cleanup
        let doc = self.get(key)?;
        let doc_value = doc.to_value();

        // Delete document
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.delete_cf(cf, Self::doc_key(key))
                .map_err(|e| DbError::InternalError(format!("Failed to delete document: {}", e)))?;
        }

        // If blob collection, delete chunks
        if *self.collection_type.read().unwrap() == "blob" {
            self.delete_blob_data(key)?;
        }

        // Update indexes
        self.update_indexes_on_delete(key, &doc_value)?;

        // Update fulltext indexes
        self.update_fulltext_on_delete(key, &doc_value)?;

        // Update vector indexes
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

        let db = self.db.read().unwrap();
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
                // Update existing document
                if let Ok(bytes) = db.get_cf(cf, Self::doc_key(&key)) {
                    if let Some(bytes) = bytes {
                        if let Ok(mut existing) = serde_json::from_slice::<Document>(&bytes) {
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
                }
            } else {
                Document::with_key(&self.name, key.clone(), data)
            };

            if let Ok(doc_bytes) = serde_json::to_vec(&doc) {
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
            self.doc_count
                .fetch_add(insert_count, Ordering::Relaxed);
            self.count_dirty
                .store(true, Ordering::Relaxed);
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

    /// Batch delete documents
    pub fn delete_batch(&self, keys: Vec<String>) -> DbResult<usize> {
        use rocksdb::WriteBatch;

        if keys.is_empty() {
            return Ok(0);
        }

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut batch = WriteBatch::default();
        let mut deleted_count = 0;
        let mut deleted_docs = Vec::new(); // To store doc_value for change events

        // 1. Prepare batch and handle auxiliary updates (indexes, blobs)
        for key in &keys {
            // Get document first (needed for index cleanup and change events)
            if let Ok(Some(bytes)) = db.get_cf(cf, Self::doc_key(key)) {
                if let Ok(doc) = serde_json::from_slice::<Document>(&bytes) {
                    let doc_value = doc.to_value();

                    // Add to batch for deletion
                    batch.delete_cf(cf, Self::doc_key(key));

                    // Handle blobs
                    if *self.collection_type.read().unwrap() == "blob" {
                        let _ = self.delete_blob_data(key);
                    }

                    // Update indexes (Note: these are separate writes currently)
                    if let Err(e) = self.update_indexes_on_delete(key, &doc_value) {
                        tracing::warn!("Failed to clean indexes for {}: {}", key, e);
                    }
                    if let Err(e) = self.update_fulltext_on_delete(key, &doc_value) {
                        tracing::warn!("Failed to clean fulltext for {}: {}", key, e);
                    }
                    // Update vector indexes
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

        // 2. Commit storage batch
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch delete: {}", e)))?;

        // 3. Update count
        self.doc_count
            .fetch_sub(deleted_count, Ordering::Relaxed);
        self.count_dirty
            .store(true, Ordering::Relaxed);

        // 4. Send Change Events
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

    /// Batch update multiple documents
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

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut batch = WriteBatch::default();
        let mut updated_docs = Vec::new();
        let mut change_events = Vec::new();

        // 1. Prepare batch and handle auxiliary updates (indexes, validation)
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
                if let Some(schema) = self.get_json_schema() {
                    if let Ok(validator) = SchemaValidator::new(schema) {
                        if let Err(e) = validator.validate(&new_value) {
                            tracing::warn!("Schema validation failed for {}: {}", key, e);
                            continue;
                        }
                    }
                }

                // Serialize document
                if let Ok(doc_bytes) = serde_json::to_vec(&doc) {
                    // Add to batch
                    batch.put_cf(cf, Self::doc_key(key), &doc_bytes);

                    // Update indexes (Note: these are separate writes currently)
                    if let Err(e) = self.update_indexes_on_update(key, &old_value, &new_value) {
                        tracing::warn!("Failed to update indexes for {}: {}", key, e);
                    }

                    // Update fulltext indexes (delete old, insert new)
                    if let Err(e) = self.update_fulltext_on_delete(key, &old_value) {
                        tracing::warn!("Failed to clean fulltext for {}: {}", key, e);
                    }
                    if let Err(e) = self.update_fulltext_on_insert(key, &new_value) {
                        tracing::warn!("Failed to update fulltext for {}: {}", key, e);
                    }

                    // Update vector indexes (remove old, add new)
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

        // 2. Commit storage batch
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch update: {}", e)))?;

        // 3. Send Change Events
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

    /// Insert multiple documents
    pub fn insert_batch(&self, documents: Vec<Value>) -> DbResult<Vec<Document>> {
        let mut inserted_docs = Vec::new();
        for doc_data in documents {
            // Re-use single insert which handles validation, indexing, etc.
            // This is slower than batch write but safer for consistency with indexes.
            // Converting to batch write would be an optimization similar to upsert_batch.
            let doc = self.insert(doc_data)?;
            inserted_docs.push(doc);
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
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let prefix = DOC_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        let iter = iter.filter_map(|result| {
            result.ok().and_then(|(key, value)| {
                // Check if key starts with doc prefix
                if key.starts_with(prefix) {
                    serde_json::from_slice(&value).ok()
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
        let db = self.db.read().unwrap();
        if let Some(cf) = db.cf_handle(&self.name) {
            let prefix = DOC_PREFIX.as_bytes();
            let count = db
                .prefix_iterator_cf(cf, prefix)
                .take_while(|r| r.as_ref().map_or(false, |(k, _)| k.starts_with(prefix)))
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
        let db_guard = self.db.read().unwrap();
        if let Some(cf) = db_guard.cf_handle(&self.name) {
            let prefix = DOC_PREFIX.as_bytes();
            let actual_count = db_guard
                .prefix_iterator_cf(cf, prefix)
                .take_while(|r| r.as_ref().map_or(false, |(k, _)| k.starts_with(prefix)))
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
    pub fn prune_older_than(&self, _timestamp: u64) -> DbResult<usize> {
        // TODO: Implement pruning logic based on collection type/structure
        // For now, return 0 to allow compilation.
        tracing::warn!("prune_older_than not fully implemented in refactor");
        Ok(0)
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
