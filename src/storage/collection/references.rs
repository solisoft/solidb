//! The collection's side of foreign keys: see `storage::foreign_keys`.

use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::foreign_keys::{self, ForeignKey, HeldGate, Links, OnDelete};
use crate::transaction::Operation;
use std::collections::HashMap;

/// The collection's links as last read, with the epoch they were read at.
pub(crate) type CachedLinks = (Option<u64>, Option<Arc<Links>>);

/// Documents of a child that refer to a parent that is not there.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Orphans {
    /// How many there are.
    pub count: usize,
    /// The first of them: `{ "_key": …, "<field>": … }`.
    pub sample: Vec<Value>,
}

/// Orphans listed in an answer or an error.
const ORPHAN_SAMPLE: usize = 20;

impl Collection {
    /// What this collection has to do with foreign keys: `None` when
    /// nothing, which costs one atomic load and an uncontended read lock.
    pub(crate) fn fk(&self) -> Option<Arc<Links>> {
        let now = foreign_keys::epoch();
        {
            let cached = self.fk_links.read();
            if cached.0 == Some(now) {
                return cached.1.clone();
            }
        }
        let links = foreign_keys::links_of(&self.db, &self.name);
        *self.fk_links.write() = (Some(now), links.clone());
        links
    }

    /// Hold the database's foreign-key gate for a write to this collection,
    /// before any key lock. `None` when the collection takes part in no
    /// constraint, or the thread holds the gate already.
    pub(crate) fn enter_gate(&self, exclusive: bool) -> DbResult<Option<HeldGate>> {
        match self.fk() {
            Some(links) => links.gate.enter(exclusive),
            None => Ok(None),
        }
    }

    /// The links of this collection when other documents may refer to its
    /// own: a delete then goes through [`Self::delete_referenced`].
    pub(crate) fn referenced(&self) -> Option<Arc<Links>> {
        self.fk().filter(|links| !links.referrers.is_empty())
    }

    /// A child collection, other than this one, that refers to this one.
    pub(crate) fn referring_collection(&self) -> Option<String> {
        self.fk()?
            .referrers
            .iter()
            .find(|r| r.child != self.name)
            .map(|r| r.child.clone())
    }

    /// Check that every parent `new` refers to exists. The caller holds the
    /// gate. `old` is the document being replaced: a reference it already
    /// held is not looked up again. `written_with` says whether a key of
    /// this collection is written by the same batch (a document may refer
    /// to itself, or to another document of its batch).
    pub(crate) fn check_references(
        &self,
        key: &str,
        new: &Value,
        old: Option<&Value>,
        written_with: impl Fn(&str) -> bool,
    ) -> DbResult<()> {
        let Some(links) = self.fk() else {
            return Ok(());
        };
        for link in &links.outgoing {
            let Some(parent_key) = foreign_keys::reference(&link.key, &self.name, new)? else {
                continue;
            };
            let kept = old.is_some_and(|old| {
                old.get(&link.key.field).and_then(Value::as_str) == Some(parent_key)
            });
            if kept {
                continue;
            }
            if link.parent == self.name && (parent_key == key || written_with(parent_key)) {
                continue;
            }
            if !foreign_keys::parent_is_stored(&self.db, link, parent_key)? {
                return Err(foreign_keys::missing_parent(link, &self.name, parent_key));
            }
        }
        Ok(())
    }

    /// The keys of the documents whose `field` is `parent_key`.
    pub(crate) fn keys_referencing(&self, field: &str, parent_key: &str) -> DbResult<Vec<String>> {
        let value = Value::String(parent_key.to_string());
        if let Some((keys, _)) = self.index_eq_doc_keys(field, &value, 0, usize::MAX, |_| Ok(()))? {
            return Ok(keys
                .into_iter()
                .filter_map(|k| String::from_utf8(k[DOC_PREFIX.len()..].to_vec()).ok())
                .collect());
        }
        // The constraint's index is gone: read the documents.
        let mut keys = Vec::new();
        self.for_each_value(None, |doc| {
            if doc.get(field).and_then(Value::as_str) == Some(parent_key) {
                if let Some(key) = doc.get("_key").and_then(Value::as_str) {
                    keys.push(key.to_string());
                }
            }
            Ok(())
        })?;
        Ok(keys)
    }

