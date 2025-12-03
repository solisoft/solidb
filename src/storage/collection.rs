use std::sync::{Arc, RwLock};
use std::collections::{HashMap, HashSet};
use rocksdb::DB;
use serde_json::Value;

use crate::error::{DbError, DbResult};
use super::document::Document;
use super::index::{Index, IndexType, IndexStats, extract_field_value, generate_ngrams, tokenize, levenshtein_distance, FulltextMatch, NGRAM_SIZE};
use super::geo::{GeoIndex, GeoIndexStats};

/// Key prefixes for different data types
const DOC_PREFIX: &str = "doc:";
const IDX_PREFIX: &str = "idx:";
const IDX_META_PREFIX: &str = "idx_meta:";
const GEO_PREFIX: &str = "geo:";
const GEO_META_PREFIX: &str = "geo_meta:";
const FT_PREFIX: &str = "ft:";          // Fulltext n-gram entries
const FT_META_PREFIX: &str = "ft_meta:"; // Fulltext index metadata
const FT_TERM_PREFIX: &str = "ft_term:"; // Fulltext term → doc mapping

/// Fulltext index metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct FulltextIndex {
    name: String,
    field: String,
    min_length: usize,
}

/// Represents a collection of documents backed by RocksDB
#[derive(Clone)]
pub struct Collection {
    /// Collection name (column family name)
    pub name: String,
    /// RocksDB instance
    db: Arc<RwLock<DB>>,
}

impl std::fmt::Debug for Collection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collection")
            .field("name", &self.name)
            .finish()
    }
}

impl Collection {
    /// Create a new collection handle
    pub fn new(name: String, db: Arc<RwLock<DB>>) -> Self {
        Self { name, db }
    }

    /// Build a document key
    fn doc_key(key: &str) -> Vec<u8> {
        format!("{}{}", DOC_PREFIX, key).into_bytes()
    }

    /// Build an index metadata key
    fn idx_meta_key(index_name: &str) -> Vec<u8> {
        format!("{}{}", IDX_META_PREFIX, index_name).into_bytes()
    }

    /// Build an index entry key
    fn idx_entry_key(index_name: &str, value: &Value, doc_key: &str) -> Vec<u8> {
        let value_str = serde_json::to_string(value).unwrap_or_default();
        format!("{}{}:{}:{}", IDX_PREFIX, index_name, value_str, doc_key).into_bytes()
    }

    /// Build a geo index metadata key
    fn geo_meta_key(index_name: &str) -> Vec<u8> {
        format!("{}{}", GEO_META_PREFIX, index_name).into_bytes()
    }

    /// Build a geo index entry key
    fn geo_entry_key(index_name: &str, doc_key: &str) -> Vec<u8> {
        format!("{}{}:{}", GEO_PREFIX, index_name, doc_key).into_bytes()
    }

    /// Build a fulltext index metadata key
    fn ft_meta_key(index_name: &str) -> Vec<u8> {
        format!("{}{}", FT_META_PREFIX, index_name).into_bytes()
    }

    /// Build a fulltext n-gram entry key (ngram → doc_key)
    fn ft_ngram_key(index_name: &str, ngram: &str, doc_key: &str) -> Vec<u8> {
        format!("{}{}:{}:{}", FT_PREFIX, index_name, ngram, doc_key).into_bytes()
    }

    /// Build a fulltext term entry key (term → doc_key with position)
    fn ft_term_key(index_name: &str, term: &str, doc_key: &str) -> Vec<u8> {
        format!("{}{}:{}:{}", FT_TERM_PREFIX, index_name, term, doc_key).into_bytes()
    }

    // ==================== Document Operations ====================

