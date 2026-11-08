//! Process-wide cache of index definitions.
//!
//! Every index-metadata getter used to run a `prefix_iterator_cf` over the
//! `*_meta:` key range plus a serde_json parse per definition — and the write
//! path calls those getters ~5-6 times per single-document insert. This module
//! loads all definitions for a collection once and shares the snapshot until
//! an index (or the collection itself) is created or dropped.
//!
//! The cache is a process-global keyed by `(DB instance, column family)`
//! rather than a field on `Collection`: `StorageEngine` and `Database` each
//! keep their own handle cache and build independent `Collection` instances
//! for the same CF, so a per-instance cache could not be invalidated across
//! them. The DB half of the key is
//! the `Arc` data pointer, with a stored `Weak` to detect a dead-and-reused
//! allocation — several engines can be open in one process (tests, embedded
//! use) with colliding CF names like `"users"`.

use super::*;
use crate::storage::RocksDb as DB;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

/// One collection's index definitions of every type.
#[derive(Debug, Default)]
pub(crate) struct IndexMetaSnapshot {
    pub indexes: Vec<Index>,
    pub fulltext: Vec<FulltextIndex>,
    pub geo: Vec<GeoIndex>,
    pub ttl: Vec<TtlIndex>,
    /// Vector index configurations (`vec_meta:`). Every document write reads
    /// them, which used to open an iterator per write.
    pub vector: Vec<VectorIndexConfig>,
    /// The top-level fields any of the definitions above reads: see
    /// [`Collection::indexed_view`].
    pub read_fields: Vec<String>,
}

impl IndexMetaSnapshot {
    fn collect_read_fields(&mut self) {
        let paths = self
            .indexes
            .iter()
            .flat_map(|i| i.fields.iter())
            .chain(self.fulltext.iter().flat_map(|f| f.fields.iter()))
            .chain(self.geo.iter().map(|g| &g.field))
            .chain(self.ttl.iter().map(|t| &t.field))
            .chain(self.vector.iter().map(|v| &v.field))
            .chain(
                self.vector
                    .iter()
                    .filter_map(|v| v.embedding_source.as_ref()),
            );
        let mut fields: Vec<String> = Vec::new();
        for path in paths {
            let top = path.split('.').next().unwrap_or_default();
            if !fields.iter().any(|f| f == top) {
                fields.push(top.to_string());
            }
        }
        self.read_fields = fields;
    }
}

struct CacheEntry {
    /// The DB this snapshot was loaded from. Holding the `Weak` keeps that
    /// allocation from being reused, so an equal pointer is the same live
    /// instance.
    db: Weak<DB>,
    /// The column family's name; the key holds only its hash.
    cf_name: String,
    /// [`META_EPOCH`] when the snapshot was read.
    epoch: u64,
    snapshot: Arc<IndexMetaSnapshot>,
}

/// (DB instance, hash of the column family's name). Built without
/// allocating: this is looked up several times per document write.
type CacheKey = (usize, u64);

static INDEX_META_CACHE: Lazy<DashMap<CacheKey, CacheEntry>> = Lazy::new(DashMap::new);

/// Moved by every invalidation. A snapshot is served only under the epoch it
/// was read at, so one whose read raced an index change — loaded before the
/// change, stored after the invalidation — is never served.
static META_EPOCH: AtomicU64 = AtomicU64::new(0);

fn cache_key(db: &DB, cf_name: &str) -> CacheKey {
    (db as *const DB as usize, seahash::hash(cf_name.as_bytes()))
}

/// Drop the cached definitions for a column family. Must be called after
/// anything that creates or drops an index — or the CF itself, so a
/// same-name recreate cannot serve the previous incarnation's definitions.
pub(crate) fn invalidate_index_meta(db: &DB, cf_name: &str) {
    INDEX_META_CACHE.remove(&cache_key(db, cf_name));
    META_EPOCH.fetch_add(1, Ordering::AcqRel);
}

