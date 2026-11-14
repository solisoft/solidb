//! Checking an index against the documents it is supposed to describe.
//!
//! Every write keeps a document and its index entries in one batch, so on
//! the node that wrote they agree. They can stop agreeing where a write did
//! not go through that path's checks: a replicated write is applied without
//! them, so two nodes can each accept the same unique value; a migration or
//! a restore can leave entries behind. [`Collection::check_indexes`] reads
//! everything and says what it found. It changes nothing:
//! `rebuild_all_indexes` repairs entries, and a duplicated unique value is
//! the application's to settle.

use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::index::extract_field_value;
use crate::storage::kv::{Direction, IterMode};
use std::collections::{HashMap, HashSet};
use std::hash::Hasher;

/// Findings listed per kind and per index.
const SAMPLE: usize = 20;

/// A count, and the first cases found.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Finding {
    pub count: usize,
    pub sample: Vec<Value>,
}

impl Finding {
    fn add(&mut self, case: impl FnOnce() -> Value) {
        self.count += 1;
        if self.sample.len() < SAMPLE {
            self.sample.push(case());
        }
    }
}

/// One index compared with the collection's documents.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexCheck {
    pub name: String,
    pub index_type: IndexType,
    pub fields: Vec<String>,
    pub unique: bool,
    /// Entries stored for the index.
    pub entries: usize,
    /// Entries the documents should have and that are not stored:
    /// `{"_key", "values"}`. A read through the index misses these
    /// documents.
    pub missing: Finding,
    /// Entries stored that no document accounts for: `{"_key"}`, the
    /// document the entry names (gone, or holding another value now).
    pub stale: Finding,
    /// Unique indexes only: values held by more than one document:
    /// `{"values", "keys"}` (the first two documents found).
    pub duplicates: Finding,
}

impl IndexCheck {
    /// Whether the index describes its documents exactly.
    pub fn is_consistent(&self) -> bool {
        self.missing.count == 0 && self.stale.count == 0 && self.duplicates.count == 0
    }
}

/// What [`Collection::check_indexes`] found.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexReport {
    pub documents: usize,
    pub consistent: bool,
    pub indexes: Vec<IndexCheck>,
    /// Indexes of the families this does not read: geo, fulltext, TTL and
    /// vector.
    pub not_checked: Vec<String>,
}

