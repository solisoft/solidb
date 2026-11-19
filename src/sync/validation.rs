//! What a collection asks of its documents, set on this node and logged
//! for the others: its field rules (`storage::field_rules`) and its JSON
//! schema; and whether it keeps document history (`set_versioning`). Every transport sets them through here, so that none forgets
//! the log. A peer applies them with `Collection::apply_field_rules` and
//! `Collection::apply_schema`.

use std::sync::Arc;

use super::log::{LogEntry, SyncLog};
use super::protocol::Operation;
use crate::error::DbResult;
use crate::storage::field_rules::FieldRule;
use crate::storage::schema::CollectionSchema;
use crate::storage::Collection;

fn append(
    log: Option<&SyncLog>,
    database: &str,
    name: &str,
    operation: Operation,
    data: Option<Vec<u8>>,
) {
    if let Some(log) = log {
        let _ = log.append(LogEntry::new_op(database, name, operation, "", data));
    }
}

/// Declare `collection`'s field rules, in place of those it had; an empty
/// list removes them. Returns the rules as stored.
pub fn set_field_rules(
    collection: &Collection,
    log: Option<&SyncLog>,
    database: &str,
    name: &str,
    rules: Vec<FieldRule>,
) -> DbResult<Arc<Vec<FieldRule>>> {
    collection.set_field_rules(rules)?;
    let stored = collection.field_rules();
    append(
        log,
        database,
        name,
        Operation::SetFieldRules,
        serde_json::to_vec(stored.as_ref()).ok(),
    );
    Ok(stored)
}

/// Log the schema `collection` holds, for one set where nothing logged it:
/// a collection created with a schema, after its creation is logged.
pub fn log_schema(collection: &Collection, log: Option<&SyncLog>, database: &str, name: &str) {
    if let Some(schema) = collection.get_json_schema() {
        append(
            log,
            database,
            name,
            Operation::SetSchema,
            serde_json::to_vec(&schema).ok(),
        );
    }
}

/// Set `collection`'s JSON schema, or remove it.
pub fn set_schema(
    collection: &Collection,
    log: Option<&SyncLog>,
    database: &str,
    name: &str,
    schema: Option<CollectionSchema>,
) -> DbResult<()> {
    let logged = schema.as_ref().and_then(|s| serde_json::to_vec(s).ok());
    match schema {
        Some(schema) => collection.set_json_schema(schema)?,
        None => collection.remove_json_schema()?,
    }
    append(log, database, name, Operation::SetSchema, logged);
    Ok(())
}

/// Turn `collection`'s document versioning on or off, and log it for the
/// other nodes (card #998): a replica that kept no history answered
/// `AS OF` differently from the node that did.
pub fn set_versioning(
    collection: &Collection,
    log: Option<&SyncLog>,
    database: &str,
    name: &str,
    enabled: bool,
) -> DbResult<()> {
    if enabled {
        collection.enable_versioning()?;
    } else {
        collection.disable_versioning()?;
    }
    append(
        log,
        database,
        name,
        Operation::SetVersioning,
        serde_json::to_vec(&serde_json::json!({ "enabled": enabled })).ok(),
    );
    Ok(())
}
