//! A periodic look at what a cluster cannot promise: foreign keys and
//! unique indexes.
//!
//! A constraint is checked by the node that takes a write; the other nodes
//! apply it as it was written. Two nodes that accept conflicting writes at
//! the same moment — a child inserted on one while its parent is deleted
//! on the other, or the same unique value inserted on both — leave every
//! node with an orphan, or with two documents sharing the value (the
//! decision of card #851: multi-master stays, constraints are detected).
//! The checks that find them exist on demand
//! (`Collection::foreign_key_orphans`, `Collection::check_indexes`); this
//! runs them on a schedule and reports, in the log and on `/metrics`. It
//! repairs nothing: which document to keep is the application's decision.
//!
//! Off unless `SOLIDB_CONSTRAINT_CHECK_SECS` is set: a pass reads every
//! collection that carries a foreign key or a unique index.

use std::sync::RwLock;

use super::collection::references::ForeignKeySpec;
use super::StorageEngine;

/// What the last pass found in one collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub database: String,
    pub collection: String,
    /// Documents whose foreign key names a parent that is not there.
    pub orphans: usize,
    /// Unique values held by more than one document.
    pub duplicates: usize,
}

static LAST: RwLock<Vec<Found>> = RwLock::new(Vec::new());

/// Seconds between two passes (`SOLIDB_CONSTRAINT_CHECK_SECS`); 0, the
/// default: no pass.
pub fn interval_secs() -> u64 {
    std::env::var("SOLIDB_CONSTRAINT_CHECK_SECS")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0)
}

/// What the last pass found: one entry per collection that holds an
/// orphan or a duplicate. Empty before the first pass.
pub fn last() -> Vec<Found> {
    LAST.read().map(|found| found.clone()).unwrap_or_default()
}

/// One pass over every collection that carries a foreign key or a unique
/// index. The findings replace those of the pass before, and each is
/// logged as a warning.
pub fn sweep(storage: &StorageEngine) -> Vec<Found> {
    let mut all = Vec::new();
    for (db_name, coll_names) in storage.collections_grouped() {
        let Ok(db) = storage.get_database(&db_name) else {
            continue;
        };
        for coll_name in coll_names {
            // A dropped collection, or one that breaks under the check,
            // must not end the pass.
            let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let collection = db.system_collection(&coll_name).ok()?;
                let mut orphans = 0;
                for key in collection.foreign_keys() {
                    let spec = ForeignKeySpec {
                        field: key.field.clone(),
                        references: key.references.clone(),
                        on_delete: key.on_delete,
                        by: Some(key.by),
                        name: Some(key.name.clone()),
                    };
                    if let Ok(found) = collection.foreign_key_orphans(&spec) {
                        orphans += found.count;
                    }
                }
                let unique = collection
                    .index_meta()
                    .is_some_and(|meta| meta.indexes.iter().any(|index| index.unique));
                let duplicates = if unique {
                    collection.check_indexes().map_or(0, |report| {
                        report
                            .indexes
                            .iter()
                            .map(|index| index.duplicates.count)
                            .sum()
                    })
                } else {
                    0
                };
                Some((orphans, duplicates))
            }));
            let Ok(Some((orphans, duplicates))) = checked else {
                continue;
            };
            if orphans > 0 || duplicates > 0 {
                tracing::warn!(
                    "Constraint check: {}.{} holds {} orphan(s) of a foreign key and {} unique \
                     value(s) held by more than one document; see POST …/foreign-keys/check and \
                     POST …/index/{{collection}}/check",
                    db_name,
                    coll_name,
                    orphans,
                    duplicates
                );
                all.push(Found {
                    database: db_name.clone(),
                    collection: coll_name,
                    orphans,
                    duplicates,
                });
            }
        }
    }
    all.sort_by(|a, b| (&a.database, &a.collection).cmp(&(&b.database, &b.collection)));
    if let Ok(mut last) = LAST.write() {
        *last = all.clone();
    }
    all
}

/// The findings as `/metrics` lines.
pub fn metrics() -> String {
    let found = last();
    let mut out = String::new();
    for (name, help, value) in [
        (
            "solidb_fk_orphans",
            "Documents whose foreign key names a missing parent, at the last constraint check",
            (|f: &Found| f.orphans) as fn(&Found) -> usize,
        ),
        (
            "solidb_unique_duplicates",
            "Unique values held by more than one document, at the last constraint check",
            (|f: &Found| f.duplicates) as fn(&Found) -> usize,
        ),
    ] {
        out.push_str(&format!(
            "# HELP {} {}\n# TYPE {} gauge\n",
            name, help, name
        ));
        for f in found.iter().filter(|f| value(f) > 0) {
            out.push_str(&format!(
                "{}{{database=\"{}\",collection=\"{}\"}} {}\n",
                name,
                f.database,
                f.collection,
                value(f)
            ));
        }
        out.push('\n');
    }
    out
}
