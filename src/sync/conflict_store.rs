//! Conflict detection and storage for offline-sync pushes.
//!
//! Documents carry no version vector, so the server keeps a small side record
//! per document it has synced (`_sync_versions`): the sync-log sequence and
//! `_rev` of the last write it made, and the device that made it.
//!
//! A pulled change arrives with a vector `{server_node: log_sequence}`, so a
//! client's vector says how far into this node's log it has seen. A pushed
//! change therefore conflicts when the document changed **after** the client's
//! last-seen sequence:
//!
//! * by another device's sync write (`seen < recorded sequence`), or
//! * by an ordinary write (the document's `_rev` no longer matches the
//!   record), unless the client has since pulled it (`seen > recorded
//!   sequence`).
//!
//! A device never conflicts with its own earlier push, and a change without a
//! vector, or for a document the server has never synced, keeps the old
//! last-write-wins behaviour. A conflicting change is *not* applied: it is
//! stored in `_sync_conflicts` until it is resolved.

use crate::error::{DbError, DbResult};
use crate::storage::{Collection, Database};
use crate::sync::session::{ChangeOperation, SyncChange};
use crate::sync::version_vector::{ConflictInfo, VersionVector};
use serde_json::{json, Value};

pub const VERSIONS_COLLECTION: &str = "_sync_versions";
pub const CONFLICTS_COLLECTION: &str = "_sync_conflicts";

/// What the server knows about the last sync write to one document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncRecord {
    /// Sync-log sequence assigned to that write (0 if the log was off).
    pub seq: u64,
    /// The document's `_rev` right after it.
    pub rev: String,
    /// The device that made it.
    pub device: String,
}

fn ensure(db: &Database, name: &str) -> DbResult<Collection> {
    match db.system_collection(name) {
        Ok(c) => Ok(c),
        Err(DbError::CollectionNotFound(_)) => {
            match db.create_collection(name.to_string(), None) {
                Ok(_) | Err(DbError::CollectionAlreadyExists(_)) => {}
                Err(e) => return Err(e),
            }
            db.system_collection(name)
        }
        Err(e) => Err(e),
    }
}

fn record_key(collection: &str, key: &str) -> String {
    format!("{}/{}", collection, key)
}

pub fn get_record(db: &Database, collection: &str, key: &str) -> Option<SyncRecord> {
    let versions = db.system_collection(VERSIONS_COLLECTION).ok()?;
    let doc = versions.get(&record_key(collection, key)).ok()?.to_value();
    Some(SyncRecord {
        seq: doc.get("seq")?.as_u64()?,
        rev: doc.get("rev")?.as_str()?.to_string(),
        device: doc.get("device")?.as_str()?.to_string(),
    })
}

/// Remember the write the server just made for `change`. A delete forgets the
/// document.
pub fn record_write(db: &Database, change: &SyncChange, seq: u64, device: &str) -> DbResult<()> {
    let versions = ensure(db, VERSIONS_COLLECTION)?;
    let id = record_key(&change.collection, &change.document_key);
    if change.operation == ChangeOperation::Delete {
        return match versions.delete(&id) {
            Ok(()) | Err(DbError::DocumentNotFound(_)) => Ok(()),
            Err(e) => Err(e),
        };
    }
    let rev = db
        .get_collection(&change.collection)?
        .get(&change.document_key)?
        .revision()
        .to_string();
    versions.insert_or_replace(json!({
        "_key": id,
        "seq": seq,
        "rev": rev,
        "device": device,
    }))?;
    Ok(())
}

