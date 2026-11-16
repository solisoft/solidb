//! Columnar collections and the replication log, in one place.
//!
//! Creating, dropping and filling a columnar collection used to be logged
//! by whichever handler remembered to: the HTTP insert did, the HTTP create
//! and drop did not, and the driver logged nothing. A peer therefore never
//! had the collection, and every insert it received failed to load it.
//!
//! Every transport goes through [`create`], [`drop`], [`insert`],
//! [`delete_rows`], [`delete_where`] and [`truncate`], which
//! do the work on this node and append what a peer needs; the replication
//! worker applies those entries with [`apply_create`], [`apply_insert`],
//! [`apply_drop`], [`apply_create_index`] and [`apply_drop_index`]. A full
//! sync sends a collection whole (the second half of this file).

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
    /// How long rows are kept; an older node neither sends nor reads it.
    #[serde(default)]
    retention_secs: Option<u64>,
}

/// The payload of a `ColumnarSetRetention`.
#[derive(serde::Serialize, serde::Deserialize)]
struct Retention {
    seconds: Option<u64>,
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
                retention_secs: None,
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

/// Log the deletion of these rows, each under its id.
fn log_deleted(log: Option<&SyncLog>, db: &Database, name: &str, ids: &[String]) {
    let Some(log) = writing(log) else {
        return;
    };
    for chunk in ids.chunks(LOG_BATCH) {
        log.append_batch(
            chunk
                .iter()
                .map(|id| {
                    LogEntry::new_op(
                        db.name.as_str(),
                        name,
                        Operation::ColumnarDelete,
                        id.as_str(),
                        None,
                    )
                })
                .collect(),
        );
    }
}

/// Delete rows by id on this node and log each for the peers. Returns the
/// ids that were there.
pub fn delete_rows(
    db: &Database,
    log: Option<&SyncLog>,
    name: &str,
    ids: &[String],
) -> DbResult<Vec<String>> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    let deleted = col.delete_rows(ids)?;
    log_deleted(log, db, name, &deleted);
    Ok(deleted)
}

/// Delete the rows a filter keeps on this node, and log each for the
/// peers: by id, not as the filter, which a peer holding other rows would
/// answer differently. Returns how many went.
pub fn delete_where(
    db: &Database,
    log: Option<&SyncLog>,
    name: &str,
    filter: &crate::storage::columnar::ColumnFilter,
) -> DbResult<usize> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    let deleted = col.delete_where(filter)?;
    log_deleted(log, db, name, &deleted);
    Ok(deleted.len())
}

/// Empty a collection on this node, keeping its columns, and log it for
/// the peers.
pub fn truncate(db: &Database, log: Option<&SyncLog>, name: &str) -> DbResult<()> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    col.truncate()?;
    if let Some(log) = writing(log) {
        log.append(LogEntry::new_op(
            db.name.as_str(),
            name,
            Operation::ColumnarTruncate,
            "",
            None,
        ));
    }
    Ok(())
}

fn log_column(
    log: Option<&SyncLog>,
    db: &Database,
    name: &str,
    operation: Operation,
    column: &str,
    data: Option<Vec<u8>>,
) {
    if let Some(log) = writing(log) {
        log.append(LogEntry::new_op(
            db.name.as_str(),
            name,
            operation,
            column,
            data,
        ));
    }
}

/// Add a column to a collection, and log it for the peers.
pub fn add_column(
    db: &Database,
    log: Option<&SyncLog>,
    name: &str,
    column: ColumnDef,
) -> DbResult<()> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    col.add_column(column.clone())?;
    log_column(
        log,
        db,
        name,
        Operation::ColumnarAddColumn,
        &column.name,
        serde_json::to_vec(&column).ok(),
    );
    Ok(())
}

/// Drop a column of a collection, and log it for the peers.
pub fn drop_column(db: &Database, log: Option<&SyncLog>, name: &str, column: &str) -> DbResult<()> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    col.drop_column(column)?;
    log_column(log, db, name, Operation::ColumnarDropColumn, column, None);
    Ok(())
}

/// Rename a column of a collection, and log it for the peers.
pub fn rename_column(
    db: &Database,
    log: Option<&SyncLog>,
    name: &str,
    from: &str,
    to: &str,
) -> DbResult<()> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    col.rename_column(from, to)?;
    log_column(
        log,
        db,
        name,
        Operation::ColumnarRenameColumn,
        from,
        serde_json::to_vec(&serde_json::json!({ "name": to })).ok(),
    );
    Ok(())
}

