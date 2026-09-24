use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::geo::{haversine_distance, GeoIndex, GeoIndexStats, GeoPoint};
use serde_json::Value;

impl Collection {
    // ==================== Geo Index Operations ====================

    /// Get all geo index metadata
    pub fn get_all_geo_indexes(&self) -> Vec<GeoIndex> {
        // Empty when the column family is gone (dropped mid-operation): a
        // background caller such as the TTL worker must not panic (audit P11).
        self.index_meta().map(|m| m.geo.clone()).unwrap_or_default()
    }

    /// Get a geo index by name
    pub(crate) fn get_geo_index(&self, name: &str) -> Option<GeoIndex> {
        self.index_meta()?
            .geo
            .iter()
            .find(|i| i.name == name)
            .cloned()
    }

    /// Create a geospatial index
    pub fn create_geo_index(&self, name: String, field: String) -> DbResult<GeoIndexStats> {
        if self.get_geo_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!(
                "Geo Index '{}' already exists",
                name
            )));
        }

        let index = GeoIndex {
            name: name.clone(),
            field: field.clone(),
            precision: 6,
        };
        let index_bytes = serde_json::to_vec(&index)?;

        // Store metadata
        {
            let db = &self.db;
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(&cf, Self::geo_meta_key(&name), &index_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create geo index: {}", e))
                })?;
        }
        self.invalidate_index_meta();

        // Build index from existing documents. Same rule as the insert path
        // (`update_indexes_on_insert`): every non-null value at the (possibly
        // nested) field path is indexed. The previous backfill read
        // `doc[field]` — wrong for a nested path — and skipped anything that
        // was not a `{lat, ...}` object, so documents inserted before the
        // index existed were invisible to it while later ones were not.
        let docs = self.all();
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut count = 0;
        for doc in &docs {
            let doc_value = doc.to_value();
            let field_value = crate::storage::index::extract_field_value(&doc_value, &field);
            if !field_value.is_null() {
                let entry_key = Self::geo_entry_key(&name, &doc.key);
                let geo_data = serde_json::to_vec(&field_value)?;
                db.put_cf(&cf, entry_key, &geo_data).map_err(|e| {
                    DbError::InternalError(format!("Failed to build geo index: {}", e))
                })?;
                count += 1;
            }
        }

        Ok(GeoIndexStats {
            name,
            field,
            precision: 6,
            indexed_documents: count,
            geohash_buckets: 0,
        })
    }

    /// Drop a geo index
    pub fn drop_geo_index(&self, name: &str) -> DbResult<()> {
        if self.get_geo_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!(
                "Geo Index '{}' not found",
                name
            )));
        }

        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Delete metadata
        db.delete_cf(&cf, Self::geo_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop geo index: {}", e)))?;
        self.invalidate_index_meta();

        // Delete entries
        let prefix = format!("{}{}:", GEO_PREFIX, name);
        let iter = db.prefix_iterator_cf(&cf, prefix.as_bytes());

        for result in iter.flatten() {
            let (key, _) = result;
            if key.starts_with(prefix.as_bytes()) {
                db.delete_cf(&cf, &key).map_err(|e| {
                    DbError::InternalError(format!("Failed to drop geo index entry: {}", e))
                })?;
            } else {
                break;
            }
        }

        Ok(())
    }

    /// List all geo indexes
    pub fn list_geo_indexes(&self) -> Vec<GeoIndexStats> {
        self.get_all_geo_indexes()
            .iter()
            .map(|idx| {
                // Count entries
                let db = &self.db;
                let cf = db
                    .cf_handle(&self.name)
                    .expect("Column family should exist");
                let prefix = format!("{}{}:", GEO_PREFIX, idx.name);
                let count = db
                    .prefix_iterator_cf(&cf, prefix.as_bytes())
                    .take_while(|r| {
                        r.as_ref()
                            .is_ok_and(|(k, _)| k.starts_with(prefix.as_bytes()))
                    })
                    .count();

                GeoIndexStats {
                    name: idx.name.clone(),
                    field: idx.field.clone(),
                    precision: idx.precision,
                    indexed_documents: count,
                    geohash_buckets: 0,
                }
            })
            .collect()
    }

    /// Visit every entry of the geo index covering `field`, in document-key
    /// order, as `(doc_key, stored field value)`. `visit` returns `false` to
    /// stop early. `None` when no geo index covers `field` (or the column
    /// family is gone); otherwise the geo index's name.
    ///
    /// This is the primitive the SDBQL geo optimizer builds on: it reads the
    /// small index entries instead of whole documents.
    pub fn geo_index_scan(
        &self,
        field: &str,
        mut visit: impl FnMut(&str, &Value) -> bool,
    ) -> Option<String> {
        let index = self
            .get_all_geo_indexes()
            .into_iter()
            .find(|idx| idx.field == field)?;
        let db = &self.db;
        let cf = db.cf_handle(&self.name)?;
        let prefix = format!("{}{}:", GEO_PREFIX, index.name);
        for (key, value) in db.prefix_iterator_cf(&cf, prefix.as_bytes()).flatten() {
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            let Some(doc_key) = key
                .get(prefix.len()..)
                .and_then(|k| std::str::from_utf8(k).ok())
            else {
                continue;
            };
            if doc_key.is_empty() {
                continue;
            }
            let Ok(point_val) = serde_json::from_slice::<Value>(&value) else {
                continue;
            };
            if !visit(doc_key, &point_val) {
                break;
            }
        }
        Some(index.name)
    }

    /// Fetch documents by key, keeping `keys`' order and pairing each with its
    /// payload. Missing documents (deleted since the index was read) are
    /// skipped. O(n): no per-result search over the fetched set.
    fn fetch_in_order<T>(&self, keyed: Vec<(String, T)>) -> Vec<(Document, T)> {
        keyed
            .into_iter()
            .filter_map(|(key, extra)| self.get(&key).ok().map(|doc| (doc, extra)))
            .collect()
    }

    /// Find the `limit` documents nearest to a point, closest first.
    ///
    /// Keeps a bounded max-heap of the `limit` best candidates while scanning,
    /// so memory is O(limit) and the work O(n log limit), instead of
    /// collecting and sorting every entry and then re-attaching documents with
    /// a linear search per result (audit P11).
    pub fn geo_near(
        &self,
        field: &str,
        lat: f64,
        lon: f64,
        limit: usize,
    ) -> Option<Vec<(Document, f64)>> {
        use std::collections::BinaryHeap;

        let center = GeoPoint::new(lat, lon);
        // (distance, sequence, key): the sequence keeps ties in key order.
        let mut heap: BinaryHeap<(HeapDist, usize, String)> = BinaryHeap::new();
        let mut seq = 0usize;
        self.geo_index_scan(field, |doc_key, point_val| {
            if limit == 0 {
                return false;
            }
            if let Some(target) = GeoPoint::from_value(point_val) {
                let dist = haversine_distance(&center, &target);
                if dist.is_nan() {
                    return true;
                }
                let entry = (HeapDist(dist), seq, doc_key.to_string());
                seq += 1;
                if heap.len() < limit {
                    heap.push(entry);
                } else if heap.peek().is_some_and(|worst| entry < *worst) {
                    heap.pop();
                    heap.push(entry);
                }
            }
            true
        })?;

        let matches: Vec<(String, f64)> = heap
            .into_sorted_vec()
            .into_iter()
            .map(|(d, _, k)| (k, d.0))
            .collect();
        Some(self.fetch_in_order(matches))
    }

    /// Find documents within a radius (meters), in document-key order.
    pub fn geo_within(
        &self,
        field: &str,
        lat: f64,
        lon: f64,
        radius: f64,
    ) -> Option<Vec<(Document, f64)>> {
        let center = GeoPoint::new(lat, lon);
        let mut matches = Vec::new();
        self.geo_index_scan(field, |doc_key, point_val| {
            if let Some(target) = GeoPoint::from_value(point_val) {
                let dist = haversine_distance(&center, &target);
                if dist <= radius {
                    matches.push((doc_key.to_string(), dist));
                }
            }
            true
        })?;
        Some(self.fetch_in_order(matches))
    }
}

/// A distance with a total order for the nearest-neighbour heap (NaN never
/// reaches it).
#[derive(Debug, Clone, Copy, PartialEq)]
struct HeapDist(f64);

impl Eq for HeapDist {}

impl PartialOrd for HeapDist {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapDist {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&other.0)
    }
}
