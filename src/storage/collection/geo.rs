use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::geo::{haversine_distance, GeoIndex, GeoIndexStats, GeoPoint};
use serde_json::Value;

impl Collection {
    // ==================== Geo Index Operations ====================

    /// Get all geo index metadata
    pub fn get_all_geo_indexes(&self) -> Vec<GeoIndex> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let prefix = GEO_META_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        iter.filter_map(|result| {
            result.ok().and_then(|(key, value)| {
                if key.starts_with(prefix) {
                    serde_json::from_slice(&value).ok()
                } else {
                    None
                }
            })
        })
        .collect()
    }

    /// Get a geo index by name
    pub(crate) fn get_geo_index(&self, name: &str) -> Option<GeoIndex> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        db.get_cf(cf, Self::geo_meta_key(name))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
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
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::geo_meta_key(&name), &index_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create geo index: {}", e))
                })?;
        }

        // Build index from existing documents
        let docs = self.all();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let mut count = 0;
        for doc in &docs {
            let doc_value = doc.to_value();
            if let Some(val) =
                crate::storage::index::extract_field_value(&doc_value, &field).as_object()
            {
                // Check if it looks like a geo point
                if val.contains_key("lat") || val.contains_key("latitude") {
                    let entry_key = Self::geo_entry_key(&name, &doc.key);
                    let geo_data = serde_json::to_vec(&doc_value[&field])?;
                    db.put_cf(cf, entry_key, &geo_data).map_err(|e| {
                        DbError::InternalError(format!("Failed to build geo index: {}", e))
                    })?;
                    count += 1;
                }
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

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Delete metadata
        db.delete_cf(cf, Self::geo_meta_key(name)).map_err(|e| {
            DbError::InternalError(format!("Failed to drop geo index: {}", e))
        })?;

        // Delete entries
        let prefix = format!("{}{}:", GEO_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(prefix.as_bytes()) {
                    db.delete_cf(cf, &key).map_err(|e| {
                        DbError::InternalError(format!("Failed to drop geo index entry: {}", e))
                    })?;
                } else {
                    break;
                }
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
                let db = self.db.read().unwrap();
                let cf = db
                    .cf_handle(&self.name)
                    .expect("Column family should exist");
                let prefix = format!("{}{}:", GEO_PREFIX, idx.name);
                let count = db
                    .prefix_iterator_cf(cf, prefix.as_bytes())
                    .take_while(|r| r.as_ref().map_or(false, |(k, _)| k.starts_with(prefix.as_bytes())))
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

    /// Find documents near a point
    pub fn geo_near(
        &self,
        field: &str,
        lat: f64,
        lon: f64,
        limit: usize,
    ) -> Option<Vec<(Document, f64)>> {
        // Find index that covers this field
        let indexes = self.get_all_geo_indexes();
        let index = indexes.iter().find(|idx| idx.field == field)?;

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let prefix = format!("{}{}:", GEO_PREFIX, index.name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        let mut matches = Vec::new();

        for result in iter {
            if let Ok((key, value)) = result {
                if !key.starts_with(prefix.as_bytes()) {
                    break;
                }

                if let Ok(point_val) = serde_json::from_slice::<Value>(&value) {
                    if let Some(target) = GeoPoint::from_value(&point_val) {
                        let dist = haversine_distance(&GeoPoint::new(lat, lon), &target);
                        // No specific radius in request? Handler implies sorted nearest.
                        // We store all then sort. 
                        // Key format: geo:<name>:<doc_key>
                        let key_str = String::from_utf8_lossy(&key);
                        let doc_key = key_str.strip_prefix(&prefix).unwrap_or("");
                        if !doc_key.is_empty() {
                            matches.push((doc_key.to_string(), dist));
                        }
                    }
                }
            }
        }

        // Sort by distance
        matches.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        matches.truncate(limit);

        // Fetch documents
        let doc_keys: Vec<String> = matches.iter().map(|(k, _)| k.clone()).collect();
        let docs = self.get_many(&doc_keys); // Returns Vec<Document>

        // Map back to (Document, distance)
        // Order of docs might not match matches order (get_many is batch).
        // So we need to re-attach distance.
        let mut results = Vec::new();
        for (key, dist) in matches {
            if let Some(doc) = docs.iter().find(|d| d.key == key) {
                results.push((doc.clone(), dist));
            }
        }

        Some(results)
    }

    /// Find documents within a radius
    pub fn geo_within(
        &self,
        field: &str,
        lat: f64,
        lon: f64,
        radius: f64,
    ) -> Option<Vec<(Document, f64)>> {
         let indexes = self.get_all_geo_indexes();
        let index = indexes.iter().find(|idx| idx.field == field)?;

        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        let prefix = format!("{}{}:", GEO_PREFIX, index.name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        let mut matches = Vec::new();

        for result in iter {
             if let Ok((key, value)) = result {
                if !key.starts_with(prefix.as_bytes()) {
                    break;
                }

                if let Ok(point_val) = serde_json::from_slice::<Value>(&value) {
                    if let Some(target) = GeoPoint::from_value(&point_val) {
                         let dist = haversine_distance(&GeoPoint::new(lat, lon), &target);
                         if dist <= radius {
                             let key_str = String::from_utf8_lossy(&key);
                             let doc_key = key_str.strip_prefix(&prefix).unwrap_or("");
                             if !doc_key.is_empty() {
                                 matches.push((doc_key.to_string(), dist));
                             }
                         }
                    }
                }
            }
        }

        // Fetch documents and attach distance
        let doc_keys: Vec<String> = matches.iter().map(|(k, _)| k.clone()).collect();
        let docs = self.get_many(&doc_keys);

        let mut results = Vec::new();
        for (key, dist) in matches {
            if let Some(doc) = docs.iter().find(|d| d.key == key) {
                results.push((doc.clone(), dist));
            }
        }

        Some(results)
    }
}