/// Apply a peer's change of a collection's columns. A replayed entry
/// finds its work done and changes nothing: a column that is there is not
/// added, one that is gone is not dropped, one that has its new name is
/// not renamed.
pub fn apply_column_change(
    db: &Database,
    name: &str,
    operation: Operation,
    column: &str,
    data: Option<&[u8]>,
) -> DbResult<()> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    let has = |column: &str| -> DbResult<bool> {
        Ok(col.metadata()?.columns.iter().any(|c| c.name == column))
    };
    match operation {
        Operation::ColumnarAddColumn => {
            let definition: ColumnDef = serde_json::from_slice(data.unwrap_or_default())?;
            if has(&definition.name)? {
                return Ok(());
            }
            col.add_column(definition)
        }
        Operation::ColumnarDropColumn => {
            if !has(column)? {
                return Ok(());
            }
            // The origin dropped the column's index before it; here the
            // index may still be on its way out.
            if col.has_index(column) {
                col.drop_index(column)?;
            }
            col.drop_column(column)
        }
        Operation::ColumnarRenameColumn => {
            let to = serde_json::from_slice::<Value>(data.unwrap_or_default())
                .ok()
                .and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_string))
                .ok_or_else(|| {
                    crate::error::DbError::BadRequest("a rename without a name".to_string())
                })?;
            if !has(column)? || has(&to)? {
                return Ok(());
            }
            if col.has_index(column) {
                col.drop_index(column)?;
            }
            col.rename_column(column, &to)
        }
        _ => Ok(()),
    }
}

/// Set how long a collection keeps its rows (`None`: until they are
/// deleted), and log it for the peers. Each node removes its own old rows
/// ([`prune`]): what a node removed is not logged, since every node would
/// remove the same rows by the same rule.
pub fn set_retention(
    db: &Database,
    log: Option<&SyncLog>,
    name: &str,
    seconds: Option<u64>,
) -> DbResult<()> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    col.set_retention(seconds)?;
    if let Some(log) = writing(log) {
        log.append(LogEntry::new_op(
            db.name.as_str(),
            name,
            Operation::ColumnarSetRetention,
            "",
            serde_json::to_vec(&Retention { seconds }).ok(),
        ));
    }
    Ok(())
}

/// Apply a peer's `ColumnarSetRetention`.
pub fn apply_set_retention(db: &Database, name: &str, data: &[u8]) -> DbResult<()> {
    let retention: Retention = serde_json::from_slice(data)?;
    ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?
        .set_retention(retention.seconds)
}

/// Remove the rows a collection's retention no longer keeps, on this node.
/// Returns how many went.
pub fn prune(db: &Database, name: &str) -> DbResult<u64> {
    ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?.prune_expired()
}

/// Merge the small blocks of a collection, on this node: how many blocks
/// there were, and how many there are. Not logged: the rows are what they
/// were, and each node holds them in blocks of its own.
pub fn compact(db: &Database, name: &str) -> DbResult<(usize, usize)> {
    ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?.compact()
}

/// Apply a peer's `ColumnarDelete`. A row that is not there is deleted.
pub fn apply_delete(db: &Database, name: &str, id: &str) -> DbResult<()> {
    let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
    col.delete_row(id)?;
    Ok(())
}

/// Apply a peer's `ColumnarTruncate`.
pub fn apply_truncate(db: &Database, name: &str) -> DbResult<()> {
    ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?.truncate()
}

/// Apply a peer's `ColumnarCreateCollection`. `data` is what [`create`]
/// logged: the columns and the compression.
///
/// A collection that is already there is left alone: a log is replayed from
/// its start by a joining node, and creating it again would reset the row
/// count of one that has rows.
pub fn apply_create(db: &Database, name: &str, data: &[u8]) -> DbResult<()> {
    let definition: Definition = serde_json::from_slice(data)?;
    if let Ok(col) = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc()) {
        // What a full sync says of the retention holds for a collection
        // that is already here; a replayed create says nothing of it.
        if definition.retention_secs.is_some() {
            col.set_retention(definition.retention_secs)?;
        }
        return Ok(());
    }
    // The backing collection may be left from a create that stopped half
    // way; `new` says so if it is really missing.
    let _ = db.create_collection(backing(name), None);
    ColumnarCollection::new(
        name.to_string(),
        &db.name,
        db.db_arc(),
        definition.columns,
        definition.compression,
    )?
    .set_retention(definition.retention_secs)?;
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

