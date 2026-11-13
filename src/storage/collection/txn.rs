use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::kv::KvBatch;
use crate::storage::serializer::serialize_doc;
use crate::transaction::lock_manager::LockManager;
use crate::transaction::wal::WalWriter;
use crate::transaction::{Operation, Transaction, TransactionId};
use serde_json::Value;
use std::collections::HashMap;
use uuid;

/// Which staged document holds each unique value, so a transaction cannot
/// stage two documents with the same one.
#[derive(Default)]
struct UniqueClaims {
    /// `<index>:<encoded values>` -> the document key that holds it.
    owner: HashMap<String, String>,
    /// The tokens each document key currently holds, to release on change.
    held: HashMap<String, Vec<String>>,
}

impl UniqueClaims {
    /// `key` now holds the unique values of `value` (and no longer those of its
    /// previous version). Fails if another staged document already holds one.
    fn claim(&mut self, coll: &Collection, key: &str, value: &Value) -> DbResult<()> {
        let tokens = coll.unique_tokens(value);
        for token in &tokens {
            if let Some(owner) = self.owner.get(token) {
                if owner != key {
                    return Err(DbError::InvalidDocument(format!(
                        "Unique constraint violated: '{}' is claimed by both '{}' and '{}' in this transaction",
                        token, owner, key
                    )));
                }
            }
        }
        self.release(key);
        for token in &tokens {
            self.owner.insert(token.clone(), key.to_string());
        }
        self.held.insert(key.to_string(), tokens);
        Ok(())
    }