    /// Insert a document into the collection
    pub fn insert(&self, mut data: Value) -> DbResult<Document> {
        // Extract or generate key
        let key = if let Some(obj) = data.as_object_mut() {
            if let Some(key_value) = obj.remove("_key") {
                if let Some(key_str) = key_value.as_str() {
                    key_str.to_string()
                } else {
                    return Err(DbError::InvalidDocument("_key must be a string".to_string()));
                }
            } else {
                uuid::Uuid::new_v4().to_string()
            }
        } else {
            uuid::Uuid::new_v4().to_string()
        };

        let doc = Document::with_key(&self.name, key.clone(), data);
        let doc_bytes = serde_json::to_vec(&doc)?;
        let doc_value = doc.to_value();

        // Store document
        {
            let db = self.db.read().unwrap();
            let cf = db.cf_handle(&self.name).expect("Column family should exist");
            db.put_cf(cf, Self::doc_key(&key), &doc_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to insert document: {}", e)))?;
        }

        // Update indexes
        self.update_indexes_on_insert(&key, &doc_value)?;

        // Update fulltext indexes
        self.update_fulltext_on_insert(&key, &doc_value)?;

        Ok(doc)
    }

    /// Get a document by key
    pub fn get(&self, key: &str) -> DbResult<Document> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        let bytes = db.get_cf(cf, Self::doc_key(key))
            .map_err(|e| DbError::InternalError(format!("Failed to get document: {}", e)))?
            .ok_or_else(|| DbError::DocumentNotFound(key.to_string()))?;

        let doc: Document = serde_json::from_slice(&bytes)?;
        Ok(doc)
    }

    /// Get multiple documents by keys
    pub fn get_many(&self, keys: &[String]) -> Vec<Document> {
        keys.iter()
            .filter_map(|k| self.get(k).ok())
            .collect()
    }

    /// Update a document
    pub fn update(&self, key: &str, data: Value) -> DbResult<Document> {
        // Get old document for index updates
        let old_doc = self.get(key)?;
        let old_value = old_doc.to_value();

        // Create updated document
        let mut doc = old_doc;
        doc.update(data);
        let new_value = doc.to_value();
        let doc_bytes = serde_json::to_vec(&doc)?;

        // Store updated document
        {
            let db = self.db.read().unwrap();
            let cf = db.cf_handle(&self.name).expect("Column family should exist");
            db.put_cf(cf, Self::doc_key(key), &doc_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to update document: {}", e)))?;
        }

        // Update indexes
        self.update_indexes_on_update(key, &old_value, &new_value)?;

        // Update fulltext indexes (delete old, insert new)
        self.update_fulltext_on_delete(key, &old_value)?;
        self.update_fulltext_on_insert(key, &new_value)?;

        Ok(doc)
    }

    /// Delete a document
    pub fn delete(&self, key: &str) -> DbResult<()> {
        // Get document for index cleanup
        let doc = self.get(key)?;
        let doc_value = doc.to_value();

        // Delete document
        {
            let db = self.db.read().unwrap();
            let cf = db.cf_handle(&self.name).expect("Column family should exist");
            db.delete_cf(cf, Self::doc_key(key))
                .map_err(|e| DbError::InternalError(format!("Failed to delete document: {}", e)))?;
        }

        // Update indexes
        self.update_indexes_on_delete(key, &doc_value)?;

        // Update fulltext indexes
        self.update_fulltext_on_delete(key, &doc_value)?;

        Ok(())
    }

    /// Get all documents
    pub fn all(&self) -> Vec<Document> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");
        let prefix = DOC_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(cf, prefix);

        iter.filter_map(|result| {
            result.ok().and_then(|(key, value)| {
                // Check if key starts with doc prefix
                if key.starts_with(prefix) {
                    serde_json::from_slice(&value).ok()
                } else {
                    None
                }
            })
        })
        .collect()
    }

    /// Get the number of documents
    pub fn count(&self) -> usize {
        self.all().len()
    }

    // ==================== Index Operations ====================

    /// Get all index metadata
    fn get_all_indexes(&self) -> Vec<Index> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");
        let prefix = IDX_META_PREFIX.as_bytes();
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