fn hash(bytes: &[u8]) -> u64 {
    let mut hasher = seahash::SeaHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// What one index needs while the documents are read.
struct Expected {
    /// Hashes of the entry keys the documents call for.
    entries: HashSet<u64>,
    /// Unique indexes: hash of a value -> the first document holding it.
    holders: HashMap<u64, String>,
}

impl Collection {
    /// Compare every regular index (hash, persistent, trigram, bloom,
    /// cuckoo) with the documents. Reads the whole collection, twice when
    /// an entry is missing, and every index entry: for an operator, not for
    /// a request path. Writes running meanwhile can show up as findings.
    pub fn check_indexes(&self) -> DbResult<IndexReport> {
        let meta = self.index_meta().unwrap_or_default();
        let cf = self.live_cf()?;
        let mut checks: Vec<IndexCheck> = meta
            .indexes
            .iter()
            .map(|index| IndexCheck {
                name: index.name.clone(),
                index_type: index.index_type.clone(),
                fields: index.fields.clone(),
                unique: index.unique,
                entries: 0,
                missing: Finding::default(),
                stale: Finding::default(),
                duplicates: Finding::default(),
            })
            .collect();
        let mut expected: Vec<Expected> = meta
            .indexes
            .iter()
            .map(|_| Expected {
                entries: HashSet::new(),
                holders: HashMap::new(),
            })
            .collect();

        // What the documents call for.
        let documents = self.for_each_value(None, |doc| {
            let Some(doc_key) = doc.get("_key").and_then(Value::as_str) else {
                return Ok(());
            };
            for ((index, want), check) in meta.indexes.iter().zip(&mut expected).zip(&mut checks) {
                let values: Vec<Value> = index
                    .fields
                    .iter()
                    .map(|field| extract_field_value(&doc, field))
                    .collect();
                if !index.holds(&values) {
                    continue;
                }
                for entry in Self::index_entry_keys(index, &values, doc_key) {
                    want.entries.insert(hash(&entry));
                }
                // A unique index never constrains a value that is all null.
                if index.unique && !values.iter().all(Value::is_null) {
                    let value = hash(&Self::idx_entry_key(&index.name, &values, ""));
                    match want.holders.get(&value) {
                        Some(first) => {
                            let first = first.clone();
                            check.duplicates.add(
                                || serde_json::json!({"values": values, "keys": [first, doc_key]}),
                            );
                        }
                        None => {
                            want.holders.insert(value, doc_key.to_string());
                        }
                    }
                }
            }
            Ok(())
        })?;

        // What is stored.
        let mut stored: Vec<HashSet<u64>> = Vec::with_capacity(checks.len());
        for ((index, want), check) in meta.indexes.iter().zip(&expected).zip(&mut checks) {
            let prefix = format!("{}{}:", IDX_PREFIX, index.name).into_bytes();
            let mut upper = prefix.clone();
            *upper.last_mut().expect("the prefix ends in ':'") += 1;
            let mut seen = HashSet::new();
            let entries = self.db.iterator_ks_bounded(
                &cf,
                IterMode::From(&prefix, Direction::Forward),
                None,
                Some(&upper),
            );
            for entry in entries {
                let (key, doc_key) = entry.map_err(|e| DbError::InternalError(e.to_string()))?;
                if !key.starts_with(&prefix) {
                    break;
                }
                check.entries += 1;
                let entry = hash(&key);
                if want.entries.contains(&entry) {
                    seen.insert(entry);
                } else {
                    check
                        .stale
                        .add(|| serde_json::json!({"_key": String::from_utf8_lossy(&doc_key)}));
                }
            }
            check.missing.count = want.entries.len() - seen.len();
            stored.push(seen);
        }

        // Which documents the missing entries belong to.
        if checks.iter().any(|check| check.missing.count > 0) {
            self.for_each_value(None, |doc| {
                let Some(doc_key) = doc.get("_key").and_then(Value::as_str) else {
                    return Ok(());
                };
                for ((index, seen), check) in meta.indexes.iter().zip(&stored).zip(&mut checks) {
                    if check.missing.count == 0 || check.missing.sample.len() >= SAMPLE {
                        continue;
                    }
                    let values: Vec<Value> = index
                        .fields
                        .iter()
                        .map(|field| extract_field_value(&doc, field))
                        .collect();
                    if !index.holds(&values) {
                        continue;
                    }
                    let absent = Self::index_entry_keys(index, &values, doc_key)
                        .iter()
                        .any(|entry| !seen.contains(&hash(entry)));
                    if absent {
                        check
                            .missing
                            .sample
                            .push(serde_json::json!({"_key": doc_key, "values": values}));
                    }
                }
                Ok(())
            })?;
        }

        let mut not_checked: Vec<String> = Vec::new();
        not_checked.extend(meta.geo.iter().map(|index| index.name.clone()));
        not_checked.extend(meta.fulltext.iter().map(|index| index.name.clone()));
        not_checked.extend(meta.ttl.iter().map(|index| index.name.clone()));
        not_checked.extend(meta.vector.iter().map(|index| index.name.clone()));

        Ok(IndexReport {
            documents,
            consistent: checks.iter().all(IndexCheck::is_consistent),
            indexes: checks,
            not_checked,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::kv::KvBatch;
    use crate::storage::StorageEngine;
    use serde_json::json;

    fn collection() -> (tempfile::TempDir, Collection) {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = StorageEngine::new(tmp.path().to_str().unwrap()).unwrap();
        engine.initialize().unwrap();
        engine.create_database("d".to_string()).unwrap();
        let db = engine.get_database("d").unwrap();
        db.create_collection("users".to_string(), None).unwrap();
        let users = db.get_collection("users").unwrap();
        users
            .create_index(
                "by_email".to_string(),
                vec!["email".to_string()],
                IndexType::Hash,
                true,
            )
            .unwrap();
        users
            .create_index(
                "by_city".to_string(),
                vec!["city".to_string(), "age".to_string()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
        users
            .create_index(
                "grams".to_string(),
                vec!["name".to_string()],
                IndexType::Trigram,
                false,
            )
            .unwrap();
        users
            .create_ttl_index("ttl".to_string(), "expires".to_string(), 60)
            .unwrap();
        for i in 0..50 {
            users
                .insert(json!({
                    "_key": format!("u{i}"),
                    "email": format!("u{i}@example.com"),
                    "city": if i % 2 == 0 { json!("Lyon") } else { json!(null) },
                    "age": i,
                    "name": format!("User number {i}"),
                }))
                .unwrap();
        }
        // No email: a sparse unique index holds nothing for it.
        users.insert(json!({"_key": "anon", "name": "x"})).unwrap();
        (tmp, users)
    }

    fn check<'a>(report: &'a IndexReport, name: &str) -> &'a IndexCheck {
        report.indexes.iter().find(|c| c.name == name).unwrap()
    }

    #[test]
    fn indexes_kept_by_writes_are_consistent() {
        let (_tmp, users) = collection();
        users
            .update("u3", json!({"email": "three@example.com", "city": "Nice"}))
            .unwrap();
        users.delete("u4").unwrap();
        users
            .insert_batch(vec![
                json!({"_key": "b1", "email": "b1@example.com", "name": "Batch"}),
            ])
            .unwrap();

        let report = users.check_indexes().unwrap();
        assert!(report.consistent, "{report:?}");
        assert_eq!(report.documents, 51);
        assert_eq!(check(&report, "by_email").entries, 50);
        // The TTL index is not one this reads.
        assert_eq!(report.not_checked, ["ttl"]);
    }

    #[test]
    fn what_writes_never_leave_is_found() {
        let (_tmp, users) = collection();
        let cf = users.live_cf().unwrap();

        // An entry whose document is gone, and one for a value its document
        // no longer holds.
        let mut batch = KvBatch::default();
        batch.put_ks(
            &cf,
            Collection::idx_entry_key("by_email", &[json!("ghost@example.com")], "ghost"),
            b"ghost",
        );
        batch.put_ks(
            &cf,
            Collection::idx_entry_key("by_city", &[json!("Paris"), json!(1)], "u1"),
            b"u1",
        );
        // A document that lost its entry.
        batch.delete_ks(
            &cf,
            Collection::idx_entry_key("by_email", &[json!("u7@example.com")], "u7"),
        );
        users.db.write(batch).unwrap();
        // Two documents with one unique value, written without the index
        // (what two nodes that each accepted it look like once replicated),
        // and so without their entries either.
        users
            .insert_no_index(json!({"_key": "twin1", "email": "same@example.com"}))
            .unwrap();
        users
            .insert_no_index(json!({"_key": "twin2", "email": "same@example.com"}))
            .unwrap();

        let report = users.check_indexes().unwrap();
        assert!(!report.consistent);

        let email = check(&report, "by_email");
        assert_eq!(email.stale.count, 1);
        assert_eq!(email.stale.sample, [json!({"_key": "ghost"})]);
        assert_eq!(email.missing.count, 3);
        let mut missing: Vec<&str> = email
            .missing
            .sample
            .iter()
            .map(|m| m["_key"].as_str().unwrap())
            .collect();
        missing.sort();
        assert_eq!(missing, ["twin1", "twin2", "u7"]);
        assert_eq!(email.duplicates.count, 1);
        assert_eq!(
            email.duplicates.sample,
            [json!({"values": ["same@example.com"], "keys": ["twin1", "twin2"]})]
        );

        let city = check(&report, "by_city");
        assert_eq!(city.stale.count, 1);
        assert_eq!(city.stale.sample, [json!({"_key": "u1"})]);
        assert_eq!(city.missing.count, 0);
        assert_eq!(city.duplicates.count, 0);

        // A rebuild repairs the entries; the duplicate is still the
        // application's to settle.
        users.rebuild_all_indexes().ok();
        let after = users.check_indexes().unwrap();
        assert_eq!(check(&after, "by_city").stale.count, 0);
        assert_eq!(check(&after, "by_email").stale.count, 0);
        assert_eq!(check(&after, "by_email").duplicates.count, 1);
    }
}
