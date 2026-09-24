use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::serializer::serialize_doc;
use crate::transaction::lock_manager::LockManager;
use crate::transaction::wal::WalWriter;
use crate::transaction::{Operation, Transaction, TransactionId};
use rust_rocksdb::WriteBatch;
use serde_json::Value;
use std::collections::HashMap;
use uuid;

impl Collection {
    // ==================== Transactional Operations ====================

    /// A transaction that is committing (or finished) takes no more
    /// operations: commit snapshots the operation list when it starts, so
    /// one added afterwards would be silently dropped.
    fn ensure_tx_active(tx: &Transaction) -> DbResult<()> {
        if tx.is_active() {
            Ok(())
        } else {
            Err(DbError::TransactionConflict(format!(
                "Transaction {} is not active (state: {:?})",
                tx.id, tx.state
            )))
        }
    }

    fn parse_db_coll(&self) -> (String, String) {
        let (a, b) = self.name.split_once(':').unwrap_or(("", &self.name));
        (a.to_string(), b.to_string())
    }

    pub fn get_tx(
        &self,
        tx_id: TransactionId,
        lock_manager: &Arc<LockManager>,
        key: &str,
    ) -> DbResult<Option<Document>> {
        let (db_name, coll_name) = self.parse_db_coll();

        lock_manager.acquire_shared(tx_id, &db_name, &coll_name, key)?;

        match self.get(key) {
            Ok(doc) => Ok(Some(doc)),
            Err(DbError::DocumentNotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn insert_tx(
        &self,
        tx: &mut Transaction,
        _wal: &Arc<WalWriter>,
        lock_manager: &Arc<LockManager>,
        mut data: Value,
    ) -> DbResult<Document> {
        Self::ensure_tx_active(tx)?;
        if self.collection_type.read().as_str() == "edge" {
            self.validate_edge_document(&data)?;
        }

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

        let (db_name, coll_name) = self.parse_db_coll();
        lock_manager.acquire_exclusive(tx.id, &db_name, &coll_name, &key)?;

        let doc = Document::with_key(&self.name, key.clone(), data);
        self.check_unique_constraints(&key, &doc.to_value())?;

        tx.add_operation(Operation::Insert {
            database: db_name,
            collection: coll_name,
            key: key.clone(),
            data: doc.to_value(),
        });

        Ok(doc)
    }

    pub fn update_tx(
        &self,
        tx: &mut Transaction,
        _wal: &Arc<WalWriter>,
        lock_manager: &Arc<LockManager>,
        key: &str,
        data: Value,
    ) -> DbResult<Document> {
        Self::ensure_tx_active(tx)?;
        if self.collection_type.read().as_str() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Update operations are not allowed on timeseries collections".to_string(),
            ));
        }

        let (db_name, coll_name) = self.parse_db_coll();
        lock_manager.acquire_exclusive(tx.id, &db_name, &coll_name, key)?;

        let mut doc = self.tx_base_document(tx, &db_name, &coll_name, key)?;
        let old_data = doc.to_value();

        doc.update(data);

        if self.collection_type.read().as_str() == "edge" {
            self.validate_edge_document(&doc.to_value())?;
        }

        tx.add_operation(Operation::Update {
            database: db_name,
            collection: coll_name,
            key: key.to_string(),
            old_data,
            new_data: doc.to_value(),
        });

        Ok(doc)
    }

    pub fn delete_tx(
        &self,
        tx: &mut Transaction,
        _wal: &Arc<WalWriter>,
        lock_manager: &Arc<LockManager>,
        key: &str,
    ) -> DbResult<()> {
        Self::ensure_tx_active(tx)?;
        let (db_name, coll_name) = self.parse_db_coll();
        lock_manager.acquire_exclusive(tx.id, &db_name, &coll_name, key)?;

        let doc = self.tx_base_document(tx, &db_name, &coll_name, key)?;
        let old_data = doc.to_value();

        tx.add_operation(Operation::Delete {
            database: db_name,
            collection: coll_name,
            key: key.to_string(),
            old_data,
        });
        Ok(())
    }