    /// Get an index by name
    fn get_index(&self, name: &str) -> Option<Index> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        db.get_cf(cf, Self::idx_meta_key(name))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Update indexes on document insert
    fn update_indexes_on_insert(&self, doc_key: &str, doc_value: &Value) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for index in indexes {
            let field_value = extract_field_value(doc_value, &index.field);
            if !field_value.is_null() {
                let entry_key = Self::idx_entry_key(&index.name, &field_value, doc_key);
                db.put_cf(cf, entry_key, doc_key.as_bytes())
                    .map_err(|e| DbError::InternalError(format!("Failed to update index: {}", e)))?;
            }
        }
        drop(db);

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for geo_index in geo_indexes {
            let field_value = extract_field_value(doc_value, &geo_index.field);
            if !field_value.is_null() {
                let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
                let geo_data = serde_json::to_vec(&field_value)?;
                db.put_cf(cf, entry_key, &geo_data)
                    .map_err(|e| DbError::InternalError(format!("Failed to update geo index: {}", e)))?;
            }
        }

        Ok(())
    }

    /// Update indexes on document update
    fn update_indexes_on_update(&self, doc_key: &str, old_value: &Value, new_value: &Value) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for index in indexes {
            let old_field = extract_field_value(old_value, &index.field);
            let new_field = extract_field_value(new_value, &index.field);

            // Remove old entry
            if !old_field.is_null() {
                let old_entry_key = Self::idx_entry_key(&index.name, &old_field, doc_key);
                db.delete_cf(cf, old_entry_key)
                    .map_err(|e| DbError::InternalError(format!("Failed to update index: {}", e)))?;
            }

            // Add new entry
            if !new_field.is_null() {
                let new_entry_key = Self::idx_entry_key(&index.name, &new_field, doc_key);
                db.put_cf(cf, new_entry_key, doc_key.as_bytes())
                    .map_err(|e| DbError::InternalError(format!("Failed to update index: {}", e)))?;
            }
        }
        drop(db);

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for geo_index in geo_indexes {
            let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
            let new_field = extract_field_value(new_value, &geo_index.field);

            if !new_field.is_null() {
                let geo_data = serde_json::to_vec(&new_field)?;
                db.put_cf(cf, entry_key, &geo_data)
                    .map_err(|e| DbError::InternalError(format!("Failed to update geo index: {}", e)))?;
            } else {
                db.delete_cf(cf, entry_key)
                    .map_err(|e| DbError::InternalError(format!("Failed to update geo index: {}", e)))?;
            }
        }

        Ok(())
    }

    /// Update indexes on document delete
    fn update_indexes_on_delete(&self, doc_key: &str, doc_value: &Value) -> DbResult<()> {
        let indexes = self.get_all_indexes();
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for index in indexes {
            let field_value = extract_field_value(doc_value, &index.field);
            if !field_value.is_null() {
                let entry_key = Self::idx_entry_key(&index.name, &field_value, doc_key);
                db.delete_cf(cf, entry_key)
                    .map_err(|e| DbError::InternalError(format!("Failed to update index: {}", e)))?;
            }
        }
        drop(db);

        // Update geo indexes
        let geo_indexes = self.get_all_geo_indexes();
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for geo_index in geo_indexes {
            let entry_key = Self::geo_entry_key(&geo_index.name, doc_key);
            db.delete_cf(cf, entry_key)
                .map_err(|e| DbError::InternalError(format!("Failed to update geo index: {}", e)))?;
        }

        Ok(())
    }

    /// Create an index on a field
    pub fn create_index(&self, name: String, field: String, index_type: IndexType, unique: bool) -> DbResult<IndexStats> {
        // Check if index already exists
        if self.get_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!("Index '{}' already exists", name)));
        }

        // Create index metadata
        let index = Index::new(name.clone(), field.clone(), index_type.clone(), unique);
        let index_bytes = serde_json::to_vec(&index)?;

        // Store index metadata and build index
        {
            let db = self.db.read().unwrap();
            let cf = db.cf_handle(&self.name).expect("Column family should exist");
            db.put_cf(cf, Self::idx_meta_key(&name), &index_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to create index: {}", e)))?;
        }

        // Build index from existing documents
        let docs = self.all();
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for doc in &docs {
            let doc_value = doc.to_value();
            let field_value = extract_field_value(&doc_value, &field);
            if !field_value.is_null() {
                let entry_key = Self::idx_entry_key(&name, &field_value, &doc.key);
                db.put_cf(cf, entry_key, doc.key.as_bytes())
                    .map_err(|e| DbError::InternalError(format!("Failed to build index: {}", e)))?;
            }
        }

        Ok(IndexStats {
            name,
            field,
            index_type,
            unique,
            unique_values: docs.len(),
            indexed_documents: docs.len(),
        })
    }

    /// Drop an index
    pub fn drop_index(&self, name: &str) -> DbResult<()> {
        if self.get_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!("Index '{}' not found", name)));
        }

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        // Delete index metadata
        db.delete_cf(cf, Self::idx_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop index: {}", e)))?;

        // Delete all index entries
        let prefix = format!("{}{}:", IDX_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(prefix.as_bytes()) {
                    db.delete_cf(cf, &key)
                        .map_err(|e| DbError::InternalError(format!("Failed to drop index entry: {}", e)))?;
                } else {
                    break;
                }
            }
        }

        Ok(())
    }

    /// List all indexes
    pub fn list_indexes(&self) -> Vec<IndexStats> {
        self.get_all_indexes()
            .iter()
            .filter_map(|idx| self.get_index_stats(&idx.name))
            .collect()
    }

    /// Get index statistics
    fn get_index_stats(&self, name: &str) -> Option<IndexStats> {
        let index = self.get_index(name)?;

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        // Count entries
        let prefix = format!("{}{}:", IDX_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());
        let count = iter.filter(|r| {
            r.as_ref().map(|(k, _)| k.starts_with(prefix.as_bytes())).unwrap_or(false)
        }).count();

        Some(IndexStats {
            name: index.name,
            field: index.field,
            index_type: index.index_type,
            unique: index.unique,
            unique_values: count,
            indexed_documents: count,
        })
    }

    /// Get an index for a field
    pub fn get_index_for_field(&self, field: &str) -> Option<Index> {
        self.get_all_indexes()
            .into_iter()
            .find(|idx| idx.field == field)
    }

    /// Lookup documents using index (equality)
    pub fn index_lookup_eq(&self, field: &str, value: &Value) -> Option<Vec<Document>> {
        let index = self.get_index_for_field(field)?;

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        let prefix = format!("{}{}:{}:", IDX_PREFIX, index.name, serde_json::to_string(value).ok()?);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        let keys: Vec<String> = iter
            .filter_map(|r| r.ok())
            .filter(|(k, _)| k.starts_with(prefix.as_bytes()))
            .filter_map(|(_, v)| String::from_utf8(v.to_vec()).ok())
            .collect();
        drop(db);

        Some(self.get_many(&keys))
    }

    /// Lookup documents using index (greater than)
    pub fn index_lookup_gt(&self, _field: &str, _value: &Value) -> Option<Vec<Document>> {
        // For simplicity, fall back to scan for range queries
        None
    }

    /// Lookup documents using index (greater than or equal)
    pub fn index_lookup_gte(&self, _field: &str, _value: &Value) -> Option<Vec<Document>> {
        None
    }

    /// Lookup documents using index (less than)
    pub fn index_lookup_lt(&self, _field: &str, _value: &Value) -> Option<Vec<Document>> {
        None
    }

    /// Lookup documents using index (less than or equal)
    pub fn index_lookup_lte(&self, _field: &str, _value: &Value) -> Option<Vec<Document>> {
        None
    }

    /// Get documents sorted by indexed field
    pub fn index_sorted(&self, _field: &str, _ascending: bool) -> Option<Vec<Document>> {
        None
    }

    // ==================== Geo Index Operations ====================

    /// Get all geo index metadata
    fn get_all_geo_indexes(&self) -> Vec<GeoIndex> {
        let db = self.db.read().unwrap();
        let cf = match db.cf_handle(&self.name) {
            Some(cf) => cf,
            None => return vec![],
        };
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
    fn get_geo_index(&self, name: &str) -> Option<GeoIndex> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        db.get_cf(cf, Self::geo_meta_key(name))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Create a geo index on a field
    pub fn create_geo_index(&self, name: String, field: String) -> DbResult<GeoIndexStats> {
        if self.get_geo_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!("Geo index '{}' already exists", name)));
        }

        let geo_index = GeoIndex::new(name.clone(), field.clone());
        let index_bytes = serde_json::to_vec(&geo_index)?;

        // Store geo index metadata
        {
            let db = self.db.read().unwrap();
            let cf = db.cf_handle(&self.name).expect("Column family should exist");
            db.put_cf(cf, Self::geo_meta_key(&name), &index_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to create geo index: {}", e)))?;
        }

        // Build index from existing documents
        let docs = self.all();
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for doc in &docs {
            let doc_value = doc.to_value();
            let field_value = extract_field_value(&doc_value, &field);
            if !field_value.is_null() {
                let entry_key = Self::geo_entry_key(&name, &doc.key);
                let geo_data = serde_json::to_vec(&field_value)?;
                db.put_cf(cf, entry_key, &geo_data)
                    .map_err(|e| DbError::InternalError(format!("Failed to build geo index: {}", e)))?;
            }
        }

        Ok(geo_index.stats())
    }

    /// Drop a geo index
    pub fn drop_geo_index(&self, name: &str) -> DbResult<()> {
        if self.get_geo_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!("Geo index '{}' not found", name)));
        }

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        // Delete geo index metadata
        db.delete_cf(cf, Self::geo_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop geo index: {}", e)))?;

        // Delete all geo index entries
        let prefix = format!("{}{}:", GEO_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(prefix.as_bytes()) {
                    db.delete_cf(cf, &key)
                        .map_err(|e| DbError::InternalError(format!("Failed to drop geo index entry: {}", e)))?;
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
            .map(|idx| idx.stats())
            .collect()
    }

    /// Find documents near a point
    pub fn geo_near(&self, field: &str, lat: f64, lon: f64, limit: usize) -> Option<Vec<(Document, f64)>> {
        use super::geo::{GeoPoint, haversine_distance};

        let geo_index = self.get_all_geo_indexes()
            .into_iter()
            .find(|idx| idx.field == field)?;

        let target = GeoPoint::new(lat, lon);

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        let prefix = format!("{}{}:", GEO_PREFIX, geo_index.name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        let mut results: Vec<(String, f64)> = iter
            .filter_map(|r| r.ok())
            .filter(|(k, _)| k.starts_with(prefix.as_bytes()))
            .filter_map(|(key, value)| {
                let key_str = String::from_utf8(key.to_vec()).ok()?;
                let doc_key = key_str.strip_prefix(&prefix)?;
                let geo_value: Value = serde_json::from_slice(&value).ok()?;
                let point = GeoPoint::from_value(&geo_value)?;
                let dist = haversine_distance(&target, &point);
                Some((doc_key.to_string(), dist))
            })
            .collect();
        drop(db);

        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        let docs: Vec<(Document, f64)> = results
            .into_iter()
            .filter_map(|(key, dist)| {
                self.get(&key).ok().map(|doc| (doc, dist))
            })
            .collect();

        Some(docs)
    }

    /// Find documents within a radius of a point
    pub fn geo_within(&self, field: &str, lat: f64, lon: f64, radius_meters: f64) -> Option<Vec<(Document, f64)>> {
        use super::geo::{GeoPoint, haversine_distance};

        let geo_index = self.get_all_geo_indexes()
            .into_iter()
            .find(|idx| idx.field == field)?;

        let target = GeoPoint::new(lat, lon);

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        let prefix = format!("{}{}:", GEO_PREFIX, geo_index.name);
        let iter = db.prefix_iterator_cf(cf, prefix.as_bytes());

        let mut results: Vec<(String, f64)> = iter
            .filter_map(|r| r.ok())
            .filter(|(k, _)| k.starts_with(prefix.as_bytes()))
            .filter_map(|(key, value)| {
                let key_str = String::from_utf8(key.to_vec()).ok()?;
                let doc_key = key_str.strip_prefix(&prefix)?;
                let geo_value: Value = serde_json::from_slice(&value).ok()?;
                let point = GeoPoint::from_value(&geo_value)?;
                let dist = haversine_distance(&target, &point);
                if dist <= radius_meters {
                    Some((doc_key.to_string(), dist))
                } else {
                    None
                }
            })
            .collect();
        drop(db);

        results.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        let docs: Vec<(Document, f64)> = results
            .into_iter()
            .filter_map(|(key, dist)| {
                self.get(&key).ok().map(|doc| (doc, dist))
            })
            .collect();

        Some(docs)
    }

    // ==================== Fulltext Index Operations ====================

    /// Get all fulltext index metadata
    fn get_all_fulltext_indexes(&self) -> Vec<FulltextIndex> {
        let db = self.db.read().unwrap();
        let cf = match db.cf_handle(&self.name) {
            Some(cf) => cf,
            None => return vec![],
        };
        let prefix = FT_META_PREFIX.as_bytes();
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

    /// Get a fulltext index by name
    fn get_fulltext_index(&self, name: &str) -> Option<FulltextIndex> {
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;
        db.get_cf(cf, Self::ft_meta_key(name))
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Get fulltext index for a field
    pub fn get_fulltext_index_for_field(&self, field: &str) -> Option<String> {
        self.get_all_fulltext_indexes()
            .into_iter()
            .find(|idx| idx.field == field)
            .map(|idx| idx.name)
    }

    /// Create a fulltext index on a field
    pub fn create_fulltext_index(&self, name: String, field: String, min_length: Option<usize>) -> DbResult<IndexStats> {
        if self.get_fulltext_index(&name).is_some() {
            return Err(DbError::InvalidDocument(format!("Fulltext index '{}' already exists", name)));
        }

        let min_len = min_length.unwrap_or(3);
        let ft_index = FulltextIndex {
            name: name.clone(),
            field: field.clone(),
            min_length: min_len,
        };
        let index_bytes = serde_json::to_vec(&ft_index)?;

        // Store fulltext index metadata
        {
            let db = self.db.read().unwrap();
            let cf = db.cf_handle(&self.name).expect("Column family should exist");
            db.put_cf(cf, Self::ft_meta_key(&name), &index_bytes)
                .map_err(|e| DbError::InternalError(format!("Failed to create fulltext index: {}", e)))?;
        }

        // Build index from existing documents
        let docs = self.all();
        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for doc in &docs {
            let doc_value = doc.to_value();
            let field_value = extract_field_value(&doc_value, &field);
            if let Some(text) = field_value.as_str() {
                // Index terms
                let terms = tokenize(text);
                for term in &terms {
                    if term.len() >= min_len {
                        let term_key = Self::ft_term_key(&name, term, &doc.key);
                        db.put_cf(cf, term_key, doc.key.as_bytes())
                            .map_err(|e| DbError::InternalError(format!("Failed to build fulltext index: {}", e)))?;
                    }
                }

                // Index n-grams for fuzzy matching
                let ngrams = generate_ngrams(text, NGRAM_SIZE);
                for ngram in &ngrams {
                    let ngram_key = Self::ft_ngram_key(&name, ngram, &doc.key);
                    db.put_cf(cf, ngram_key, doc.key.as_bytes())
                        .map_err(|e| DbError::InternalError(format!("Failed to build fulltext index: {}", e)))?;
                }
            }
        }

        Ok(IndexStats {
            name,
            field,
            index_type: IndexType::Fulltext,
            unique: false,
            unique_values: docs.len(),
            indexed_documents: docs.len(),
        })
    }

    /// Drop a fulltext index
    pub fn drop_fulltext_index(&self, name: &str) -> DbResult<()> {
        if self.get_fulltext_index(name).is_none() {
            return Err(DbError::InvalidDocument(format!("Fulltext index '{}' not found", name)));
        }

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        // Delete fulltext index metadata
        db.delete_cf(cf, Self::ft_meta_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to drop fulltext index: {}", e)))?;

        // Delete all n-gram entries
        let ngram_prefix = format!("{}{}:", FT_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, ngram_prefix.as_bytes());
        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(ngram_prefix.as_bytes()) {
                    db.delete_cf(cf, &key)
                        .map_err(|e| DbError::InternalError(format!("Failed to drop fulltext index: {}", e)))?;
                } else {
                    break;
                }
            }
        }

        // Delete all term entries
        let term_prefix = format!("{}{}:", FT_TERM_PREFIX, name);
        let iter = db.prefix_iterator_cf(cf, term_prefix.as_bytes());
        for result in iter {
            if let Ok((key, _)) = result {
                if key.starts_with(term_prefix.as_bytes()) {
                    db.delete_cf(cf, &key)
                        .map_err(|e| DbError::InternalError(format!("Failed to drop fulltext index: {}", e)))?;
                } else {
                    break;
                }
            }
        }

        Ok(())
    }

    /// Update fulltext indexes on document insert
    fn update_fulltext_on_insert(&self, doc_key: &str, doc_value: &Value) -> DbResult<()> {
        let ft_indexes = self.get_all_fulltext_indexes();
        if ft_indexes.is_empty() {
            return Ok(());
        }

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for ft_index in ft_indexes {
            let field_value = extract_field_value(doc_value, &ft_index.field);
            if let Some(text) = field_value.as_str() {
                // Index terms
                let terms = tokenize(text);
                for term in &terms {
                    if term.len() >= ft_index.min_length {
                        let term_key = Self::ft_term_key(&ft_index.name, term, doc_key);
                        db.put_cf(cf, term_key, doc_key.as_bytes())
                            .map_err(|e| DbError::InternalError(format!("Failed to update fulltext index: {}", e)))?;
                    }
                }

                // Index n-grams
                let ngrams = generate_ngrams(text, NGRAM_SIZE);
                for ngram in &ngrams {
                    let ngram_key = Self::ft_ngram_key(&ft_index.name, ngram, doc_key);
                    db.put_cf(cf, ngram_key, doc_key.as_bytes())
                        .map_err(|e| DbError::InternalError(format!("Failed to update fulltext index: {}", e)))?;
                }
            }
        }

        Ok(())
    }

    /// Update fulltext indexes on document delete
    fn update_fulltext_on_delete(&self, doc_key: &str, doc_value: &Value) -> DbResult<()> {
        let ft_indexes = self.get_all_fulltext_indexes();
        if ft_indexes.is_empty() {
            return Ok(());
        }

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name).expect("Column family should exist");

        for ft_index in ft_indexes {
            let field_value = extract_field_value(doc_value, &ft_index.field);
            if let Some(text) = field_value.as_str() {
                // Remove terms
                let terms = tokenize(text);
                for term in &terms {
                    if term.len() >= ft_index.min_length {
                        let term_key = Self::ft_term_key(&ft_index.name, term, doc_key);
                        let _ = db.delete_cf(cf, term_key);
                    }
                }

                // Remove n-grams
                let ngrams = generate_ngrams(text, NGRAM_SIZE);
                for ngram in &ngrams {
                    let ngram_key = Self::ft_ngram_key(&ft_index.name, ngram, doc_key);
                    let _ = db.delete_cf(cf, ngram_key);
                }
            }
        }

        Ok(())
    }

    /// Fulltext search with fuzzy matching
    /// Returns documents matching the query with relevance scores
    pub fn fulltext_search(&self, field: &str, query: &str, max_distance: usize) -> Option<Vec<FulltextMatch>> {
        let ft_index = self.get_all_fulltext_indexes()
            .into_iter()
            .find(|idx| idx.field == field)?;

        let query_terms = tokenize(query);
        let query_ngrams = generate_ngrams(query, NGRAM_SIZE);

        let db = self.db.read().unwrap();
        let cf = db.cf_handle(&self.name)?;

        // Step 1: Find candidate documents using n-gram matching
        let mut candidate_scores: HashMap<String, (usize, HashSet<String>)> = HashMap::new();

        // Search for exact term matches first
        for term in &query_terms {
            let term_prefix = format!("{}{}:{}:", FT_TERM_PREFIX, ft_index.name, term);
            let iter = db.prefix_iterator_cf(cf, term_prefix.as_bytes());

            for result in iter {
                if let Ok((key, _)) = result {
                    if key.starts_with(term_prefix.as_bytes()) {
                        let key_str = String::from_utf8(key.to_vec()).ok()?;
                        if let Some(doc_key) = key_str.strip_prefix(&term_prefix) {
                            let entry = candidate_scores.entry(doc_key.to_string()).or_insert((0, HashSet::new()));
                            entry.0 += 10; // High score for exact match
                            entry.1.insert(term.clone());
                        }
                    } else {
                        break;
                    }
                }
            }
        }

        // Search using n-grams for fuzzy matching
        for ngram in &query_ngrams {
            let ngram_prefix = format!("{}{}:{}:", FT_PREFIX, ft_index.name, ngram);
            let iter = db.prefix_iterator_cf(cf, ngram_prefix.as_bytes());

            for result in iter {
                if let Ok((key, _)) = result {
                    if key.starts_with(ngram_prefix.as_bytes()) {
                        let key_str = String::from_utf8(key.to_vec()).ok()?;
                        if let Some(doc_key) = key_str.strip_prefix(&ngram_prefix) {
                            let entry = candidate_scores.entry(doc_key.to_string()).or_insert((0, HashSet::new()));
                            entry.0 += 1; // Lower score for n-gram match
                        }
                    } else {
                        break;
                    }
                }
            }
        }
        drop(db);

        // Step 2: Verify candidates with Levenshtein distance and compute final scores
        let mut results: Vec<FulltextMatch> = Vec::new();

        for (doc_key, (ngram_score, matched_terms)) in candidate_scores.into_iter() {
            if let Ok(doc) = self.get(&doc_key) {
                let doc_value = doc.to_value();
                let field_value = extract_field_value(&doc_value, field);

                if let Some(doc_text) = field_value.as_str() {
                    let doc_terms = tokenize(doc_text);
                    let mut total_score = ngram_score as f64;
                    let mut all_matched: HashSet<String> = matched_terms;

                    // Check fuzzy matches
                    for query_term in &query_terms {
                        for doc_term in &doc_terms {
                            let distance = levenshtein_distance(query_term, doc_term);
                            if distance <= max_distance {
                                // Score based on distance (closer = higher score)
                                let match_score = ((max_distance - distance + 1) as f64) * 5.0;
                                total_score += match_score;
                                all_matched.insert(doc_term.clone());
                            }
                        }
                    }

                    if !all_matched.is_empty() || total_score > 0.0 {
                        // Normalize score
                        let final_score = total_score / (query_terms.len().max(1) as f64);

                        results.push(FulltextMatch {
                            doc_key: doc_key.clone(),
                            score: final_score,
                            matched_terms: all_matched.into_iter().collect(),
                        });
                    }
                }
            }
        }

        // Sort by score descending
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        Some(results)
    }

    /// List all fulltext indexes
    pub fn list_fulltext_indexes(&self) -> Vec<IndexStats> {
        self.get_all_fulltext_indexes()
            .iter()
            .map(|idx| IndexStats {
                name: idx.name.clone(),
                field: idx.field.clone(),
                index_type: IndexType::Fulltext,
                unique: false,
                unique_values: 0,
                indexed_documents: 0,
            })
            .collect()
    }
}