/// Is `change` a concurrent edit? `None` means apply it.
pub fn detect(
    db: &Database,
    server_node: &str,
    device: &str,
    change: &SyncChange,
) -> DbResult<Option<ConflictInfo>> {
    if change.vector.is_empty() {
        return Ok(None);
    }
    let Some(rec) = get_record(db, &change.collection, &change.document_key) else {
        return Ok(None);
    };
    let current = match db
        .get_collection(&change.collection)
        .and_then(|c| c.get(&change.document_key))
    {
        Ok(doc) => Some(doc),
        Err(DbError::DocumentNotFound(_)) | Err(DbError::CollectionNotFound(_)) => None,
        Err(e) => return Err(e),
    };
    // Deleting what is already gone is the end state the client wants.
    if current.is_none() && change.operation == ChangeOperation::Delete {
        return Ok(None);
    }

    let rev_changed = current.as_ref().is_none_or(|d| d.revision() != rec.rev);
    let seen = change.vector.get(server_node);
    let conflict = if rev_changed {
        seen <= rec.seq
    } else if rec.device == device {
        false
    } else {
        seen < rec.seq
    };
    if !conflict {
        return Ok(None);
    }

    Ok(Some(ConflictInfo {
        document_key: change.document_key.clone(),
        collection: change.collection.clone(),
        local_vector: VersionVector::with_node(server_node, rec.seq),
        remote_vector: change.vector.clone(),
        local_data: current.map(|d| d.to_value()),
        remote_data: change.document_data.clone(),
        detected_at: chrono::Utc::now().timestamp_millis() as u64,
    }))
}

/// Keep a conflicting change until it is resolved. Returns the stored row
/// (its `_key` is the conflict id).
pub fn store_conflict(
    db: &Database,
    session_id: &str,
    user: &str,
    device: &str,
    change: &SyncChange,
    info: &ConflictInfo,
) -> DbResult<Value> {
    let conflicts = ensure(db, CONFLICTS_COLLECTION)?;
    let id = uuid::Uuid::now_v7().to_string();
    let row = json!({
        "_key": id,
        "session_id": session_id,
        "user": user,
        "device": device,
        "database": change.database,
        "collection": change.collection,
        "document_key": change.document_key,
        "change": serde_json::to_value(change)
            .map_err(|e| DbError::InternalError(e.to_string()))?,
        "local_vector": info.local_vector,
        "remote_vector": info.remote_vector,
        "local_data": info.local_data,
        "remote_data": info.remote_data,
        "detected_at": info.detected_at,
    });
    conflicts.insert(row.clone())?;
    Ok(row)
}

/// A stored conflict as a client sees it: the internals (`change`, session,
/// user) are dropped, and the server's copy of the document is shown only to a
/// caller who could read it anyway — pushing needs Write, not Read.
pub fn client_view(row: &Value, can_read: bool) -> Value {
    let field = |name: &str| row.get(name).cloned().unwrap_or(Value::Null);
    json!({
        "id": field("_key"),
        "database": field("database"),
        "collection": field("collection"),
        "document_key": field("document_key"),
        "operation": row.pointer("/change/operation").cloned().unwrap_or(Value::Null),
        "local_vector": field("local_vector"),
        "remote_vector": field("remote_vector"),
        "local_data": if can_read { field("local_data") } else { Value::Null },
        "remote_data": field("remote_data"),
        "detected_at": field("detected_at"),
    })
}

/// Open conflicts of one session in one database, oldest first.
pub fn open_conflicts(db: &Database, session_id: &str) -> Vec<Value> {
    let Ok(conflicts) = db.system_collection(CONFLICTS_COLLECTION) else {
        return Vec::new();
    };
    let mut rows: Vec<Value> = conflicts
        .scan_values(None)
        .into_iter()
        .filter(|r| r.get("session_id").and_then(Value::as_str) == Some(session_id))
        .collect();
    rows.sort_by_key(|r| r.get("detected_at").and_then(Value::as_u64).unwrap_or(0));
    rows
}

pub fn remove_conflict(db: &Database, id: &str) -> DbResult<()> {
    let conflicts = db.system_collection(CONFLICTS_COLLECTION)?;
    match conflicts.delete(id) {
        Ok(()) | Err(DbError::DocumentNotFound(_)) => Ok(()),
        Err(e) => Err(e),
    }
}