    /// Apply operations from a committed transaction with atomic document +
    /// index writes. Single-collection convenience over
    /// [`Self::stage_transaction_operations`]; a multi-collection commit goes
    /// through `StorageEngine::commit_transaction`, which stages every
    /// collection into one batch.
    pub fn apply_transaction_operations(&self, operations: Vec<Operation>) -> DbResult<()> {
        let _guards = self.lock_for_transaction(&operations);
        let mut batch = WriteBatch::default();
        let staged = self.stage_transaction_operations(&operations, &mut batch)?;
        self.db.write(&batch).map_err(|e| {
            DbError::InternalError(format!("Failed to commit transaction batch: {}", e))
        })?;
        self.finish_transaction_operations(staged);
        Ok(())
    }

    /// Take the per-key and unique-value write stripes (audit D4) covering
    /// `operations`, so no single-document writer can slip in between the
    /// conflict checks in [`Self::stage_transaction_operations`] and the
    /// batch write. Hold the guards until the batch is written. A commit
    /// spanning collections must lock them in a consistent (sorted) order.
    pub(crate) fn lock_for_transaction(
        &self,
        operations: &[Operation],
    ) -> Vec<super::locks::WriteGuard> {
        let key_guard = self.lock_keys(operations.iter().map(|op| op.key()));
        let tokens: Vec<String> = operations
            .iter()
            .filter_map(|op| match op {
                Operation::Insert { data, .. } => Some(data),
                Operation::Update { new_data, .. } => Some(new_data),
                _ => None,
            })
            .flat_map(|value| self.unique_tokens(value))
            .collect();
        let unique_guard = self.lock_unique_tokens(tokens.iter());
        vec![key_guard, unique_guard]
    }