impl Collection {
    /// Cached index definitions for this collection. `None` when the column
    /// family no longer exists (mirrors the `cf_handle` checks the per-call
    /// scans used to do). Definitions are kept in `*_meta:` key order so
    /// "first matching index" tie-breaks stay identical to a direct scan.
    pub(crate) fn index_meta(&self) -> Option<Arc<IndexMetaSnapshot>> {
        let key = cache_key(&self.db, &self.name);
        let epoch = META_EPOCH.load(Ordering::Acquire);
        if let Some(entry) = INDEX_META_CACHE.get(&key) {
            // Reject an entry from another DB instance, another column
            // family whose name hashed the same, or an older epoch.
            if entry.epoch == epoch
                && Weak::as_ptr(&entry.db) == Arc::as_ptr(&self.db)
                && entry.cf_name == self.name
            {
                return Some(Arc::clone(&entry.snapshot));
            }
        }

        let cf = self.ks.handle(&self.db)?;
        let mut snapshot = IndexMetaSnapshot::default();

        macro_rules! load {
            ($prefix:expr, $target:expr) => {
                let prefix = $prefix.as_bytes();
                for (key, value) in self.db.prefix_iterator_ks(&cf, prefix).flatten() {
                    if !key.starts_with(prefix) {
                        break;
                    }
                    if let Ok(def) = serde_json::from_slice(&value) {
                        $target.push(def);
                    }
                }
            };
        }

        load!(IDX_META_PREFIX, snapshot.indexes);
        load!(FT_META_PREFIX, snapshot.fulltext);
        load!(GEO_META_PREFIX, snapshot.geo);
        load!(TTL_META_PREFIX, snapshot.ttl);
        load!(VEC_META_PREFIX, snapshot.vector);
        snapshot.collect_read_fields();

        let snapshot = Arc::new(snapshot);
        INDEX_META_CACHE.insert(
            key,
            CacheEntry {
                db: Arc::downgrade(&self.db),
                cf_name: self.name.clone(),
                epoch,
                snapshot: Arc::clone(&snapshot),
            },
        );
        Some(snapshot)
    }

    /// Invalidate this collection's cached index definitions.
    pub(crate) fn invalidate_index_meta(&self) {
        invalidate_index_meta(&self.db, &self.name);
    }

    /// `doc.to_value()` restricted to the top-level fields that some index,
    /// fulltext, geo, TTL or vector definition of `meta` reads (system
    /// fields included when one is read). Every entry, token and vector
    /// computed from it is the one the whole value gives — those all read
    /// field paths through `extract_field_value`, which only descends from
    /// the path's first segment — without copying the rest of the document.
    pub(crate) fn indexed_view(doc: &Document, meta: &IndexMetaSnapshot) -> Value {
        if !doc.data.is_object() {
            // `to_value` has its own answer for these.
            return doc.to_value();
        }
        let mut view = serde_json::Map::new();
        for field in &meta.read_fields {
            let value = match field.as_str() {
                "_key" => Some(Value::String(doc.key.clone())),
                "_id" => Some(Value::String(doc.id.clone())),
                "_rev" => Some(Value::String(doc.rev.clone())),
                "_created_at" => Some(Value::String(doc.created_at.to_rfc3339())),
                "_updated_at" => Some(Value::String(doc.updated_at.to_rfc3339())),
                other => doc.data.get(other).cloned(),
            };
            if let Some(value) = value {
                view.insert(field.clone(), value);
            }
        }
        Value::Object(view)
    }
}

/// Per collection, process-wide like the cache above: a write generation,
/// and the answer to "does this sparse index hold every document?" for the
/// generation it was computed at.
///
/// The generation is bumped by [`bump_write_generation`], which runs from
/// `Collection::invalidate_cached_queries` — after every document write
/// (`emit_change`), truncate and index DDL or rebuild, each already
/// committed. A reader loads the generation *before* counting, so a count
/// that races a write is stored under a generation the write then moves
/// past, and is recomputed on the next read instead of being trusted.
#[derive(Default)]
struct Completeness {
    generation: std::sync::atomic::AtomicU64,
    answers: DashMap<String, (u64, bool)>,
}

struct CompletenessEntry {
    db: Weak<DB>,
    /// The key holds only the name's hash.
    cf_name: String,
    state: Arc<Completeness>,
}

