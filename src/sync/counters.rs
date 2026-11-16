//! Counter fields and the replication log, in one place.
//!
//! Declaring a counter, setting its next number, handing it to another
//! node and dropping it are done here on every transport, and logged as
//! what a peer applies ([`apply`]). The numbers themselves travel in the
//! documents: a peer does not count, it stores what the numbering node
//! gave. What it keeps of the counter is the last record it was sent,
//! which is exact when the numbering node hands the counter over
//! ([`move_to`]), since that node stops numbering in the same write.

use serde_json::json;

use super::log::{LogEntry, SyncLog};
use super::protocol::Operation;
use crate::error::{DbError, DbResult};
use crate::storage::collection::counters::CounterSpec;
use crate::storage::counters::CounterRecord;
use crate::storage::Collection;

fn log_record(log: Option<&SyncLog>, database: &str, collection: &str, record: &CounterRecord) {
    if let Some(log) = log {
        let _ = log.append(LogEntry::new_op(
            database,
            collection,
            Operation::PutCounter,
            record.def.field.as_str(),
            serde_json::to_vec(record).ok(),
        ));
    }
}

/// Declare a counter on `collection`.
pub fn declare(
    collection: &Collection,
    log: Option<&SyncLog>,
    database: &str,
    name: &str,
    spec: CounterSpec,
) -> DbResult<CounterRecord> {
    let record = collection.add_counter(spec)?;
    log_record(log, database, name, &record);
    Ok(record)
}

/// Make `next` the counter's next number.
pub fn set_next(
    collection: &Collection,
    log: Option<&SyncLog>,
    database: &str,
    name: &str,
    field: &str,
    next: u64,
) -> DbResult<CounterRecord> {
    let record = collection.set_counter_next(field, next)?;
    log_record(log, database, name, &record);
    Ok(record)
}

/// Hand the numbering to `node`.
pub fn move_to(
    collection: &Collection,
    log: Option<&SyncLog>,
    database: &str,
    name: &str,
    field: &str,
    node: Option<String>,
) -> DbResult<CounterRecord> {
    let record = collection.move_counter(field, node)?;
    log_record(log, database, name, &record);
    Ok(record)
}

/// Take the numbering for this node, without the node that numbers.
pub fn take_over(
    collection: &Collection,
    log: Option<&SyncLog>,
    database: &str,
    name: &str,
    field: &str,
) -> DbResult<CounterRecord> {
    let record = collection.take_over_counter(field)?;
    log_record(log, database, name, &record);
    Ok(record)
}

/// Drop the counter on `field`.
pub fn drop(
    collection: &Collection,
    log: Option<&SyncLog>,
    database: &str,
    name: &str,
    field: &str,
) -> DbResult<()> {
    let epoch = collection.drop_counter(field)?;
    if let Some(log) = log {
        let _ = log.append(LogEntry::new_op(
            database,
            name,
            Operation::DropCounter,
            field,
            serde_json::to_vec(&json!({ "epoch": epoch })).ok(),
        ));
    }
    Ok(())
}

/// Apply a peer's `PutCounter` or `DropCounter` to `collection`. Takes any
/// entry twice, and in any order: the later change of a counter wins.
pub fn apply(
    collection: &Collection,
    operation: Operation,
    field: &str,
    data: Option<&[u8]>,
) -> DbResult<()> {
    let data = data.unwrap_or_default();
    match operation {
        Operation::PutCounter => {
            let record: CounterRecord = serde_json::from_slice(data)
                .map_err(|e| DbError::BadRequest(format!("Invalid counter: {}", e)))?;
            collection.apply_counter(&record)
        }
        Operation::DropCounter => {
            let epoch = serde_json::from_slice::<serde_json::Value>(data)
                .ok()
                .and_then(|v| v.get("epoch").and_then(serde_json::Value::as_u64))
                .unwrap_or(u64::MAX);
            collection.apply_counter_drop(field, epoch)
        }
        _ => Ok(()),
    }
}

/// The counters of `collection` as a full sync sends them.
pub fn full_sync_records(collection: &Collection) -> Vec<Vec<u8>> {
    collection
        .counter_records()
        .unwrap_or_default()
        .iter()
        .filter_map(|record| serde_json::to_vec(record).ok())
        .collect()
}

/// Store the counters a full sync sent.
pub fn apply_full_sync(collection: &Collection, records: &[Vec<u8>]) {
    for record in records {
        match serde_json::from_slice::<CounterRecord>(record) {
            Ok(record) => {
                if let Err(e) = collection.apply_counter(&record) {
                    tracing::warn!(
                        "Full sync: counter '{}' not stored: {}",
                        record.def.field,
                        e
                    );
                }
            }
            Err(e) => tracing::warn!("Full sync: unreadable counter: {}", e),
        }
    }
}
