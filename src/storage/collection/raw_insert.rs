//! Inserts, bulk inserts and merge updates from the client's MessagePack.
//!
//! The same writes as [`Collection::insert_value`], [`Collection::insert_batch`]
//! and [`Collection::merge_update`], for documents that arrive as MessagePack
//! (`storage::raw_write`): the stored bytes are built from the client's bytes,
//! only the fields an index reads are decoded, and the reply is written from
//! the stored bytes (`raw_rows`). The keys, revisions, timestamps, stored
//! bytes, index entries, change events and errors are the decoded path's.
//!
//! Each returns `None`, before writing anything, where it does not apply —
//! a collection that is not a plain document collection, one with a schema,
//! versioning or a vector index, a document it cannot read this way — and
//! the caller then takes the decoded path.

use super::crud::key_exists_error;
use super::index_meta::IndexMetaSnapshot;
use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::kv::KvBatch;
use crate::storage::raw_rows::{write_row, RawShape};
use crate::storage::raw_write::{
    indexed_view, merge_fields, parse_stored_timestamp, write_api_value, CanonicalDoc, Header,
    KeyField,
};
use crate::storage::serializer::{
    deserialize_doc_as_value, serialize_doc_raw, stored_timestamp, write_msgpack, StoredDocRef,
};
use chrono::Utc;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// A UUID as its `to_string` writes it, without the formatting machinery.
fn uuid_text(id: uuid::Uuid) -> String {
    let mut buf = [0u8; uuid::fmt::Hyphenated::LENGTH];
    id.hyphenated().encode_lower(&mut buf).to_owned()
}

/// A new document's system fields, made as `Document::with_key` makes them.
struct NewHeader {
    key: String,
    id: String,
    rev: String,
    now: chrono::DateTime<Utc>,
    /// `now` as it is stored (created and updated alike).
    now_text: String,
}

impl NewHeader {
    fn new(collection: &str, key: String) -> Self {
        let mut id = String::with_capacity(collection.len() + 1 + key.len());
        id.push_str(collection);
        id.push('/');
        id.push_str(&key);
        let now = Utc::now();
        NewHeader {
            id,
            key,
            rev: uuid_text(uuid::Uuid::new_v4()),
            now_text: stored_timestamp(&now),
            now,
        }
    }

    fn view(&self) -> Header<'_> {
        Header {
            key: &self.key,
            id: &self.id,
            rev: &self.rev,
            created_at: self.now,
            updated_at: self.now,
        }
    }

    fn stored(&self, data: &[u8]) -> DbResult<Vec<u8>> {
        let t = &self.now_text;
        serialize_doc_raw(&self.key, &self.id, &self.rev, t, t, data)
    }

    /// The document's API value in MessagePack.
    fn reply(&self, stored: &[u8], data: &[u8]) -> DbResult<Vec<u8>> {
        let t = &self.now_text;
        api_bytes(stored, |out| {
            write_api_value(out, &self.key, &self.id, &self.rev, t, t, data)
        })
    }
}

/// A document's API value (`Document::to_value`) in MessagePack: written
/// from its parts by `write`, or failing that from its stored bytes.
fn api_bytes(stored: &[u8], write: impl FnOnce(&mut Vec<u8>) -> Option<()>) -> DbResult<Vec<u8>> {
    let mut out = Vec::with_capacity(stored.len() + 96);
    if write(&mut out).is_some() {
        return Ok(out);
    }
    out.clear();
    if write_row(stored, &RawShape::Whole, &mut out).is_some() {
        return Ok(out);
    }
    out.clear();
    write_msgpack(&mut out, &deserialize_doc_as_value(stored)?);
    Ok(out)
}

/// The value a change event carries for stored bytes.
fn event_value(stored: &[u8]) -> Value {
    deserialize_doc_as_value(stored).unwrap_or(Value::Null)
}

/// The key a document gets: the one given, else its `_key`, else a new one
/// (`take_key`). `None` for a `_key` that is not a string, which the
/// decoded path reports.
fn document_key(given: Option<&str>, doc: &CanonicalDoc) -> Option<String> {
    Some(match (given, &doc.key) {
        (Some(key), _) => key.to_owned(),
        (None, KeyField::Str(key)) => key.clone(),
        (None, KeyField::Absent) => {
            uuid_text(uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)))
        }
        (None, KeyField::Other) => return None,
    })
}

impl Collection {
    /// The index definitions, when writes to this collection can be built
    /// from the client's bytes: a document collection (an edge is
    /// validated), with no schema (it validates a decoded document), no
    /// versioning (its record holds one) and no vector index (it reads and
    /// writes beside the document).
    pub(crate) fn raw_write_ready(&self) -> Option<Arc<IndexMetaSnapshot>> {
        if self.collection_type.read().as_str() != "document" {
            return None;
        }
        if self.stored_schema().is_some() || self.is_versioned() {
            return None;
        }
        let meta = self.index_meta()?;
        meta.vector.is_empty().then_some(meta)
    }