    /// Delete `keys` from a collection other documents refer to, with what
    /// the constraints make of their children — refused, deleted too, or
    /// their field set to null — as one write. `extra_deletes` are raw keys
    /// of this collection removed by the same write. A key that is not
    /// stored is skipped, or is an error when `must_exist`.
    pub(crate) fn delete_referenced(
        &self,
        links: &Links,
        keys: impl IntoIterator<Item = String>,
        extra_deletes: Vec<Vec<u8>>,
        must_exist: bool,
    ) -> DbResult<usize> {
        // Exclusive: no other write reaches this collection or its children
        // until the delete is in, so they are read here without their locks.
        let _gate = links.gate.enter(true)?;
        let (database, collection) = self.name.split_once(':').unwrap_or(("", &self.name));

        let mut operations = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for key in keys {
            if !seen.insert(key.clone()) {
                continue;
            }
            match self.get(&key) {
                Ok(doc) => operations.push(Operation::Delete {
                    database: database.to_string(),
                    collection: collection.to_string(),
                    key,
                    old_data: doc.to_value(),
                }),
                Err(DbError::DocumentNotFound(_)) if !must_exist => {}
                Err(e) => return Err(e),
            }
        }
        let deleted = operations.len();
        if deleted == 0 && extra_deletes.is_empty() {
            return Ok(0);
        }

        let mut prepared =
            crate::storage::group_commit::prepare(&self.db, operations, &[], &|name| {
                if name == self.name {
                    Ok(self.clone())
                } else {
                    foreign_keys::resolve(&self.db, name)
                }
            })?;
        if !extra_deletes.is_empty() {
            let cf = self.live_cf()?;
            for extra in extra_deletes {
                prepared.batch.delete_ks(&cf, extra);
            }
        }
        self.db
            .write(std::mem::take(&mut prepared.batch))
            .map_err(|e| DbError::InternalError(format!("Failed to delete document: {}", e)))?;
        prepared.finish();
        self.persist_vector_indexes_throttled();
        Ok(deleted)
    }

    // ------------------------------------------------------- declarations

    /// The constraints declared on this collection.
    pub fn foreign_keys(&self) -> Vec<ForeignKey> {
        foreign_keys::stored(&self.db, &self.ks)
    }

    fn store_foreign_keys(&self, keys: &[ForeignKey]) -> DbResult<()> {
        let cf = self.live_cf()?;
        let written = if keys.is_empty() {
            self.db
                .delete_ks(&cf, foreign_keys::FOREIGN_KEYS_KEY.as_bytes())
        } else {
            self.db.put_ks(
                &cf,
                foreign_keys::FOREIGN_KEYS_KEY.as_bytes(),
                serde_json::to_vec(keys)?,
            )
        };
        foreign_keys::bump_epoch();
        written.map_err(|e| DbError::InternalError(format!("Failed to store foreign keys: {}", e)))
    }

    fn database_name(&self) -> &str {
        self.name.split_once(':').map_or("", |(db, _)| db)
    }

    fn can_take_part(collection: &Collection, role: &str) -> DbResult<()> {
        let bare = collection.name.split_once(':').map_or("", |(_, c)| c);
        if bare.starts_with('_') {
            return Err(DbError::BadRequest(format!(
                "A system collection ('{}') cannot take part in a foreign key",
                bare
            )));
        }
        let kind = collection.get_type();
        if kind != "document" && kind != "edge" {
            return Err(DbError::BadRequest(format!(
                "The {} collection of a foreign key must be a document or edge collection; '{}' is a {} collection",
                role, bare, kind
            )));
        }
        if collection.is_sharded() {
            return Err(DbError::OperationNotSupported(format!(
                "Foreign keys are not supported on sharded collections ('{}')",
                bare
            )));
        }
        Ok(())
    }

    /// Declare that `field` holds the `_key` of a document of `references`
    /// (a collection of the same database) or null. Creates an index on
    /// `field` unless one serves it. Refused, and nothing is kept, when
    /// documents already stored refer to a parent that is not there.
    pub fn add_foreign_key(
        &self,
        name: Option<String>,
        field: &str,
        references: &str,
        on_delete: OnDelete,
    ) -> DbResult<ForeignKey> {
        if field.is_empty() || field.contains('.') || field.starts_with('_') {
            return Err(DbError::BadRequest(
                "A foreign key's field is a top-level field that is not a system attribute"
                    .to_string(),
            ));
        }
        let name = name.unwrap_or_else(|| format!("fk_{}", field));
        if name.is_empty() {
            return Err(DbError::BadRequest(
                "A foreign key's name cannot be empty".to_string(),
            ));
        }
        Self::can_take_part(self, "child")?;
        let parent_name = format!("{}:{}", self.database_name(), references);
        let parent = if parent_name == self.name {
            self.clone()
        } else {
            foreign_keys::resolve(&self.db, &parent_name)?
        };
        Self::can_take_part(&parent, "referenced")?;

        // Alone among the writers of the collections that already take
        // part; the others meet the constraint once the epoch has moved.
        let gate = foreign_keys::gate_of(&self.db, self.database_name());
        let _gate = gate.enter(true)?;

        let before = self.foreign_keys();
        if before.iter().any(|k| k.name == name) {
            return Err(DbError::ConflictError(format!(
                "Foreign key '{}' already exists",
                name
            )));
        }
        if before.iter().any(|k| k.field == field) {
            return Err(DbError::ConflictError(format!(
                "'{}' already has a foreign key",
                field
            )));
        }

        let (index, created) = match self.get_single_field_index_for_field(field) {
            Some(index) => {
                if !matches!(index.index_type, IndexType::Persistent | IndexType::Hash) {
                    return Err(DbError::BadRequest(format!(
                        "Index '{}' on '{}' cannot serve a foreign key: it must be a persistent or hash index",
                        index.name, field
                    )));
                }
                (index.name, false)
            }
            None => {
                let index = format!("fk_{}", field);
                self.create_index_with(
                    index.clone(),
                    vec![field.to_string()],
                    IndexType::Persistent,
                    false,
                    true,
                )?;
                (index, true)
            }
        };

        let key = ForeignKey {
            name,
            field: field.to_string(),
            references: references.to_string(),
            on_delete,
            index,
        };
        let mut after = before.clone();
        after.push(key.clone());
        self.store_foreign_keys(&after)?;

        let refused = match self.foreign_key_orphans(field, references) {
            Ok(orphans) if orphans.count == 0 => None,
            Ok(orphans) => {
                let keys: Vec<&str> = orphans
                    .sample
                    .iter()
                    .filter_map(|doc| doc.get("_key").and_then(Value::as_str))
                    .collect();
                Some(foreign_keys::violation(format!(
                    "{} document(s) of '{}' refer through '{}' to a document that is not in '{}' (first keys: {})",
                    orphans.count,
                    self.name.split_once(':').map_or("", |(_, c)| c),
                    field,
                    references,
                    keys.join(", ")
                )))
            }
            Err(e) => Some(e),
        };
        if let Some(error) = refused {
            self.store_foreign_keys(&before)?;
            if created {
                let _ = self.drop_index(&key.index);
            }
            return Err(error);
        }
        Ok(key)
    }

