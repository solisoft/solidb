//! Per-row mutation writes: UPDATE / REPLACE / INSERT with `OPTIONS`, and the
//! `OLD` / `NEW` pseudo-variables.
//!
//! `clauses.rs` keeps its bulk fast paths for plain mutations. As soon as a
//! statement needs something those paths cannot give — a pre-image (`OLD`),
//! a per-row `NEW` on UPDATE, `REPLACE`, or any OPTIONS — it writes row by
//! row through the helpers here.

use serde_json::{Map, Value};

use super::super::QueryExecutor;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{MutationOptions, OverwriteMode};
use crate::storage::Collection;
use crate::sync::protocol::Operation;

/// What one row's write produced: the stored document before and after.
/// `None` where there is no such document (`OLD` of an insert, `NEW` of a
/// remove or of an ignored insert).
pub(super) struct RowWrite {
    pub old: Option<Value>,
    pub new: Option<Value>,
    /// Counted as an update (`true`) or an insert (`false`) in the stats.
    pub updated: bool,
    /// Nothing was written (`overwriteMode: "ignore"` on an existing key).
    pub skipped: bool,
}

/// Document-level failures that `OPTIONS { ignoreErrors: true }` skips.
/// Anything else — an expression error, a timeout, storage trouble — still
/// fails the query.
pub(super) fn is_document_error(err: &DbError) -> bool {
    matches!(
        err,
        DbError::DocumentNotFound(_)
            | DbError::InvalidDocument(_)
            | DbError::ConflictError(_)
            | DbError::SchemaValidationError(_)
    )
}

/// The `_key` a mutation selector names: a key string, or a document with a
/// string `_key`. Errors are `InvalidDocument`, which `ignoreErrors` skips.
pub(super) fn selector_key(value: &Value, statement: &str) -> DbResult<String> {
    match value {
        Value::String(s) => Ok(s.clone()),
        Value::Object(obj) => obj
            .get("_key")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .ok_or_else(|| {
                DbError::InvalidDocument(format!(
                    "{}: selector object must have a _key field",
                    statement
                ))
            }),
        _ => Err(DbError::InvalidDocument(format!(
            "{}: selector must be a string key or an object with _key field",
            statement
        ))),
    }
}

