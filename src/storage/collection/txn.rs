use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::serializer::serialize_doc;
use crate::transaction::wal::WalWriter;
use crate::transaction::{Operation, Transaction};
use rocksdb::WriteBatch;
use serde_json::Value;
use uuid;

impl Collection {
    // ==================== Transactional Operations ====================

    /// Insert a document within a transaction
    pub fn insert_tx(
        &self,
        tx: &mut Transaction,
        _wal: &Arc<WalWriter>,
        mut data: Value,
    ) -> DbResult<Document> {
        // Validation similar to insert
        if *self.collection_type.read().unwrap() == "edge" {
            self.validate_edge_document(&data)?;
        }

        // Generate key if needed
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

        // Check uniqueness in DB (snapshot isolation?)
        // Standard check_unique_constraints checks CURRENT db state.
        // It does NOT see uncommitted changes in other transactions,
        // nor this transaction's previous ops without extra logic. (Simplified)
        self.check_unique_constraints(&key, &doc.to_value())?;

        // Parse database and collection from self.name (format: "db:coll")
        let (db_name, coll_name) = self.name.split_once(':').unwrap_or(("", &self.name));

        // Add to transaction log
        tx.add_operation(Operation::Insert {
            database: db_name.to_string(),
            collection: coll_name.to_string(),
            key: key.clone(),
            data: doc.to_value(),
        });

        Ok(doc)
    }