    /// Remove the constraint `name`. Its index stays.
    pub fn drop_foreign_key(&self, name: &str) -> DbResult<()> {
        let gate = foreign_keys::gate_of(&self.db, self.database_name());
        let _gate = gate.enter(true)?;
        let mut keys = self.foreign_keys();
        let before = keys.len();
        keys.retain(|k| k.name != name);
        if keys.len() == before {
            return Err(DbError::BadRequest(format!(
                "Foreign key '{}' not found",
                name
            )));
        }
        self.store_foreign_keys(&keys)
    }

    /// The constraint that looks its children up through the index `name`.
    pub(crate) fn foreign_key_using_index(&self, name: &str) -> Option<ForeignKey> {
        self.fk()?
            .outgoing
            .iter()
            .find(|link| link.key.index == name)
            .map(|link| link.key.clone())
    }

    /// The documents whose `field` names a document that is not in
    /// `references`: what declaring that constraint would be refused for.
    pub fn foreign_key_orphans(&self, field: &str, references: &str) -> DbResult<Orphans> {
        let parent_name = format!("{}:{}", self.database_name(), references);
        let parent_ks =
            crate::storage::collection_registry::keyspace_of(&self.db, &parent_name, |_| false)
                .ok_or_else(|| DbError::CollectionNotFound(references.to_string()))?;
        let parent_cf = parent_ks
            .handle(&self.db)
            .ok_or_else(|| DbError::CollectionNotFound(references.to_string()))?;

        let mut known: HashMap<String, bool> = HashMap::new();
        let mut orphans = Orphans {
            count: 0,
            sample: Vec::new(),
        };
        self.for_each_value(None, |doc| {
            let value = match doc.get(field) {
                None | Some(Value::Null) => return Ok(()),
                Some(value) => value,
            };
            let there = match value.as_str() {
                None => false,
                Some(parent_key) => match known.get(parent_key) {
                    Some(there) => *there,
                    None => {
                        let there = self
                            .db
                            .get_pinned_ks(&parent_cf, Self::doc_key(parent_key))
                            .map_err(|e| {
                                DbError::InternalError(format!(
                                    "Failed to read a referenced document: {}",
                                    e
                                ))
                            })?
                            .is_some();
                        if known.len() < 1_000_000 {
                            known.insert(parent_key.to_string(), there);
                        }
                        there
                    }
                },
            };
            if !there {
                orphans.count += 1;
                if orphans.sample.len() < ORPHAN_SAMPLE {
                    let mut row = serde_json::Map::new();
                    row.insert(
                        "_key".to_string(),
                        doc.get("_key").cloned().unwrap_or(Value::Null),
                    );
                    row.insert(field.to_string(), value.clone());
                    orphans.sample.push(Value::Object(row));
                }
            }
            Ok(())
        })?;
        Ok(orphans)
    }
}

/// A constraint of another collection that refers to this one.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReferencedBy {
    pub collection: String,
    #[serde(flatten)]
    pub key: ForeignKey,
}

impl Collection {
    /// The constraints, declared on any collection, that refer to this one.
    pub fn referenced_by(&self) -> Vec<ReferencedBy> {
        let Some(links) = self.fk() else {
            return Vec::new();
        };
        links
            .referrers
            .iter()
            .map(|r| ReferencedBy {
                collection: r.child.split_once(':').map_or("", |(_, c)| c).to_string(),
                key: r.key.clone(),
            })
            .collect()
    }
}