static COMPLETENESS: Lazy<DashMap<CacheKey, CompletenessEntry>> = Lazy::new(DashMap::new);

/// The live completeness state of a collection, created when absent or when
/// the stored one belongs to a DB that died (its address reused) or to a
/// column family whose name hashed the same.
fn completeness(db: &Arc<DB>, cf_name: &str) -> Arc<Completeness> {
    let key = cache_key(db, cf_name);
    if let Some(entry) = COMPLETENESS.get(&key) {
        if entry.cf_name == cf_name
            && entry
                .db
                .upgrade()
                .is_some_and(|live| Arc::ptr_eq(&live, db))
        {
            return Arc::clone(&entry.state);
        }
    }
    let state = Arc::new(Completeness::default());
    COMPLETENESS.insert(
        key,
        CompletenessEntry {
            db: Arc::downgrade(db),
            cf_name: cf_name.to_string(),
            state: Arc::clone(&state),
        },
    );
    state
}

/// Move a collection's write generation, dropping every cached answer.
/// Nothing to do when no answer was ever cached for it. (On a name-hash
/// collision this moves the other collection's generation: that only makes
/// it recount.)
pub(crate) fn bump_write_generation(db: &DB, cf_name: &str) {
    if let Some(entry) = COMPLETENESS.get(&cache_key(db, cf_name)) {
        entry
            .state
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
}

impl Collection {
    /// Whether `index` has an entry for every document. Always for a
    /// non-sparse index; for a sparse one, when no document has all its index
    /// fields null — its entry count equals [`Self::count`]. That count reads
    /// every key of the index, so the answer is cached until the collection's
    /// next write.
    pub(crate) fn index_holds_every_document(&self, index: &Index) -> bool {
        if !index.sparse {
            return true;
        }
        let state = completeness(&self.db, &self.name);
        let generation = state.generation.load(std::sync::atomic::Ordering::Acquire);
        if let Some(answer) = state.answers.get(&index.name) {
            if answer.0 == generation {
                return answer.1;
            }
        }
        let complete = self.index_entry_count(index) >= self.count();
        state
            .answers
            .insert(index.name.clone(), (generation, complete));
        complete
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageEngine;
    use serde_json::json;

    fn collection_with_every_kind(engine: &StorageEngine) -> Collection {
        engine.create_collection("kinds".to_string(), None).unwrap();
        let coll = engine.get_collection("kinds").unwrap();
        for (name, fields, kind, unique) in [
            (
                "by_customer",
                vec!["customer_id"],
                IndexType::Persistent,
                false,
            ),
            (
                "by_city",
                vec!["status", "address.city"],
                IndexType::Persistent,
                false,
            ),
            ("by_key", vec!["_key"], IndexType::Hash, false),
            (
                "by_created",
                vec!["_created_at"],
                IndexType::Persistent,
                false,
            ),
            ("by_email", vec!["email"], IndexType::Hash, true),
            ("note_grams", vec!["note"], IndexType::Trigram, false),
            (
                "missing",
                vec!["nowhere.deep"],
                IndexType::Persistent,
                false,
            ),
        ] {
            coll.create_index(
                name.to_string(),
                fields.into_iter().map(String::from).collect(),
                kind,
                unique,
            )
            .unwrap();
        }
        coll.create_fulltext_index("ft".to_string(), vec!["note".to_string()], None)
            .unwrap();
        coll.create_geo_index("geo".to_string(), "loc".to_string())
            .unwrap();
        coll.create_ttl_index("ttl".to_string(), "expires".to_string(), 60)
            .unwrap();
        coll
    }

    fn order(key: &str, status: &str) -> Document {
        Document::with_key(
            "kinds",
            key.to_string(),
            json!({
                "customer_id": "c00042",
                "status": status,
                "email": format!("{key}@example.com"),
                "address": {"city": "Lyon", "zip": "69001"},
                "items": [{"sku": "A", "qty": 2}, {"sku": "B", "qty": 1}],
                "note": "Leave the parcel with the concierge",
                "loc": {"lat": 45.76, "lon": 4.83},
                "expires": 1_900_000_000u64,
                "unindexed": {"deep": [1, 2, 3]},
            }),
        )
    }

    /// Index maintenance computed from the indexed view is what it computes
    /// from the whole document, for every kind of definition.
    #[test]
    fn indexed_view_gives_the_entries_of_the_whole_document() {
        let dir = tempfile::TempDir::new().unwrap();
        let engine = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
        let coll = collection_with_every_kind(&engine);
        let meta = coll.index_meta().unwrap();
        assert!(meta.read_fields.contains(&"address".to_string()));
        assert!(!meta.read_fields.contains(&"unindexed".to_string()));

        let doc = order("k1", "pending");
        let full = doc.to_value();
        let view = Collection::indexed_view(&doc, &meta);
        assert!(view.get("unindexed").is_none() && view.get("items").is_none());

        let entries = |v: &Value| coll.compute_index_entries_for_insert("k1", v).unwrap();
        assert_eq!(entries(&view), entries(&full));
        assert!(!entries(&full).0.is_empty() && !entries(&full).1.is_empty());
        assert_eq!(
            coll.compute_fulltext_entries_for_insert("k1", &view),
            coll.compute_fulltext_entries_for_insert("k1", &full)
        );
        assert_eq!(
            coll.compute_ttl_expiry_entries_for_insert("k1", &view),
            coll.compute_ttl_expiry_entries_for_insert("k1", &full)
        );
        assert_eq!(coll.unique_tokens(&view), coll.unique_tokens(&full));
        assert_eq!(coll.unique_tokens(&full).len(), 1);

        // An update diffed from the old version's view.
        let mut new_doc = doc.clone();
        new_doc.update(
            json!({"status": "shipped", "note": "Ring twice", "expires": 2_000_000_000u64}),
        );
        let new_full = new_doc.to_value();
        assert_eq!(
            coll.compute_index_entries_for_update("k1", &view, &new_full)
                .unwrap(),
            coll.compute_index_entries_for_update("k1", &full, &new_full)
                .unwrap()
        );
        assert_eq!(
            coll.compute_fulltext_entries_for_update("k1", &view, &new_full),
            coll.compute_fulltext_entries_for_update("k1", &full, &new_full)
        );
        assert_eq!(
            coll.compute_ttl_expiry_entries_for_update("k1", &view, &new_full),
            coll.compute_ttl_expiry_entries_for_update("k1", &full, &new_full)
        );

        // A document that is not an object is viewed as `to_value` sees it.
        let odd = Document::with_key("kinds", "k2".to_string(), json!([1, 2]));
        assert_eq!(Collection::indexed_view(&odd, &meta), odd.to_value());
    }

    /// A definition created or dropped after the snapshot was read is seen
    /// by the next write: the vector configurations included.
    #[test]
    fn snapshot_follows_index_changes() {
        let dir = tempfile::TempDir::new().unwrap();
        let engine = StorageEngine::new(dir.path().to_str().unwrap()).unwrap();
        engine.create_collection("snap".to_string(), None).unwrap();
        let coll = engine.get_collection("snap").unwrap();
        assert!(coll.index_meta().unwrap().read_fields.is_empty());

        coll.create_index(
            "a".to_string(),
            vec!["a".to_string()],
            IndexType::Hash,
            false,
        )
        .unwrap();
        assert_eq!(
            coll.index_meta().unwrap().read_fields,
            vec!["a".to_string()]
        );

        let config: VectorIndexConfig = serde_json::from_value(json!({
            "name": "emb", "field": "embedding", "dimension": 2
        }))
        .unwrap();
        coll.create_vector_index(config).unwrap();
        assert_eq!(coll.get_all_vector_index_configs().len(), 1);
        assert!(coll
            .index_meta()
            .unwrap()
            .read_fields
            .contains(&"embedding".to_string()));
        coll.drop_vector_index("emb").unwrap();
        assert!(coll.get_all_vector_index_configs().is_empty());

        coll.drop_index("a").unwrap();
        assert!(coll.index_meta().unwrap().read_fields.is_empty());
    }
}