/// Merge `patch` into `target` with AQL's `keepNull` / `mergeObjects`.
///
/// Top-level system attributes (`_key`, `_rev`, ...) in the patch are
/// skipped, as the storage layer's own merge skips them.
pub(super) fn merge_patch(
    target: &mut Map<String, Value>,
    patch: &Map<String, Value>,
    keep_null: bool,
    merge_objects: bool,
    top_level: bool,
) {
    for (key, value) in patch {
        if top_level && key.starts_with('_') {
            continue;
        }
        if value.is_null() && !keep_null {
            target.remove(key);
            continue;
        }
        match (value, target.get_mut(key)) {
            (Value::Object(sub_patch), Some(Value::Object(existing))) if merge_objects => {
                merge_patch(existing, sub_patch, keep_null, merge_objects, false);
            }
            (Value::Object(_), _) if !keep_null => {
                target.insert(key.clone(), strip_nulls(value.clone()));
            }
            _ => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

/// Remove `null`-valued attributes from objects, recursively (arrays are left
/// alone, as in AQL).
fn strip_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        other => other,
    }
}

/// The body handed to `insert_or_replace` for a REPLACE of `key`: the new
/// document minus the attributes the server owns, with `_key` pinned.
fn replacement_body(document: Value, key: &str, statement: &str) -> DbResult<Value> {
    let Value::Object(mut map) = document else {
        return Err(DbError::InvalidDocument(format!(
            "{}: the replacement must be an object",
            statement
        )));
    };
    for system in ["_id", "_rev", "_created_at", "_updated_at"] {
        map.remove(system);
    }
    map.insert("_key".to_string(), Value::String(key.to_string()));
    Ok(Value::Object(map))
}

impl<'a> QueryExecutor<'a> {
    /// UPDATE (merge) or REPLACE the document `key` with `changes`.
    ///
    /// The storage merge is shallow and keeps nulls; when OPTIONS ask for
    /// something else the merged document is computed here and written with
    /// `insert_or_replace`. That path, like REPLACE, reads then writes, so a
    /// concurrent writer of the same document between the two can be lost.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn write_update_row(
        &self,
        collection: &Collection,
        collection_name: &str,
        key: &str,
        changes: Value,
        replace: bool,
        options: &MutationOptions,
        want_old: bool,
    ) -> DbResult<RowWrite> {
        let statement = if replace { "REPLACE" } else { "UPDATE" };
        if !changes.is_object() {
            return Err(DbError::InvalidDocument(format!(
                "{}: changes must be an object",
                statement
            )));
        }

        let (old, doc) = if replace {
            // REPLACE requires the document to exist.
            let existing = collection.get(key)?;
            let body = replacement_body(changes, key, statement)?;
            let doc = collection.insert_or_replace(body)?;
            (Some(existing.into_value()), doc)
        } else if options.needs_custom_merge() {
            let existing = collection.get(key)?;
            let mut data = existing.data.as_object().cloned().unwrap_or_default();
            if let Value::Object(patch) = &changes {
                merge_patch(
                    &mut data,
                    patch,
                    options.keep_null.unwrap_or(true),
                    options.merge_objects.unwrap_or(false),
                    true,
                );
            }
            data.insert("_key".to_string(), Value::String(key.to_string()));
            let doc = collection.insert_or_replace(Value::Object(data))?;
            (Some(existing.into_value()), doc)
        } else {
            let old = if want_old {
                Some(collection.get(key)?.into_value())
            } else {
                None
            };
            (old, collection.update(key, changes)?)
        };

        let new_value = doc.into_value();
        self.log_mutation(collection_name, Operation::Update, key, Some(&new_value));
        Ok(RowWrite {
            old: if want_old { old } else { None },
            new: Some(new_value),
            updated: true,
            skipped: false,
        })
    }

    /// REMOVE the document `key`, reading its pre-image first when `OLD` is
    /// wanted.
    pub(super) fn write_remove_row(
        &self,
        collection: &Collection,
        collection_name: &str,
        key: &str,
        want_old: bool,
    ) -> DbResult<RowWrite> {
        let old = if want_old {
            Some(collection.get(key)?.into_value())
        } else {
            None
        };
        collection.delete(key)?;
        self.log_mutation(collection_name, Operation::Delete, key, None);
        Ok(RowWrite {
            old,
            new: None,
            updated: false,
            skipped: false,
        })
    }

    /// INSERT `document`, honouring `overwriteMode` when its `_key` exists.
    pub(super) fn write_insert_row(
        &self,
        collection: &Collection,
        collection_name: &str,
        document: Value,
        options: &MutationOptions,
        want_old: bool,
    ) -> DbResult<RowWrite> {
        let mode = options.overwrite_mode.unwrap_or(OverwriteMode::Conflict);
        let key = document
            .get("_key")
            .and_then(|k| k.as_str())
            .map(str::to_string);

        let key = match (mode, key) {
            (OverwriteMode::Conflict, _) | (_, None) => {
                return self.plain_insert(collection, collection_name, document);
            }
            (_, Some(key)) => key,
        };

        let existing = match collection.get(&key) {
            Ok(doc) => Some(doc),
            Err(DbError::DocumentNotFound(_)) => None,
            Err(e) => return Err(e),
        };

        let Some(existing) = existing else {
            // Absent now; a concurrent insert of the same key between the
            // lookup and the write surfaces as a conflict. Treat that like
            // the key having existed all along.
            match self.plain_insert(collection, collection_name, document.clone()) {
                Err(DbError::ConflictError(_)) => {}
                other => return other,
            }
            let raced = collection.get(&key)?;
            return self.overwrite_existing(
                collection,
                collection_name,
                &key,
                raced.into_value(),
                document,
                mode,
                options,
                want_old,
            );
        };

        self.overwrite_existing(
            collection,
            collection_name,
            &key,
            existing.into_value(),
            document,
            mode,
            options,
            want_old,
        )
    }

    fn plain_insert(
        &self,
        collection: &Collection,
        collection_name: &str,
        document: Value,
    ) -> DbResult<RowWrite> {
        let doc = collection.insert(document)?;
        let key = doc.key.clone();
        let new_value = doc.into_value();
        self.log_mutation(collection_name, Operation::Insert, &key, Some(&new_value));
        Ok(RowWrite {
            old: None,
            new: Some(new_value),
            updated: false,
            skipped: false,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn overwrite_existing(
        &self,
        collection: &Collection,
        collection_name: &str,
        key: &str,
        existing: Value,
        document: Value,
        mode: OverwriteMode,
        options: &MutationOptions,
        want_old: bool,
    ) -> DbResult<RowWrite> {
        match mode {
            OverwriteMode::Ignore => Ok(RowWrite {
                old: want_old.then_some(existing),
                new: None,
                updated: false,
                skipped: true,
            }),
            OverwriteMode::Replace => {
                let body = replacement_body(document, key, "INSERT")?;
                let new_value = collection.insert_or_replace(body)?.into_value();
                self.log_mutation(collection_name, Operation::Update, key, Some(&new_value));
                Ok(RowWrite {
                    old: want_old.then_some(existing),
                    new: Some(new_value),
                    updated: true,
                    skipped: false,
                })
            }
            OverwriteMode::Update => {
                let mut write = self.write_update_row(
                    collection,
                    collection_name,
                    key,
                    document,
                    false,
                    options,
                    false,
                )?;
                write.old = want_old.then_some(existing);
                Ok(write)
            }
            OverwriteMode::Conflict => Err(DbError::ConflictError(format!(
                "Document with _key '{}' already exists",
                key
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn shallow_merge_replaces_nested_objects() {
        let mut t = obj(json!({"a": {"x": 1, "y": 2}, "b": 1}));
        merge_patch(&mut t, &obj(json!({"a": {"x": 9}})), true, false, true);
        assert_eq!(Value::Object(t), json!({"a": {"x": 9}, "b": 1}));
    }

    #[test]
    fn deep_merge_keeps_sibling_attributes() {
        let mut t = obj(json!({"a": {"x": 1, "y": 2}}));
        merge_patch(&mut t, &obj(json!({"a": {"x": 9}})), true, true, true);
        assert_eq!(Value::Object(t), json!({"a": {"x": 9, "y": 2}}));
    }

    #[test]
    fn keep_null_false_removes_attributes_at_every_level() {
        let mut t = obj(json!({"a": 1, "b": {"c": 1, "d": 2}}));
        merge_patch(
            &mut t,
            &obj(json!({"a": null, "b": {"c": null}})),
            false,
            true,
            true,
        );
        assert_eq!(Value::Object(t), json!({"b": {"d": 2}}));
    }

    #[test]
    fn keep_null_true_stores_null() {
        let mut t = obj(json!({"a": 1}));
        merge_patch(&mut t, &obj(json!({"a": null})), true, false, true);
        assert_eq!(Value::Object(t), json!({"a": null}));
    }

    #[test]
    fn system_attributes_in_patch_are_ignored() {
        let mut t = obj(json!({"a": 1}));
        merge_patch(
            &mut t,
            &obj(json!({"_key": "x", "_rev": "r"})),
            true,
            false,
            true,
        );
        assert_eq!(Value::Object(t), json!({"a": 1}));
    }

    #[test]
    fn replacement_body_pins_key_and_drops_server_attributes() {
        let body = replacement_body(
            json!({"_key": "other", "_rev": "r", "v": 1}),
            "k",
            "REPLACE",
        )
        .unwrap();
        assert_eq!(body, json!({"_key": "k", "v": 1}));
        assert!(replacement_body(json!(1), "k", "REPLACE").is_err());
    }

    #[test]
    fn selector_key_forms() {
        assert_eq!(selector_key(&json!("k"), "UPDATE").unwrap(), "k");
        assert_eq!(selector_key(&json!({"_key": "k"}), "UPDATE").unwrap(), "k");
        assert!(is_document_error(
            &selector_key(&json!(1), "UPDATE").unwrap_err()
        ));
    }
}