    /// Update a document within a transaction
    pub fn update_tx(
        &self,
        tx: &mut Transaction,
        _wal: &Arc<WalWriter>,
        key: &str,
        data: Value,
    ) -> DbResult<Document> {
        if *self.collection_type.read().unwrap() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Update operations are not allowed on timeseries collections".to_string(),
            ));
        }

        // Get CURRENT document (from DB)
        let mut doc = self.get(key)?;
        let old_data = doc.to_value();

        // Apply update
        doc.update(data);

        // Validation
        if *self.collection_type.read().unwrap() == "edge" {
            self.validate_edge_document(&doc.to_value())?;
        }

        // Parse database and collection from self.name (format: "db:coll")
        let (db_name, coll_name) = self.name.split_once(':').unwrap_or(("", &self.name));

        // Add to transaction log
        tx.add_operation(Operation::Update {
            database: db_name.to_string(),
            collection: coll_name.to_string(),
            key: key.to_string(),
            old_data,
            new_data: doc.to_value(),
        });

        Ok(doc)
    }

    /// Delete a document within a transaction
    pub fn delete_tx(
        &self,
        tx: &mut Transaction,
        _wal: &Arc<WalWriter>,
        key: &str,
    ) -> DbResult<()> {
        let doc = self.get(key)?;
        let old_data = doc.to_value();

        // Parse database and collection from self.name (format: "db:coll")
        let (db_name, coll_name) = self.name.split_once(':').unwrap_or(("", &self.name));

        tx.add_operation(Operation::Delete {
            database: db_name.to_string(),
            collection: coll_name.to_string(),
            key: key.to_string(),
            old_data,
        });
        Ok(())
    }

    /// Apply operations from a committed transaction with atomic document + index writes
    pub fn apply_transaction_operations(&self, operations: Vec<Operation>) -> DbResult<()> {
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut batch = WriteBatch::default();
        let mut change_events = Vec::new();

        // Build WriteBatch with documents AND index entries atomically
        for op in &operations {
            match op {
                Operation::Insert { key, data, .. } => {
                    // Serialize and add document to batch
                    let document = Document::with_key(&self.name, key.clone(), data.clone());
                    let doc_bytes = serialize_doc(&document)?;
                    batch.put_cf(cf, Self::doc_key(key), &doc_bytes);

                    // Compute and add index entries to batch
                    let (regular_entries, geo_entries) =
                        self.compute_index_entries_for_insert(key, data)?;
                    for (entry_key, entry_value) in regular_entries {
                        batch.put_cf(cf, entry_key, entry_value);
                    }
                    for (entry_key, entry_value) in geo_entries {
                        batch.put_cf(cf, entry_key, entry_value);
                    }

                    // Compute and add fulltext entries
                    let fulltext_entries = self.compute_fulltext_entries_for_insert(key, data);
                    for (entry_key, entry_value) in fulltext_entries {
                        batch.put_cf(cf, entry_key, entry_value);
                    }

                    // Compute and add TTL expiry entries
                    let ttl_expiry_entries = self.compute_ttl_expiry_entries_for_insert(key, data);
                    for (entry_key, _entry_value) in ttl_expiry_entries {
                        batch.put_cf(cf, entry_key, Vec::new());
                    }

                    // Queue change event for after commit
                    change_events.push(ChangeEvent {
                        type_: ChangeType::Insert,
                        key: key.clone(),
                        data: Some(data.clone()),
                        old_data: None,
                    });
                }
                Operation::Update {
                    key,
                    old_data,
                    new_data,
                    ..
                } => {
                    // Serialize and add document to batch
                    let document = Document::with_key(&self.name, key.clone(), new_data.clone());
                    let doc_bytes = serialize_doc(&document)?;
                    batch.put_cf(cf, Self::doc_key(key), &doc_bytes);

                    // Compute and apply index updates atomically
                    let (entries_to_add, keys_to_remove, geo_entries_to_add, geo_keys_to_remove) =
                        self.compute_index_entries_for_update(key, old_data, new_data)?;

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
                    let fulltext_keys_to_remove =
                        self.compute_fulltext_entries_for_delete(key, old_data);
                    for key_to_remove in fulltext_keys_to_remove {
                        batch.delete_cf(cf, key_to_remove);
                    }

                    let fulltext_entries_to_add =
                        self.compute_fulltext_entries_for_insert(key, new_data);
                    for (entry_key, entry_value) in fulltext_entries_to_add {
                        batch.put_cf(cf, entry_key, entry_value);
                    }

                    // Compute and apply TTL expiry updates
                    let (ttl_entries_to_add, ttl_keys_to_remove) =
                        self.compute_ttl_expiry_entries_for_update(key, old_data, new_data);
                    for key_to_remove in ttl_keys_to_remove {
                        batch.delete_cf(cf, key_to_remove);
                    }
                    for (entry_key, _entry_value) in ttl_entries_to_add {
                        batch.put_cf(cf, entry_key, Vec::new());
                    }

                    // Queue change event for after commit
                    change_events.push(ChangeEvent {
                        type_: ChangeType::Update,
                        key: key.clone(),
                        data: Some(new_data.clone()),
                        old_data: Some(old_data.clone()),
                    });
                }
                Operation::Delete { key, old_data, .. } => {
                    // Delete document from batch
                    batch.delete_cf(cf, Self::doc_key(key));

                    // Compute and remove index entries
                    let (regular_keys, geo_keys) =
                        self.compute_index_entries_for_delete(key, old_data)?;
                    for key_to_remove in regular_keys {
                        batch.delete_cf(cf, key_to_remove);
                    }
                    for key_to_remove in geo_keys {
                        batch.delete_cf(cf, key_to_remove);
                    }

                    // Compute and remove fulltext entries
                    let fulltext_keys = self.compute_fulltext_entries_for_delete(key, old_data);
                    for key_to_remove in fulltext_keys {
                        batch.delete_cf(cf, key_to_remove);
                    }

                    // Compute and remove TTL expiry entries
                    let ttl_keys = self.compute_ttl_expiry_entries_for_delete(key, old_data);
                    for key_to_remove in ttl_keys {
                        batch.delete_cf(cf, key_to_remove);
                    }

                    // Queue change event for after commit
                    change_events.push(ChangeEvent {
                        type_: ChangeType::Delete,
                        key: key.clone(),
                        data: None,
                        old_data: Some(old_data.clone()),
                    });
                }
                _ => {} // Other ops like PutBlobChunk handled separately
            }
        }

        // Commit batch atomically: all documents + indexes together
        db.write(batch).map_err(|e| {
            DbError::InternalError(format!("Failed to commit transaction batch: {}", e))
        })?;

        // Post-commit: update vector indexes, counts, and send change events
        for op in &operations {
            match op {
                Operation::Insert { key, data, .. } => {
                    self.update_vector_indexes_on_upsert(key, data);
                    self.increment_count();
                }
                Operation::Update { key, new_data, .. } => {
                    self.update_vector_indexes_on_delete(key);
                    self.update_vector_indexes_on_upsert(key, new_data);
                }
                Operation::Delete { .. } => {
                    self.decrement_count();
                }
                _ => {}
            }
        }

        // Send change events
        for event in change_events {
            let _ = self.change_sender.send(event);
        }

        Ok(())
    }
}
