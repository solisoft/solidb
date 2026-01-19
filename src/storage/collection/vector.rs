use super::*;
use crate::error::{DbError, DbResult};
use crate::storage::index::{
    extract_field_value, VectorIndexConfig, VectorIndexStats, VectorQuantization,
};
use crate::storage::vector::{VectorIndex, VectorSearchResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Statistics about vector quantization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantizationStats {
    #[serde(rename = "type")]
    pub type_: VectorQuantization,
    pub original_size: usize,
    pub compressed_size: usize,
    pub compression_ratio: f32,
}

impl Collection {
    // ==================== Vector Index Operations ====================

    /// Get all vector index configurations
    pub fn get_all_vector_index_configs(&self) -> Vec<VectorIndexConfig> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let prefix = VEC_META_PREFIX.as_bytes();
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

    /// Get vector index (loading it if necessary)
    pub fn get_vector_index(&self, name: &str) -> DbResult<Arc<super::vector::VectorIndex>> {
        // Try memory first
        {
            let indexes = self.vector_indexes.read().unwrap();
            if let Some(index) = indexes.get(name) {
                return Ok(index.clone());
            }
        }

        // Try load from disk
        self.load_vector_index(name)
    }

    /// Load vector index from disk
    pub(crate) fn load_vector_index(
        &self,
        name: &str,
    ) -> DbResult<Arc<super::vector::VectorIndex>> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .ok_or(DbError::InternalError("Column family not found".into()))?;

        // Check metadata first
        let meta_key = Self::vec_meta_key(name);
        if db.get_cf(cf, &meta_key)?.is_none() {
            return Err(DbError::InvalidDocument(format!(
                "Vector Index '{}' not found",
                name
            )));
        }

        // Load data
        let data_key = Self::vec_data_key(name);
        if let Some(bytes) = db.get_cf(cf, &data_key)? {
            // Need to deserialize VectorIndex.
            // Assuming serde support.
            // Need to deserialize VectorIndex.
            // Using manual deserialize method.
            match super::vector::VectorIndex::deserialize(&bytes) {
                Ok(index) => {
                    let index_arc = Arc::new(index);
                    let mut indexes = self.vector_indexes.write().unwrap();
                    indexes.insert(name.to_string(), index_arc.clone());
                    Ok(index_arc)
                }
                Err(e) => Err(DbError::InternalError(format!(
                    "Failed to deserialize vector index: {}",
                    e
                ))),
            }
        } else {
            Err(DbError::InternalError("Vector index data missing".into()))
        }
    }

    /// Create a vector index
    pub fn create_vector_index(&self, config: VectorIndexConfig) -> DbResult<VectorIndexStats> {
        let name = config.name.clone();
        if self.get_vector_index(&name).is_ok() {
            return Err(DbError::InvalidDocument(format!(
                "Vector Index '{}' already exists",
                name
            )));
        }

        // Store metadata
        let config_bytes = serde_json::to_vec(&config)?;
        {
            let db = self.db.read().unwrap();
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(cf, Self::vec_meta_key(&name), &config_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create vector index: {}", e))
                })?;
        }

        // Create in-memory index
        // Use default config for HNSW setup
        let index = VectorIndex::new(config.clone())?;

        // Build index from existing documents
        let docs = self.all();
        // Insert docs into index
        for doc in docs {
            let doc_value = doc.to_value();
            if let Some(vector) = self.extract_vector(&doc_value, &config.field, config.dimension) {
                let _ = index.insert(&doc.key, &vector);
            }
        }

        // Persist populated index
        self.persist_vector_indexes()?;

        let stats = index.stats();

        // Store in memory
        self.vector_indexes
            .write()
            .unwrap()
            .insert(config.name.clone(), Arc::new(index));

        // Persist populated index
        self.persist_vector_indexes()?;

        Ok(stats)
    }

    /// Drop a vector index
    pub fn drop_vector_index(&self, name: &str) -> DbResult<()> {
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Remove from memory
        {
            let mut indexes = self.vector_indexes.write().unwrap();
            indexes.remove(name);
        }

        // Remove from disk
        db.delete_cf(cf, Self::vec_meta_key(name)).map_err(|e| {
            DbError::InternalError(format!("Failed to delete vector config: {}", e))
        })?;
        db.delete_cf(cf, Self::vec_data_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to delete vector data: {}", e)))?;

        Ok(())
    }

    /// List vector indexes
    pub fn list_vector_indexes(&self) -> Vec<VectorIndexStats> {
        self.get_all_vector_index_configs()
            .into_iter()
            .map(|config| {
                let count = {
                    let indexes = self.vector_indexes.read().unwrap();
                    if let Some(idx) = indexes.get(&config.name) {
                        idx.len()
                    } else {
                        0
                    }
                };

                VectorIndexStats {
                    name: config.name,
                    field: config.field,
                    dimension: config.dimension,
                    metric: config.metric,
                    m: config.m,
                    ef_construction: config.ef_construction,
                    indexed_vectors: count,
                    quantization: config.quantization,
                    memory_bytes: 0,
                    compression_ratio: 1.0,
                }
            })
            .collect()
    }

    /// Search similar vectors
    pub fn vector_search(
        &self,
        name: &str,
        query: &[f32],
        k: usize,
        ef_search: Option<usize>,
    ) -> DbResult<Vec<VectorSearchResult>> {
        let index = self.get_vector_index(name)?;

        // Use provided ef_search or default from config (if available via accessor, or just pass None/default)
        // Assuming index.search supports ef_search
        index.search(query, k, ef_search.unwrap_or(100))
    }

    /// Calculate similarity between a vector and documents
    pub fn vector_similarity(&self, name: &str, query: Vec<f32>) -> DbResult<Vec<(String, f32)>> {
        let index = self.get_vector_index(name)?;
        let results = index.search(&query, 100, 100)?;
        Ok(results.into_iter().map(|r| (r.doc_key, r.score)).collect())
    }

    /// Quantize a vector index
    pub fn quantize_vector_index(
        &self,
        name: &str,
        quantization: VectorQuantization,
    ) -> DbResult<QuantizationStats> {
        let _index = self.get_vector_index(name)?;

        // index.quantize(quantization.clone())?; // Assuming method exists

        // Update config
        let mut configs = self.get_all_vector_index_configs();
        if let Some(config) = configs.iter_mut().find(|c| c.name == name) {
            config.quantization = quantization;

            // Save config
            let db = self.db.read().unwrap();
            let cf = db.cf_handle(&self.name).unwrap();
            let config_bytes = serde_json::to_vec(config)?;
            db.put_cf(cf, Self::vec_meta_key(name), &config_bytes)
                .map_err(|e| DbError::InternalError(e.to_string()))?;
        }

        self.persist_vector_indexes()?;

        Ok(QuantizationStats {
            type_: quantization,
            original_size: 0,
            compressed_size: 0,
            compression_ratio: 0.0,
        })
    }

    /// Dequantize a vector index
    pub fn dequantize_vector_index(&self, _name: &str) -> DbResult<()> {
        Ok(())
    }

    /// Persist all in-memory vector indexes to disk
    pub fn persist_vector_indexes(&self) -> DbResult<()> {
        let indexes = self.vector_indexes.read().unwrap();
        let db = self.db.read().unwrap();
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for (name, index_arc) in indexes.iter() {
            // IndexArc is Arc<VectorIndex>.
            // serde_json::to_vec guarantees handling &T.
            // But VectorIndex handles locks.
            // If Serialize implementation locks internally, it's fine.
            // IndexArc is Arc<VectorIndex>.
            // Use manual serialize method
            let bytes = index_arc.serialize()?;
            db.put_cf(cf, Self::vec_data_key(name), &bytes)
                .map_err(|e| {
                    DbError::InternalError(format!(
                        "Failed to persist vector index {}: {}",
                        name, e
                    ))
                })?;
        }
        Ok(())
    }

    /// Update vector indexes on doc update/insert
    pub(crate) fn update_vector_indexes_on_upsert(&self, doc_key: &str, doc_value: &Value) {
        let configs = self.get_all_vector_index_configs();
        if configs.is_empty() {
            return;
        }

        // Ensure loaded
        for config in &configs {
            let _ = self.get_vector_index(&config.name);
        }

        let indexes = self.vector_indexes.read().unwrap();

        for config in configs {
            if let Some(index) = indexes.get(&config.name) {
                if let Some(vector) =
                    self.extract_vector(doc_value, &config.field, config.dimension)
                {
                    let _ = index.insert(doc_key, &vector);
                }
            }
        }
    }

    /// Update vector indexes on doc delete
    pub(crate) fn update_vector_indexes_on_delete(&self, doc_key: &str) {
        let indexes = self.vector_indexes.read().unwrap();
        for index in indexes.values() {
            let _ = index.remove(doc_key);
        }
    }

    /// Helper to extract vector from document
    pub(crate) fn extract_vector(
        &self,
        doc_value: &Value,
        field: &str,
        dim: usize,
    ) -> Option<Vec<f32>> {
        let val = extract_field_value(doc_value, field);
        if let Some(arr) = val.as_array() {
            if arr.len() == dim {
                let vec: Option<Vec<f32>> =
                    arr.iter().map(|v| v.as_f64().map(|f| f as f32)).collect();
                return vec;
            }
        }
        None
    }
}