    /// [`Self::insert_value`] for a document already in its stored form,
    /// `key` overriding its `_key` as the driver's key does. Returns the key
    /// and the document's API value in MessagePack.
    pub(crate) fn insert_raw(
        &self,
        meta: &IndexMetaSnapshot,
        key: Option<&str>,
        doc: &CanonicalDoc,
    ) -> Option<DbResult<(String, Vec<u8>)>> {
        let header = NewHeader::new(&self.name, document_key(key, doc)?);
        let view = indexed_view(&header.view(), &doc.data, &meta.read_fields)?;
        let stored = match header.stored(&doc.data) {
            Ok(stored) => stored,
            Err(e) => return Some(Err(e)),
        };
        let written = self.with_key_locked(&header.key, || {
            self.insert_raw_locked(&header.key, &stored, &view)
        });
        Some(
            written
                .and_then(|()| header.reply(&stored, &doc.data))
                .map(|reply| (header.key, reply)),
        )
    }

    /// `insert_locked` for stored bytes; the caller holds the key's lock.
    fn insert_raw_locked(&self, key: &str, stored: &[u8], view: &Value) -> DbResult<()> {
        let tokens = self.unique_tokens(view);
        let _unique_guard = self.lock_unique_tokens(&tokens);

        let db = &self.db;
        let cf = self.live_cf()?;

        // Audit D1: an existing key is a conflict, never an overwrite.
        if db
            .get_pinned_ks(&cf, Self::doc_key(key))
            .map_err(|e| DbError::InternalError(format!("Failed to check existing key: {}", e)))?
            .is_some()
        {
            return Err(key_exists_error(key));
        }
        if !tokens.is_empty() {
            self.check_unique_constraints(key, view)?;
        }

        let mut batch = KvBatch::default();
        batch.put_ks(&cf, Self::doc_key(key), stored);
        self.add_insert_entries(&mut batch, &cf, key, view)?;
        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to insert document: {}", e)))?;

        self.increment_count();
        self.publish_change(|| ChangeEvent {
            type_: ChangeType::Insert,
            key: key.to_string(),
            data: Some(event_value(stored)),
            old_data: None,
        });
        Ok(())
    }

    /// [`Self::insert_batch`] for documents already in their stored form.
    /// Returns how many were written.
    pub(crate) fn insert_batch_raw(
        &self,
        meta: &IndexMetaSnapshot,
        docs: &[CanonicalDoc],
    ) -> Option<DbResult<usize>> {
        if docs.is_empty() {
            return Some(Ok(0));
        }

        // Pass 1: keys in batch order, the first problem reported.
        let mut headers: Vec<NewHeader> = Vec::with_capacity(docs.len());
        let mut batch_keys: HashSet<String> = HashSet::with_capacity(docs.len());
        for doc in docs {
            let key = document_key(None, doc)?;
            if !batch_keys.insert(key.clone()) {
                return Some(Err(DbError::InvalidDocument(format!(
                    "Duplicate _key '{}' within batch",
                    key
                ))));
            }
            headers.push(NewHeader::new(&self.name, key));
        }
        let views: Vec<Value> = headers
            .iter()
            .zip(docs)
            .map(|(header, doc)| indexed_view(&header.view(), &doc.data, &meta.read_fields))
            .collect::<Option<_>>()?;
        Some(self.insert_batch_raw_checked(docs, &headers, &views))
    }