// ---- Full sync -----------------------------------------------------------
//
// A node that joins is sent each columnar collection whole: its definition,
// its rows under their ids, then its indexes. The replication worker writes
// the parts with the functions below and applies them with
// [`apply_full_sync_part`].

/// The first part: the columns, none marked as indexed (the indexes come
/// after the rows), and the compression.
pub(crate) fn full_sync_definition(col: &ColumnarCollection) -> DbResult<Vec<u8>> {
    let meta = col.metadata()?;
    let columns = meta
        .columns
        .into_iter()
        .map(|c| ColumnDef {
            indexed: false,
            index_type: None,
            ..c
        })
        .collect();
    Ok(serde_json::to_vec(&Definition {
        columns,
        compression: meta.compression,
        retention_secs: meta.retention_secs,
    })?)
}

/// The last part: the indexed columns and the type of each index.
pub(crate) fn full_sync_indexes(col: &ColumnarCollection) -> DbResult<Vec<(String, Vec<u8>)>> {
    let mut indexes = Vec::new();
    for index in col.list_indexes()? {
        indexes.push((index.column, serde_json::to_vec(&index.index_type)?));
    }
    Ok(indexes)
}

/// The rows of a collection in id order, in batches of at most about
/// `budget` encoded bytes: `(data, rows)` for a `ColumnarPart::Rows`, not
/// compressed. A batch is rows one after another, each the MessagePack of
/// `(id, row)`: what the tail stores, which gives a float back as it was.
pub(crate) struct RowBatches {
    rows: crate::storage::columnar_blocks::RowSet,
    names: Vec<String>,
    /// Rows already handed out.
    done: usize,
}

impl RowBatches {
    pub(crate) fn of(col: &ColumnarCollection) -> DbResult<Self> {
        let names = col
            .metadata()?
            .columns
            .into_iter()
            .map(|c| c.name)
            .collect();
        Ok(Self {
            rows: col.row_set()?,
            names,
            done: 0,
        })
    }

    pub(crate) fn next_batch(&mut self, budget: usize) -> DbResult<Option<(Vec<u8>, u32)>> {
        let mut data = Vec::new();
        let mut count = 0u32;
        for at in self.rows.positions().skip(self.done) {
            let row: serde_json::Map<String, Value> = self
                .names
                .iter()
                .map(|name| (name.clone(), self.rows.value(at, name)))
                .collect();
            rmp_serde::encode::write(&mut data, &(self.rows.uuid(at), row)).map_err(|e| {
                crate::error::DbError::InternalError(format!("columnar full sync: {e}"))
            })?;
            count += 1;
            if data.len() >= budget {
                break;
            }
        }
        self.done += count as usize;
        Ok((count > 0).then_some((data, count)))
    }
}

/// Read back what [`RowBatches::next_batch`] wrote.
fn decode_rows(data: &[u8]) -> DbResult<Vec<(String, serde_json::Map<String, Value>)>> {
    let mut rows = Vec::new();
    let mut cursor = std::io::Cursor::new(data);
    while (cursor.position() as usize) < data.len() {
        let row = rmp_serde::from_read(&mut cursor).map_err(|e| {
            crate::error::DbError::BadRequest(format!("columnar full sync: unreadable rows: {e}"))
        })?;
        rows.push(row);
    }
    Ok(rows)
}

/// Apply one part of a columnar collection received in a full sync. Every
/// part may arrive again (a node may ask for a full sync twice): the
/// definition leaves a collection that is there alone, rows are taken by
/// id, and an index is built again from the rows as they then are.
pub fn apply_full_sync_part(
    db: &Database,
    name: &str,
    part: super::protocol::ColumnarPart,
) -> DbResult<()> {
    use super::protocol::ColumnarPart;
    match part {
        ColumnarPart::Definition(data) => apply_create(db, name, &data),
        ColumnarPart::Rows {
            data, compressed, ..
        } => {
            let data = if compressed {
                super::protocol::decompress_checked(&data)
                    .map_err(crate::error::DbError::BadRequest)?
            } else {
                data
            };
            let rows = decode_rows(&data)?;
            let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
            col.insert_rows_with_ids(rows)?;
            Ok(())
        }
        ColumnarPart::Indexes(indexes) => {
            let col = ColumnarCollection::load(name.to_string(), &db.name, db.db_arc())?;
            for (column, index_type) in indexes {
                let index_type: ColumnarIndexType = serde_json::from_slice(&index_type)?;
                if col.has_index(&column) {
                    col.drop_index(&column)?;
                }
                col.create_index(&column, index_type)?;
            }
            Ok(())
        }
    }
}