    fn release(&mut self, key: &str) {
        if let Some(tokens) = self.held.remove(key) {
            for token in tokens {
                if self.owner.get(&token).is_some_and(|o| o == key) {
                    self.owner.remove(&token);
                }
            }
        }
    }
}

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

    /// The document `key` as transaction `tx` sees it: its own staged write
    /// (an insert, an update, or a delete, which reads as missing) when it
    /// made one, else the committed document under a shared lock.
    pub fn get_tx_view(
        &self,
        tx: &Transaction,
        lock_manager: &Arc<LockManager>,
        key: &str,
    ) -> DbResult<Option<Document>> {
        Self::ensure_tx_active(tx)?;
        let (db_name, coll_name) = self.parse_db_coll();
        if let Some(staged) = Self::tx_local_state(tx, &db_name, &coll_name, key) {
            return Ok(staged.map(|value| Self::txn_document(&self.name, key, &value)));
        }
        lock_manager.acquire_shared(tx.id, &db_name, &coll_name, key)?;
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
        lock_manager.acquire_exclusive_for(
            tx.id,
            &db_name,
            &coll_name,
            &key,
            !tx.isolation_level.tracks_reads(),
        )?;

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
        wal: &Arc<WalWriter>,
        lock_manager: &Arc<LockManager>,
        key: &str,
        data: Value,
    ) -> DbResult<Value> {
        self.update_tx_guarded(tx, wal, lock_manager, key, data, None)?
            .ok_or_else(|| DbError::InternalError("an unguarded update was refused".into()))
    }

    /// [`Self::update_tx`] for changes computed from a document read at
    /// `expected_rev` (a query's `UPDATE d WITH {n: d.n + 1}`). If the
    /// document the transaction would write over is no longer that revision
    /// — another transaction committed it since, or this one staged a write
    /// to it that the query, reading committed data, did not see — staging
    /// it would overwrite that write with a value computed from the older
    /// one (card #796). It is not staged, and `None` is returned: the
    /// transaction then holds the key's lock, and the caller either
    /// evaluates the row again on the current document (a query at
    /// `read_committed`, as a statement outside a transaction does) or
    /// refuses the statement ([`Self::stale_write`]).
    pub fn update_tx_if_unchanged(
        &self,
        tx: &mut Transaction,
        wal: &Arc<WalWriter>,
        lock_manager: &Arc<LockManager>,
        key: &str,
        data: Value,
        expected_rev: &str,
    ) -> DbResult<Option<Value>> {
        self.update_tx_guarded(tx, wal, lock_manager, key, data, Some(expected_rev))
    }

    /// The conflict of a write computed from a revision that is no longer
    /// the document's: roll back and retry.
    pub fn stale_write(key: &str, read_rev: &str) -> DbError {
        DbError::TransactionConflict(format!(
            "Write conflict: document '{}' changed since this query read it \
             (revision '{}'); roll back and retry",
            key, read_rev
        ))
    }

    fn update_tx_guarded(
        &self,
        tx: &mut Transaction,
        _wal: &Arc<WalWriter>,
        lock_manager: &Arc<LockManager>,
        key: &str,
        data: Value,
        expected_rev: Option<&str>,
    ) -> DbResult<Option<Value>> {
        Self::ensure_tx_active(tx)?;
        if self.collection_type.read().as_str() == "timeseries" {
            return Err(DbError::OperationNotSupported(
                "Update operations are not allowed on timeseries collections".to_string(),
            ));
        }

        let (db_name, coll_name) = self.parse_db_coll();
        lock_manager.acquire_exclusive_for(
            tx.id,
            &db_name,
            &coll_name,
            key,
            !tx.isolation_level.tracks_reads(),
        )?;

        // The staged state is a value throughout (`Document::to_value`'s
        // shape), read straight from the stored bytes: building a `Document`
        // and converting it back and forth (timestamps parsed and formatted,
        // every field cloned) cost more than the work itself.
        let old_data = match Self::tx_local_state(tx, &db_name, &coll_name, key) {
            Some(Some(value)) => value,
            Some(None) => return Err(DbError::DocumentNotFound(key.to_string())),
            None => self.get_value(key)?,
        };
        if let Some(rev) = expected_rev {
            let now = old_data
                .get("_rev")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if now != rev {
                return Ok(None);
            }
        }
        let new_data = Self::updated_value(&self.name, key, &old_data, data);

        if self.collection_type.read().as_str() == "edge" {
            self.validate_edge_document(&new_data)?;
        }

        tx.add_operation(Operation::Update {
            database: db_name,
            collection: coll_name,
            key: key.to_string(),
            old_data,
            new_data: new_data.clone(),
        });

        Ok(Some(new_data))
    }

    /// `Document::update` applied to a staged value: the change's fields
    /// over the old ones (the system attributes excepted), a new revision
    /// and update time. A change that is not an object replaces the
    /// fields, as `Document::update` does, through a `Document`.
    pub(crate) fn updated_value(collection: &str, key: &str, old: &Value, changes: Value) -> Value {
        match (old, changes) {
            (Value::Object(old_fields), Value::Object(changes)) => {
                let mut fields = old_fields.clone();
                for (name, field) in changes {
                    if !crate::storage::document::is_system_field(&name) {
                        fields.insert(name, field);
                    }
                }
                fields.insert("_rev".to_string(), Value::String(Document::generate_rev()));
                fields.insert(
                    "_updated_at".to_string(),
                    Value::String(chrono::Utc::now().to_rfc3339()),
                );
                Value::Object(fields)
            }
            (_, changes) => {
                let mut doc = Self::txn_document(collection, key, old);
                doc.update(changes);
                doc.into_value()
            }
        }
    }

    /// A staged document's value and its stored bytes, written from the
    /// staged value itself when it carries the system attributes the
    /// staging gives it, else through a `Document` as before.
    fn txn_stored(&self, key: &str, staged: Value) -> DbResult<(Value, Vec<u8>)> {
        if let Some(bytes) =
            crate::storage::serializer::serialize_api_value(&self.name, key, &staged)?
        {
            return Ok((staged, bytes));
        }
        let doc = Self::txn_document(&self.name, key, &staged);
        let bytes = serialize_doc(&doc)?;
        Ok((doc.to_value(), bytes))
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
        lock_manager.acquire_exclusive_for(
            tx.id,
            &db_name,
            &coll_name,
            key,
            !tx.isolation_level.tracks_reads(),
        )?;

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
        let mut batch = KvBatch::default();
        let staged = self.stage_transaction_operations(operations, &mut batch)?;
        self.db.write(batch).map_err(|e| {
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
        self.lock_for_transaction_reading(operations, &[])
    }

    /// [`Self::lock_for_transaction`], also taking the write locks of the
    /// keys the transaction read (`read_keys`): its commit checks them, and
    /// they must not change between that check and its write.
    pub(crate) fn lock_for_transaction_reading(
        &self,
        operations: &[Operation],
        read_keys: &[(String, Option<Box<str>>)],
    ) -> Vec<super::locks::WriteGuard> {
        let key_guard = self.lock_keys(
            operations
                .iter()
                .map(|op| op.key())
                .chain(read_keys.iter().map(|(key, _)| key.as_str())),
        );
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
        operations: Vec<Operation>,
        batch: &mut KvBatch,
    ) -> DbResult<StagedTransaction> {
        let cf = self.ks.handle(&self.db).ok_or_else(|| {
            DbError::CollectionNotFound(format!(
                "{} (column family dropped before commit)",
                self.name
            ))
        })?;
        let versioned = self.is_versioned();

        // State of each key as of the operations staged so far, so a second
        // operation on the same key is checked against the first one's result
        // rather than against disk: the event that holds its value (`None`
        // once deleted). The operations are consumed, and each document moves
        // from its operation into its event without a copy; a transfer's
        // commit used to clone every document it wrote about ten times.
        let mut pending: HashMap<String, Option<usize>> = HashMap::new();
        // Unique values claimed by the operations staged so far. Each write is
        // checked against disk, which does not yet hold this transaction's
        // earlier writes, so two inserts of one value both passed.
        let mut unique_claims = UniqueClaims::default();
        let mut staged = StagedTransaction::default();

        for op in operations {
            match op {
                Operation::Insert { key, data, .. } => {
                    let exists = match pending.get(&key) {
                        Some(state) => state.is_some(),
                        None => self.stored_rev(&key)?.is_some(),
                    };
                    if exists {
                        return Err(DbError::TransactionConflict(format!(
                            "Document '{}' already exists in {}",
                            key, self.name
                        )));
                    }

                    let (value, doc_bytes) = self.txn_stored(&key, data)?;
                    self.check_unique_constraints(&key, &value)?;
                    unique_claims.claim(self, &key, &value)?;
                    batch.put_ks(&cf, Self::doc_key(&key), &doc_bytes);
                    if versioned {
                        self.append_version_to_batch(batch, &cf, &key, Some(&value));
                    }

                    let (regular_entries, geo_entries) =
                        self.compute_index_entries_for_insert(&key, &value)?;
                    for (entry_key, entry_value) in regular_entries.into_iter().chain(geo_entries) {
                        batch.put_ks(&cf, entry_key, entry_value);
                    }
                    for (entry_key, entry_value) in
                        self.compute_fulltext_entries_for_insert(&key, &value)
                    {
                        batch.put_ks(&cf, entry_key, entry_value);
                    }
                    for (entry_key, _) in self.compute_ttl_expiry_entries_for_insert(&key, &value) {
                        batch.put_ks(&cf, entry_key, Vec::new());
                    }

                    pending.insert(key.clone(), Some(staged.events.len()));
                    staged.events.push(ChangeEvent {
                        type_: ChangeType::Insert,
                        key,
                        data: Some(value),
                        old_data: None,
                    });
                }
                Operation::Update {
                    key,
                    old_data,
                    new_data,
                    ..
                } => {
                    let current = self.txn_expect_unchanged(&staged, &pending, &key, old_data)?;

                    let (value, doc_bytes) = self.txn_stored(&key, new_data)?;
                    self.check_unique_constraints(&key, &value)?;
                    unique_claims.claim(self, &key, &value)?;
                    batch.put_ks(&cf, Self::doc_key(&key), &doc_bytes);
                    if versioned {
                        self.append_version_to_batch(batch, &cf, &key, Some(&value));
                    }

                    let (entries_to_add, keys_to_remove, geo_entries_to_add, geo_keys_to_remove) =
                        self.compute_index_entries_for_update(&key, &current, &value)?;
                    for k in keys_to_remove.into_iter().chain(geo_keys_to_remove) {
                        batch.delete_ks(&cf, k);
                    }
                    for (entry_key, entry_value) in
                        entries_to_add.into_iter().chain(geo_entries_to_add)
                    {
                        batch.put_ks(&cf, entry_key, entry_value);
                    }

                    for k in self.compute_fulltext_entries_for_delete(&key, &current) {
                        batch.delete_ks(&cf, k);
                    }
                    for (entry_key, entry_value) in
                        self.compute_fulltext_entries_for_insert(&key, &value)
                    {
                        batch.put_ks(&cf, entry_key, entry_value);
                    }

                    let (ttl_entries_to_add, ttl_keys_to_remove) =
                        self.compute_ttl_expiry_entries_for_update(&key, &current, &value);
                    for k in ttl_keys_to_remove {
                        batch.delete_ks(&cf, k);
                    }
                    for (entry_key, _) in ttl_entries_to_add {
                        batch.put_ks(&cf, entry_key, Vec::new());
                    }

                    pending.insert(key.clone(), Some(staged.events.len()));
                    staged.events.push(ChangeEvent {
                        type_: ChangeType::Update,
                        key,
                        data: Some(value),
                        old_data: Some(current),
                    });
                }
                Operation::Delete { key, old_data, .. } => {
                    let current = self.txn_expect_unchanged(&staged, &pending, &key, old_data)?;
                    unique_claims.release(&key);

                    batch.delete_ks(&cf, Self::doc_key(&key));
                    if versioned {
                        self.append_version_to_batch(batch, &cf, &key, None);
                    }

                    let (regular_keys, geo_keys) =
                        self.compute_index_entries_for_delete(&key, &current)?;
                    for k in regular_keys.into_iter().chain(geo_keys) {
                        batch.delete_ks(&cf, k);
                    }
                    for k in self.compute_fulltext_entries_for_delete(&key, &current) {
                        batch.delete_ks(&cf, k);
                    }
                    for k in self.compute_ttl_expiry_entries_for_delete(&key, &current) {
                        batch.delete_ks(&cf, k);
                    }

                    pending.insert(key.clone(), None);
                    staged.events.push(ChangeEvent {
                        type_: ChangeType::Delete,
                        key,
                        data: None,
                        old_data: Some(current),
                    });
                }
                _ => {} // Other ops like PutBlobChunk handled separately
            }
        }

        Ok(staged)
    }

    /// In-memory side effects of a staged transaction, to run only once its
    /// batch has been written: vector indexes, counts, change events.
    pub(crate) fn finish_transaction_operations(&self, staged: StagedTransaction) {
        // Only where the collection has a vector index, by its cached
        // definitions (one defined but not loaded yet still has to be
        // updated). Walking the usually empty map of loaded indexes twice
        // per document was about 1% of a money transfer.
        let vectors = self.index_meta().is_none_or(|meta| !meta.vector.is_empty());
        for event in &staged.events {
            match event.type_ {
                ChangeType::Insert => {
                    if let (true, Some(value)) = (vectors, &event.data) {
                        self.update_vector_indexes_on_upsert(&event.key, value);
                    }
                    self.increment_count();
                }
                ChangeType::Update => {
                    if let (true, Some(value)) = (vectors, &event.data) {
                        self.update_vector_indexes_on_delete(&event.key);
                        self.update_vector_indexes_on_upsert(&event.key, value);
                    }
                }
                ChangeType::Delete => {
                    if vectors {
                        self.update_vector_indexes_on_delete(&event.key);
                    }
                    self.decrement_count();
                }
                ChangeType::Truncate => {}
            }
        }

        // Cached results dropped once for the whole commit.
        if !staged.events.is_empty() {
            self.publish_changes(|| staged.events);
        }
    }

    /// Current value of `key`, provided it is still the revision the
    /// operation was based on.
    ///
    /// Against storage only the revision is read, from the stored header:
    /// a revision is never written twice, so when it is still the one the
    /// operation was based on, the stored document is `based_on` itself,
    /// which is handed back. A full read and decode of every document a
    /// commit writes was ~4% of a money transfer's CPU.
    fn txn_expect_unchanged(
        &self,
        staged: &StagedTransaction,
        pending: &HashMap<String, Option<usize>>,
        key: &str,
        based_on: Value,
    ) -> DbResult<Value> {
        let deleted = || {
            DbError::TransactionConflict(format!(
                "Document '{}' in {} was deleted after the transaction read it",
                key, self.name
            ))
        };
        let modified = || {
            DbError::TransactionConflict(format!(
                "Document '{}' in {} was modified after the transaction read it",
                key, self.name
            ))
        };
        if let Some(state) = pending.get(key) {
            let current = state
                .and_then(|event| staged.events[event].data.as_ref())
                .ok_or_else(deleted)?;
            if current.get("_rev") != based_on.get("_rev") {
                return Err(modified());
            }
            return Ok(based_on);
        }
        let stored = self.stored_rev(key)?.ok_or_else(deleted)?;
        if based_on.get("_rev").and_then(Value::as_str) != Some(stored.as_str()) {
            return Err(modified());
        }
        Ok(based_on)
    }

    /// The revision `key` is stored at, from the stored header without
    /// decoding the document (a document in an older format is decoded);
    /// `None` when it does not exist.
    fn stored_rev(&self, key: &str) -> DbResult<Option<String>> {
        let cf = self.live_cf()?;
        let Some(bytes) = self
            .db
            .get_pinned_ks(&cf, Self::doc_key(key))
            .map_err(|e| DbError::InternalError(format!("Failed to get document: {}", e)))?
        else {
            return Ok(None);
        };
        if let Some(rev) = crate::storage::serializer::StoredDocRef::parse(&bytes)
            .and_then(|doc| std::str::from_utf8(doc.rev).ok().map(str::to_owned))
        {
            return Ok(Some(rev));
        }
        Ok(Some(
            crate::storage::serializer::deserialize_doc(&bytes)?.rev,
        ))
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

    /// What `tx` has staged for `key`, as a document: `Some(None)` when it
    /// deleted it, `None` when it has not written it.
    pub(crate) fn tx_staged_document(
        &self,
        tx: &Transaction,
        key: &str,
    ) -> Option<Option<Document>> {
        let (db_name, coll_name) = self.name.split_once(':').unwrap_or(("", &self.name));
        Self::tx_local_state(tx, db_name, coll_name, key)
            .map(|staged| staged.map(|value| Self::txn_document(&self.name, key, &value)))
    }

    /// Check what a transaction read from this collection against what is
    /// stored now. The caller holds the write locks of every key in
    /// `points`, and the collection's commit gate exclusively when the
    /// transaction read it by range, so nothing checked here can change
    /// before the commit's batch is written.
    pub(crate) fn validate_reads(
        &self,
        tracker: &crate::storage::keyspace::ReadTracker,
        points: &[(String, Option<Box<str>>)],
    ) -> DbResult<()> {
        if let Some(conflict) = tracker.conflict() {
            return Err(DbError::TransactionConflict(conflict));
        }
        if tracker.is_ranged() && (tracker.state().is_dead() || tracker.generation_moved()) {
            return Err(DbError::TransactionConflict(format!(
                "Read conflict: collection '{}' was written since this transaction read \
                 it other than by document key; roll back and retry",
                self.parse_db_coll().1
            )));
        }
        for (key, read) in points {
            if self.stored_rev(key)?.as_deref() != read.as_deref() {
                return Err(DbError::TransactionConflict(format!(
                    "Read conflict: document '{}' in '{}' changed since this transaction \
                     read it; roll back and retry",
                    key,
                    self.parse_db_coll().1
                )));
            }
        }
        Ok(())
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

/// Writes staged into a commit batch, awaiting their post-write effects:
/// one event per write, which also says what to do to the in-memory state.
#[derive(Default)]
pub(crate) struct StagedTransaction {
    events: Vec<ChangeEvent>,
}

#[cfg(test)]
mod tests {
    use crate::storage::index::VectorIndexConfig;
    use crate::storage::StorageEngine;
    use crate::transaction::Operation;
    use serde_json::json;

    fn insert(key: &str, data: serde_json::Value) -> Operation {
        Operation::Insert {
            database: String::new(),
            collection: "vec".to_string(),
            key: key.to_string(),
            data,
        }
    }

    /// A commit updates the collection's vector index, also when the index
    /// is defined but not loaded in memory (after a restart, before its
    /// first search): the document must not wait for a rebuild to be found.
    #[test]
    fn a_commit_updates_a_vector_index_that_is_not_loaded() {
        let dir = tempfile::TempDir::new().unwrap();
        let engine = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
        engine.create_collection("vec".to_string(), None).unwrap();
        let coll = engine.get_collection("vec").unwrap();
        let config: VectorIndexConfig = serde_json::from_value(json!({
            "name": "emb", "field": "embedding", "dimension": 2
        }))
        .unwrap();
        coll.create_vector_index(config).unwrap();

        let found = |query: [f32; 2]| -> Vec<String> {
            coll.vector_search("emb", &query, 1, None)
                .unwrap()
                .into_iter()
                .map(|hit| hit.doc_key)
                .collect()
        };

        coll.apply_transaction_operations(vec![insert(
            "near",
            json!({"_key": "near", "embedding": [1.0, 0.0]}),
        )])
        .unwrap();
        assert_eq!(found([1.0, 0.0]), vec!["near"]);

        // As after a restart: the index is defined, nothing is in memory.
        coll.persist_vector_indexes().unwrap();
        coll.vector_indexes.clear();
        coll.apply_transaction_operations(vec![insert(
            "far",
            json!({"_key": "far", "embedding": [0.0, 1.0]}),
        )])
        .unwrap();
        assert_eq!(found([0.0, 1.0]), vec!["far"]);
    }
}