    /// Check `operations` against the collection's *current* state and append
    /// their document, index, fulltext, TTL and version writes to `batch`.
    /// Writes nothing itself: on `Err` the caller discards the batch and the
    /// database is untouched.
    ///
    /// Audit D2: an `Update`/`Delete` used to replay the `old_data` captured
    /// when the operation was recorded — up to the transaction timeout
    /// earlier — both for its index diff and implicitly as the base of the
    /// write, silently overwriting anything written in between by a
    /// non-transactional writer (which takes no lock-manager lock). Each
    /// operation is now checked against the document as it stands, read
    /// while the transaction still holds its exclusive locks: a `_rev` that
    /// moved, a document that vanished, or an insert over an existing key is
    /// a `TransactionConflict`, and index diffs are computed from the current
    /// value rather than the stale one.
    pub(crate) fn stage_transaction_operations(
        &self,
        operations: &[Operation],
        batch: &mut WriteBatch,
    ) -> DbResult<StagedTransaction> {
        let cf = self.db.cf_handle(&self.name).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped before commit)",
                self.name
            ))
        })?;
        let versioned = self.is_versioned();

        // State of each key as of the operations staged so far, so a second
        // operation on the same key is checked against the first one's result
        // rather than against disk.
        let mut pending: HashMap<String, Option<Value>> = HashMap::new();
        let mut staged = StagedTransaction::default();

        for op in operations {
            match op {
                Operation::Insert { key, data, .. } => {
                    if self.txn_current(&pending, key)?.is_some() {
                        return Err(DbError::TransactionConflict(format!(
                            "Document '{}' already exists in {}",
                            key, self.name
                        )));
                    }

                    let doc = Self::txn_document(&self.name, key, data);
                    let value = doc.to_value();
                    self.check_unique_constraints(key, &value)?;
                    let doc_bytes = serialize_doc(&doc)?;
                    batch.put_cf(&cf, Self::doc_key(key), &doc_bytes);
                    if versioned {
                        self.append_version_to_batch(batch, &cf, key, Some(&value));
                    }

                    let (regular_entries, geo_entries) =
                        self.compute_index_entries_for_insert(key, &value)?;
                    for (entry_key, entry_value) in regular_entries.into_iter().chain(geo_entries) {
                        batch.put_cf(&cf, entry_key, entry_value);
                    }
                    for (entry_key, entry_value) in
                        self.compute_fulltext_entries_for_insert(key, &value)
                    {
                        batch.put_cf(&cf, entry_key, entry_value);
                    }
                    for (entry_key, _) in self.compute_ttl_expiry_entries_for_insert(key, &value) {
                        batch.put_cf(&cf, entry_key, Vec::new());
                    }

                    staged.events.push(ChangeEvent {
                        type_: ChangeType::Insert,
                        key: key.clone(),
                        data: Some(value.clone()),
                        old_data: None,
                    });
                    staged
                        .effects
                        .push(StagedEffect::Inserted(key.clone(), value.clone()));
                    pending.insert(key.clone(), Some(value));
                }
                Operation::Update {
                    key,
                    old_data,
                    new_data,
                    ..
                } => {
                    let current = self.txn_expect_unchanged(&pending, key, old_data)?;

                    let doc = Self::txn_document(&self.name, key, new_data);
                    let value = doc.to_value();
                    self.check_unique_constraints(key, &value)?;
                    let doc_bytes = serialize_doc(&doc)?;
                    batch.put_cf(&cf, Self::doc_key(key), &doc_bytes);
                    if versioned {
                        self.append_version_to_batch(batch, &cf, key, Some(&value));
                    }

                    let (entries_to_add, keys_to_remove, geo_entries_to_add, geo_keys_to_remove) =
                        self.compute_index_entries_for_update(key, &current, &value)?;
                    for k in keys_to_remove.into_iter().chain(geo_keys_to_remove) {
                        batch.delete_cf(&cf, k);
                    }
                    for (entry_key, entry_value) in
                        entries_to_add.into_iter().chain(geo_entries_to_add)
                    {
                        batch.put_cf(&cf, entry_key, entry_value);
                    }

                    for k in self.compute_fulltext_entries_for_delete(key, &current) {
                        batch.delete_cf(&cf, k);
                    }
                    for (entry_key, entry_value) in
                        self.compute_fulltext_entries_for_insert(key, &value)
                    {
                        batch.put_cf(&cf, entry_key, entry_value);
                    }

                    let (ttl_entries_to_add, ttl_keys_to_remove) =
                        self.compute_ttl_expiry_entries_for_update(key, &current, &value);
                    for k in ttl_keys_to_remove {
                        batch.delete_cf(&cf, k);
                    }
                    for (entry_key, _) in ttl_entries_to_add {
                        batch.put_cf(&cf, entry_key, Vec::new());
                    }

                    staged.events.push(ChangeEvent {
                        type_: ChangeType::Update,
                        key: key.clone(),
                        data: Some(value.clone()),
                        old_data: Some(current),
                    });
                    staged
                        .effects
                        .push(StagedEffect::Updated(key.clone(), value.clone()));
                    pending.insert(key.clone(), Some(value));
                }
                Operation::Delete { key, old_data, .. } => {
                    let current = self.txn_expect_unchanged(&pending, key, old_data)?;

                    batch.delete_cf(&cf, Self::doc_key(key));
                    if versioned {
                        self.append_version_to_batch(batch, &cf, key, None);
                    }

                    let (regular_keys, geo_keys) =
                        self.compute_index_entries_for_delete(key, &current)?;
                    for k in regular_keys.into_iter().chain(geo_keys) {
                        batch.delete_cf(&cf, k);
                    }
                    for k in self.compute_fulltext_entries_for_delete(key, &current) {
                        batch.delete_cf(&cf, k);
                    }
                    for k in self.compute_ttl_expiry_entries_for_delete(key, &current) {
                        batch.delete_cf(&cf, k);
                    }

                    staged.events.push(ChangeEvent {
                        type_: ChangeType::Delete,
                        key: key.clone(),
                        data: None,
                        old_data: Some(current),
                    });
                    staged.effects.push(StagedEffect::Deleted(key.clone()));
                    pending.insert(key.clone(), None);
                }
                _ => {} // Other ops like PutBlobChunk handled separately
            }
        }

        Ok(staged)
    }

    /// In-memory side effects of a staged transaction, to run only once its
    /// batch has been written: vector indexes, counts, change events.
    pub(crate) fn finish_transaction_operations(&self, staged: StagedTransaction) {
        for effect in &staged.effects {
            match effect {
                StagedEffect::Inserted(key, value) => {
                    self.update_vector_indexes_on_upsert(key, value);
                    self.increment_count();
                }
                StagedEffect::Updated(key, value) => {
                    self.update_vector_indexes_on_delete(key);
                    self.update_vector_indexes_on_upsert(key, value);
                }
                StagedEffect::Deleted(key) => {
                    self.update_vector_indexes_on_delete(key);
                    self.decrement_count();
                }
            }
        }

        for event in staged.events {
            let _ = self.emit_change(event);
        }
    }

    /// The document as the transaction would see it: the result of an earlier
    /// staged operation on the same key, else what is on disk.
    fn txn_current(
        &self,
        pending: &HashMap<String, Option<Value>>,
        key: &str,
    ) -> DbResult<Option<Value>> {
        if let Some(state) = pending.get(key) {
            return Ok(state.clone());
        }
        match self.get(key) {
            Ok(doc) => Ok(Some(doc.into_value())),
            Err(DbError::DocumentNotFound(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Current value of `key`, provided it is still the revision the
    /// operation was based on.
    fn txn_expect_unchanged(
        &self,
        pending: &HashMap<String, Option<Value>>,
        key: &str,
        based_on: &Value,
    ) -> DbResult<Value> {
        let current = self.txn_current(pending, key)?.ok_or_else(|| {
            DbError::TransactionConflict(format!(
                "Document '{}' in {} was deleted after the transaction read it",
                key, self.name
            ))
        })?;
        if current.get("_rev") != based_on.get("_rev") {
            return Err(DbError::TransactionConflict(format!(
                "Document '{}' in {} was modified after the transaction read it",
                key, self.name
            )));
        }
        Ok(current)
    }

    /// Rebuild the stored `Document` from a transaction's value, keeping the
    /// `_rev` and timestamps the client was handed. `Document::with_key`
    /// would mint a fresh `_rev` and `_created_at`, so the committed
    /// revision would not match the one returned by `update_tx`.
    fn txn_document(collection: &str, key: &str, value: &Value) -> Document {
        let mut doc = Document::with_key(collection, key.to_string(), value.clone());
        if let Some(rev) = value.get("_rev").and_then(|v| v.as_str()) {
            doc.rev = rev.to_string();
        }
        let parse = |field: &str| {
            value
                .get(field)
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|t| t.with_timezone(&chrono::Utc))
        };
        if let Some(t) = parse("_created_at") {
            doc.created_at = t;
        }
        if let Some(t) = parse("_updated_at") {
            doc.updated_at = t;
        }
        doc
    }

    /// Latest state of `key` inside `tx`, if the transaction has touched it:
    /// `Some(Some(v))` written, `Some(None)` deleted, `None` untouched.
    fn tx_local_state(
        tx: &Transaction,
        db_name: &str,
        coll_name: &str,
        key: &str,
    ) -> Option<Option<Value>> {
        tx.operations.iter().rev().find_map(|op| {
            if op.database() != db_name || op.collection() != coll_name || op.key() != key {
                return None;
            }
            match op {
                Operation::Insert { data, .. } => Some(Some(data.clone())),
                Operation::Update { new_data, .. } => Some(Some(new_data.clone())),
                Operation::Delete { .. } => Some(None),
                _ => None,
            }
        })
    }

    /// The document an update/delete inside `tx` is based on: the
    /// transaction's own latest write to it, else the stored one. Without
    /// this, a second update in the same transaction was based on disk and
    /// dropped the first one's changes.
    fn tx_base_document(
        &self,
        tx: &Transaction,
        db_name: &str,
        coll_name: &str,
        key: &str,
    ) -> DbResult<Document> {
        match Self::tx_local_state(tx, db_name, coll_name, key) {
            Some(Some(value)) => Ok(Self::txn_document(&self.name, key, &value)),
            Some(None) => Err(DbError::DocumentNotFound(key.to_string())),
            None => self.get(key),
        }
    }
}

/// Writes staged into a commit batch, awaiting their post-write effects.
#[derive(Default)]
pub(crate) struct StagedTransaction {
    events: Vec<ChangeEvent>,
    effects: Vec<StagedEffect>,
}

enum StagedEffect {
    Inserted(String, Value),
    Updated(String, Value),
    Deleted(String),
}
