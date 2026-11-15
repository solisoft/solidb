//! Columnar collections and the replication log, in one place.
//!
//! Creating, dropping and filling a columnar collection used to be logged
//! by whichever handler remembered to: the HTTP insert did, the HTTP create
//! and drop did not, and the driver logged nothing. A peer therefore never
//! had the collection, and every insert it received failed to load it.
//!
//! Every transport goes through [`create`], [`drop`] and [`insert`], which
//! do the work on this node and append what a peer needs; the replication
//! worker applies those entries with [`apply_create`], [`apply_insert`],
//! [`apply_drop`], [`apply_create_index`] and [`apply_drop_index`].

use serde_json::Value;

use super::log::{LogEntry, SyncLog};
use super::protocol::Operation;
use crate::error::DbResult;
use crate::storage::columnar::{ColumnDef, ColumnarCollection, ColumnarIndexType, CompressionType};
use crate::storage::Database;

/// Rows appended to the log in one batch: bounds what one insert holds
/// encoded in memory beside its rows.
const LOG_BATCH: usize = 4096;

/// The document collection whose keyspace holds a columnar collection.
fn backing(name: &str) -> String {
    format!("_columnar_{}", name)
}

/// What a peer needs to create the collection: the payload of a
/// `ColumnarCreateCollection`.
#[derive(serde::Serialize, serde::Deserialize)]
struct Definition {
    columns: Vec<ColumnDef>,
    #[serde(default)]
    compression: CompressionType,
}

/// The log, if it takes entries (a deferred or disabled one does not, and
/// nothing should be encoded for it).
fn writing(log: Option<&SyncLog>) -> Option<&SyncLog> {
    log.filter(|l| l.is_active())
}

/// Create a columnar collection on this node and log it for the peers.
pub fn create(
    db: &Database,
    log: Option<&SyncLog>,
    name: &str,
    columns: Vec<ColumnDef>,
    compression: CompressionType,
) -> DbResult<ColumnarCollection> {
    db.create_collection(backing(name), None)?;
    let col = ColumnarCollection::new(
        name.to_string(),
        &db.name,
        db.db_arc(),
        columns.clone(),
        compression.clone(),
    )?;
    if let Some(log) = writing(log) {
        log.append(LogEntry::new_op(
            db.name.as_str(),
            name,
            Operation::ColumnarCreateCollection,
            "",
            serde_json::to_vec(&Definition {
                columns,
                compression,
            })
            .ok(),
        ));
    }
    Ok(col)
}

/// Drop a columnar collection on this node and log it for the peers.
pub fn drop(db: &Database, log: Option<&SyncLog>, name: &str) -> DbResult<()> {
    db.delete_collection(&backing(name))?;
    if let Some(log) = writing(log) {
        log.append(LogEntry::new_op(
            db.name.as_str(),
            name,
            Operation::ColumnarDropCollection,
            "",
            None,
        ));
    }
    Ok(())
}

/// Insert rows on this node and log each under its id. Returns the ids.
pub fn insert(
    db: &Database,
    log: Option<&SyncLog>,
    name: &str,
    rows: Vec<Value>,
) -> DbResult<Vec<String>> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    let Some(log) = writing(log) else {
        return col.insert_rows(rows);
    };
    // Encoded before the insert, which takes the rows.
    let encoded: Vec<Option<Vec<u8>>> = rows.iter().map(|r| serde_json::to_vec(r).ok()).collect();
    let ids = col.insert_rows(rows)?;
    let mut entries = Vec::with_capacity(LOG_BATCH.min(ids.len()));
    for (id, data) in ids.iter().zip(encoded) {
        entries.push(LogEntry::new_op(
            db.name.as_str(),
            name,
            Operation::ColumnarInsert,
            id.as_str(),
            data,
        ));
        if entries.len() == LOG_BATCH {
            log.append_batch(std::mem::take(&mut entries));
        }
    }
    log.append_batch(entries);
    Ok(ids)
}

/// Apply a peer's `ColumnarCreateCollection`. `data` is what [`create`]
/// logged: the columns and the compression.
///
/// A collection that is already there is left alone: a log is replayed from
/// its start by a joining node, and creating it again would reset the row
/// count of one that has rows.
pub fn apply_create(db: &Database, name: &str, data: &[u8]) -> DbResult<()> {
    if ColumnarCollection::load(name.to_string(), &db.name, db.db_arc()).is_ok() {
        return Ok(());
    }
    let definition: Definition = serde_json::from_slice(data)?;
    // The backing collection may be left from a create that stopped half
    // way; `new` says so if it is really missing.
    let _ = db.create_collection(backing(name), None);
    ColumnarCollection::new(
        name.to_string(),
        &db.name,
        db.db_arc(),
        definition.columns,
        definition.compression,
    )?;
    Ok(())
}

/// Apply a peer's `ColumnarInsert`: the row, under the id its origin gave
/// it. A row that is already there is left alone.
pub fn apply_insert(db: &Database, name: &str, id: &str, data: &[u8]) -> DbResult<()> {
    let row: Value = serde_json::from_slice(data)?;
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    col.insert_row_with_id(id, row)?;
    Ok(())
}

/// Apply a peer's `ColumnarDropCollection`. One that is not there is
/// already dropped.
pub fn apply_drop(db: &Database, name: &str) -> DbResult<()> {
    match db.delete_collection(&backing(name)) {
        Err(crate::error::DbError::CollectionNotFound(_)) => Ok(()),
        other => other,
    }
}

/// Index a column on this node and log it for the peers.
pub fn create_index(
    db: &Database,
    log: Option<&SyncLog>,
    name: &str,
    column: &str,
    index_type: ColumnarIndexType,
) -> DbResult<()> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    col.create_index(column, index_type.clone())?;
    if let Some(log) = writing(log) {
        log.append(LogEntry::new_op(
            db.name.as_str(),
            name,
            Operation::ColumnarCreateIndex,
            column,
            serde_json::to_vec(&index_type).ok(),
        ));
    }
    Ok(())
}

/// Drop a column's index on this node and log it for the peers.
pub fn drop_index(db: &Database, log: Option<&SyncLog>, name: &str, column: &str) -> DbResult<()> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    col.drop_index(column)?;
    if let Some(log) = writing(log) {
        log.append(LogEntry::new_op(
            db.name.as_str(),
            name,
            Operation::ColumnarDropIndex,
            column,
            None,
        ));
    }
    Ok(())
}

/// Apply a peer's `ColumnarCreateIndex`: the index is built here from the
/// rows this node has. A column that already has an index of that type is
/// left alone; one of another type is replaced, the origin's being the one
/// every node should end with.
pub fn apply_create_index(db: &Database, name: &str, column: &str, data: &[u8]) -> DbResult<()> {
    let index_type: ColumnarIndexType = serde_json::from_slice(data)?;
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    if col.get_index_type(column).as_ref() == Some(&index_type) {
        return Ok(());
    }
    if col.has_index(column) {
        col.drop_index(column)?;
    }
    col.create_index(column, index_type)
}

/// Apply a peer's `ColumnarDropIndex`. A column with no index has already
/// dropped it.
pub fn apply_drop_index(db: &Database, name: &str, column: &str) -> DbResult<()> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    if !col.has_index(column) {
        return Ok(());
    }
    col.drop_index(column)
}