    fn insert_batch_raw_checked(
        &self,
        docs: &[CanonicalDoc],
        headers: &[NewHeader],
        views: &[Value],
    ) -> DbResult<usize> {
        let tokens: Vec<String> = views.iter().flat_map(|v| self.unique_tokens(v)).collect();

        // Pass 2, before any lock, as in `insert_batch`.
        let db = &self.db;
        let cf = self.live_cf()?;
        let mut batch = KvBatch::default();
        let mut stored: Vec<Vec<u8>> = Vec::with_capacity(docs.len());
        let mut doc_keys: Vec<Vec<u8>> = Vec::with_capacity(docs.len());
        for ((header, doc), view) in headers.iter().zip(docs).zip(views) {
            let bytes = header.stored(&doc.data)?;
            let doc_key = Self::doc_key(&header.key);
            batch.put_ks(&cf, &doc_key, &bytes);
            self.add_insert_entries(&mut batch, &cf, &header.key, view)?;
            stored.push(bytes);
            doc_keys.push(doc_key);
        }

        let key_guard = self.lock_keys(headers.iter().map(|h| h.key.as_str()));
        let unique_guard = self.lock_unique_tokens(&tokens);

        let existing = db.multi_get_ks(&cf, &doc_keys);
        let mut claimed: HashSet<String> = HashSet::new();
        for ((header, view), found) in headers.iter().zip(views).zip(existing) {
            let key = &header.key;
            if found
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to check existing key: {}", e))
                })?
                .is_some()
            {
                return Err(key_exists_error(key));
            }
            if !tokens.is_empty() {
                self.check_unique_constraints(key, view)?;
                for token in self.unique_tokens(view) {
                    if !claimed.insert(token) {
                        return Err(DbError::InvalidDocument(format!(
                            "Unique constraint violated: document '{}' repeats a unique value used earlier in the same batch",
                            key
                        )));
                    }
                }
            }
        }

        db.write(batch)
            .map_err(|e| DbError::InternalError(format!("Failed to batch insert: {}", e)))?;
        drop(unique_guard);
        drop(key_guard);
        self.persist_vector_indexes_throttled();

        let count = docs.len();
        self.doc_count.fetch_add(count, Ordering::Relaxed);
        self.count_dirty.store(true, Ordering::Relaxed);

        self.publish_changes(|| {
            headers
                .iter()
                .zip(&stored)
                .map(|(header, bytes)| ChangeEvent {
                    type_: ChangeType::Insert,
                    key: header.key.clone(),
                    data: Some(event_value(bytes)),
                    old_data: None,
                })
        });
        Ok(count)
    }

    /// [`Self::merge_update`] with `changes` already in stored form (system
    /// fields left out): the stored fields are merged as MessagePack, and
    /// only the indexed fields of either version are decoded. Returns the
    /// new document's API value in MessagePack.
    pub(crate) fn merge_update_raw(
        &self,
        meta: &IndexMetaSnapshot,
        key: &str,
        changes: &CanonicalDoc,
    ) -> Option<DbResult<Vec<u8>>> {
        let written = self.with_key_locked(key, || {
            // What `get` reads and how it fails.
            let cf = self.live_cf()?;
            // Read in place: the stored bytes are only parsed and copied from.
            let old = self
                .db
                .get_pinned_ks(&cf, Self::doc_key(key))
                .map_err(|e| DbError::InternalError(format!("Failed to get document: {}", e)))?
                .ok_or_else(|| DbError::DocumentNotFound(key.to_string()))?;
            self.merge_raw_locked(meta, key, &old, &changes.data)
        });
        match written {
            Ok(Some(reply)) => Some(Ok(reply)),
            // Not readable this way: nothing was written.
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }

    fn merge_raw_locked(
        &self,
        meta: &IndexMetaSnapshot,
        key: &str,
        old: &[u8],
        changes: &[u8],
    ) -> DbResult<Option<Vec<u8>>> {
        let Some(doc) = StoredDocRef::parse(old) else {
            return Ok(None);
        };
        let text = |b| std::str::from_utf8(b).ok();
        let (Some(doc_key), Some(id), Some(old_rev)) = (text(doc.key), text(doc.id), text(doc.rev))
        else {
            return Ok(None);
        };
        let (Some(created_at), Some(old_updated_at)) = (
            parse_stored_timestamp(doc.created_at),
            parse_stored_timestamp(doc.updated_at),
        ) else {
            return Ok(None);
        };
        let Some(data) = merge_fields(doc.data, changes) else {
            return Ok(None);
        };
        let old_header = Header {
            key: doc_key,
            id,
            rev: old_rev,
            created_at,
            updated_at: old_updated_at,
        };
        // As `Document::update`: a new revision, now as the update time.
        let rev = uuid_text(uuid::Uuid::new_v4());
        let updated_at = Utc::now();
        let new_header = Header {
            key: doc_key,
            id,
            rev: &rev,
            created_at,
            updated_at,
        };
        let (Some(old_view), Some(new_view)) = (
            indexed_view(&old_header, doc.data, &meta.read_fields),
            indexed_view(&new_header, &data, &meta.read_fields),
        ) else {
            return Ok(None);
        };
        let (created_text, updated_text) =
            (stored_timestamp(&created_at), stored_timestamp(&updated_at));
        let stored = serialize_doc_raw(doc_key, id, &rev, &created_text, &updated_text, &data)?;

        self.write_update_core(key, &old_view, &stored, &new_view)?;
        self.publish_change(|| ChangeEvent {
            type_: ChangeType::Update,
            key: key.to_string(),
            data: Some(event_value(&stored)),
            old_data: Some(event_value(old)),
        });
        api_bytes(&stored, |out| {
            write_api_value(out, doc_key, id, &rev, &created_text, &updated_text, &data)
        })
        .map(Some)
    }
}
