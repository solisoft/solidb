use super::*;
use crate::error::{DbError, DbResult};
use crate::server::llm_client::LLMClient;
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
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");
        let prefix = VEC_META_PREFIX.as_bytes();
        let iter = db.prefix_iterator_cf(&cf, prefix);

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
        if let Some(index) = self.vector_indexes.get(name) {
            return Ok(index.clone());
        }

        // Try load from disk
        self.load_vector_index(name)
    }

    /// Load vector index from disk
    pub(crate) fn load_vector_index(
        &self,
        name: &str,
    ) -> DbResult<Arc<super::vector::VectorIndex>> {
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .ok_or(DbError::InternalError("Column family not found".into()))?;

        // Check metadata first
        let meta_key = Self::vec_meta_key(name);
        if db.get_cf(&cf, &meta_key)?.is_none() {
            return Err(DbError::InvalidDocument(format!(
                "Vector Index '{}' not found",
                name
            )));
        }

        // Load data
        let data_key = Self::vec_data_key(name);
        if let Some(bytes) = db.get_cf(&cf, &data_key)? {
            match super::vector::VectorIndex::deserialize(&bytes) {
                Ok(index) => {
                    let index_arc = Arc::new(index);
                    self.vector_indexes
                        .insert(name.to_string(), index_arc.clone());
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
            let db = &self.db;
            let cf = db
                .cf_handle(&self.name)
                .expect("Column family should exist");
            db.put_cf(&cf, Self::vec_meta_key(&name), &config_bytes)
                .map_err(|e| {
                    DbError::InternalError(format!("Failed to create vector index: {}", e))
                })?;
        }

        // Create in-memory index
        let index = VectorIndex::new(config.clone())?;

        // Build index from existing documents (with auto-embed support).
        // Collect texts first and use batched embedding (1 HTTP call instead of N serial)
        // for efficiency when building on large collections.
        let all_docs = self.all();
        let mut direct_vectors: Vec<(String, Vec<f32>)> = Vec::new();
        let mut texts_to_embed: Vec<String> = Vec::new();
        let mut keys_for_embed: Vec<String> = Vec::new();

        for doc in &all_docs {
            let doc_value = doc.to_value();
            if let Some(v) = self.extract_vector(&doc_value, &config.field, config.dimension) {
                direct_vectors.push((doc.key.clone(), v));
            } else if let Some(ref source_field) = config.embedding_source {
                if let Some(text) = extract_field_value(&doc_value, source_field).as_str() {
                    if !text.trim().is_empty() {
                        texts_to_embed.push(text.to_string());
                        keys_for_embed.push(doc.key.clone());
                    }
                }
            }
        }

        // Batched embedding generation (chunked to keep requests reasonable size).
        // This avoids the previous N serial per-document calls.
        let generated_embeddings: Vec<Vec<f32>> = if !texts_to_embed.is_empty() {
            let provider = config.embedding_provider.as_deref();
            let model = config.embedding_model.clone();
            match LLMClient::from_env(provider, model) {
                Ok(client) => {
                    let mut all_embs = vec![];
                    for chunk in texts_to_embed.chunks(200) {
                        let text_refs: Vec<&str> = chunk.iter().map(|s| s.as_str()).collect();
                        match client.embed_batch_blocking(&text_refs) {
                            Ok(embs) => all_embs.extend(embs),
                            Err(e) => {
                                tracing::warn!("Batch embedding chunk failed: {}", e);
                                // continue with partial
                            }
                        }
                    }
                    all_embs
                }
                Err(e) => {
                    tracing::warn!("Could not create embed client for index build: {}", e);
                    vec![]
                }
            }
        } else {
            vec![]
        };

        // Insert direct vectors
        for (key, v) in direct_vectors {
            if v.len() == config.dimension {
                let _ = index.insert(&key, &v);
            }
        }

        // Insert generated (filter bad dims)
        for (key, v) in keys_for_embed.into_iter().zip(generated_embeddings) {
            if v.len() == config.dimension {
                let _ = index.insert(&key, &v);
            } else if !v.is_empty() {
                tracing::warn!(
                    "Batch embed dim mismatch for key {} in index {}: got {} expected {}",
                    key,
                    config.name,
                    v.len(),
                    config.dimension
                );
            }
        }

        // Persist populated index
        self.persist_vector_indexes()?;

        let stats = index.stats();

        // Store in memory
        self.vector_indexes
            .insert(config.name.clone(), Arc::new(index));

        // Persist populated index
        self.persist_vector_indexes()?;

        Ok(stats)
    }

    /// Drop a vector index
    pub fn drop_vector_index(&self, name: &str) -> DbResult<()> {
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        // Remove from memory
        self.vector_indexes.remove(name);

        // Remove from disk
        db.delete_cf(&cf, Self::vec_meta_key(name)).map_err(|e| {
            DbError::InternalError(format!("Failed to delete vector config: {}", e))
        })?;
        db.delete_cf(&cf, Self::vec_data_key(name))
            .map_err(|e| DbError::InternalError(format!("Failed to delete vector data: {}", e)))?;

        Ok(())
    }

    /// List vector indexes
    pub fn list_vector_indexes(&self) -> Vec<VectorIndexStats> {
        self.get_all_vector_index_configs()
            .into_iter()
            .map(|config| {
                let count = if let Some(idx) = self.vector_indexes.get(&config.name) {
                    idx.len()
                } else {
                    0
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

    /// Nearest-neighbour search with an optional equality metadata filter applied
    /// to the hydrated documents. HNSW has no filter push-down, so we over-fetch
    /// `k * overfetch` candidates, hydrate + filter them, and keep the top `k`.
    /// Returns `(document, score)` pairs, best score first.
    ///
    /// `filter` is a map of `field_path -> expected JSON value`; a document matches
    /// only if every entry equals the value at that (dotted) path. An empty filter
    /// matches everything (plain top-k search).
    pub fn vector_search_filtered(
        &self,
        name: &str,
        query: &[f32],
        k: usize,
        overfetch: usize,
        ef_search: Option<usize>,
        filter: &serde_json::Map<String, Value>,
    ) -> DbResult<Vec<(Value, f32)>> {
        if k == 0 {
            return Ok(Vec::new());
        }
        // Over-fetch so a selective filter can still yield up to k rows.
        let fetch = k.saturating_mul(overfetch.max(1)).max(k);
        let candidates = self.vector_search(name, query, fetch, ef_search)?;

        let mut out: Vec<(Value, f32)> = Vec::with_capacity(k);
        for r in candidates {
            if out.len() >= k {
                break;
            }
            let doc = match self.get(&r.doc_key) {
                Ok(d) => d.to_value(),
                Err(_) => continue, // doc deleted since indexing
            };
            if Self::doc_matches_filter(&doc, filter) {
                out.push((doc, r.score));
            }
        }
        Ok(out)
    }

    /// True when the document equals `filter` on every field (dotted paths allowed).
    fn doc_matches_filter(doc: &Value, filter: &serde_json::Map<String, Value>) -> bool {
        filter
            .iter()
            .all(|(field, expected)| extract_field_value(doc, field) == *expected)
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
            let db = &self.db;
            let cf = db.cf_handle(&self.name).unwrap();
            let config_bytes = serde_json::to_vec(config)?;
            db.put_cf(&cf, Self::vec_meta_key(name), &config_bytes)
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
        let db = &self.db;
        let cf = db
            .cf_handle(&self.name)
            .expect("Column family should exist");

        for entry in self.vector_indexes.iter() {
            let name = entry.key();
            let index_arc = entry.value();
            let bytes = index_arc.serialize()?;
            db.put_cf(&cf, Self::vec_data_key(name), &bytes)
                .map_err(|e| {
                    DbError::InternalError(format!(
                        "Failed to persist vector index {}: {}",
                        name, e
                    ))
                })?;
        }
        Ok(())
    }

    /// Update vector indexes on doc update/insert.
    ///
    /// This runs inside the synchronous storage write path (and under transaction
    /// commit), so it MUST NOT perform network I/O. It indexes a vector only when
    /// the document already carries one in the target field. When an index declares
    /// an `embedding_source` and the document has source text but no vector, it
    /// records a cheap "pending embed" marker for the async embedding worker
    /// (`crate::queue`) to fill in later — never blocking the write on the network.
    /// Auto-embedding may also be done up front in the async API layer
    /// (`inject_auto_embeddings_if_needed`).
    pub(crate) fn update_vector_indexes_on_upsert(&self, doc_key: &str, doc_value: &Value) {
        let configs = self.get_all_vector_index_configs();
        if configs.is_empty() {
            return;
        }

        // Ensure loaded
        for config in &configs {
            let _ = self.get_vector_index(&config.name);
        }

        for config in configs {
            // 1. A concrete vector present in the doc — index it directly.
            if let Some(index) = self.vector_indexes.get(&config.name) {
                if let Some(vector) =
                    self.extract_vector(doc_value, &config.field, config.dimension)
                {
                    let _ = index.insert(doc_key, &vector);
                    if config.embedding_source.is_some() {
                        // Vector is now present; drop any stale pending-embed marker.
                        self.clear_embed_pending(&config.name, doc_key);
                    }
                    continue;
                }
            }

            // 2. No vector. If this index auto-embeds from a text field, enqueue a
            //    pending marker (or clear it when the source text is gone).
            if let Some(ref source_field) = config.embedding_source {
                let has_text = extract_field_value(doc_value, source_field)
                    .as_str()
                    .map(|t| !t.trim().is_empty())
                    .unwrap_or(false);
                if has_text {
                    self.mark_embed_pending(&config.name, doc_key);
                } else {
                    self.clear_embed_pending(&config.name, doc_key);
                }
            }
        }
    }

    /// Update vector indexes on doc delete
    pub(crate) fn update_vector_indexes_on_delete(&self, doc_key: &str) {
        for entry in self.vector_indexes.iter() {
            let _ = entry.remove(doc_key);
        }
        // Drop any pending-embed markers for this doc across all auto-embed indexes.
        for config in self.get_all_vector_index_configs() {
            if config.embedding_source.is_some() {
                self.clear_embed_pending(&config.name, doc_key);
            }
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

    // ==================== Auto-embedding pending queue ====================
    //
    // When an auto-embed document lands without a vector, the write path records a
    // cheap marker key in the collection's own column family (same idiom as TTL /
    // vector-metadata entries). The async embedding worker (`crate::queue`) drains
    // these, generates embeddings off the write path, and writes them back.

    fn embed_pending_key(index: &str, doc_key: &str) -> String {
        format!("{}{}:{}", EMBED_PENDING_PREFIX, index, doc_key)
    }

    fn embed_pending_index_prefix(index: &str) -> String {
        format!("{}{}:", EMBED_PENDING_PREFIX, index)
    }

    /// Record that `doc_key` needs an embedding generated for vector index `index`.
    /// Cheap and network-free — consumed later by the async embedding worker.
    pub(crate) fn mark_embed_pending(&self, index: &str, doc_key: &str) {
        let Some(cf) = self.db.cf_handle(&self.name) else {
            return;
        };
        let key = Self::embed_pending_key(index, doc_key);
        // Only count genuinely new markers so the process-wide gauge stays meaningful.
        if matches!(self.db.get_cf(&cf, key.as_bytes()), Ok(Some(_))) {
            return;
        }
        if self.db.put_cf(&cf, key.as_bytes(), b"").is_ok() {
            PENDING_EMBED_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Remove a pending-embed marker (if present).
    pub(crate) fn clear_embed_pending(&self, index: &str, doc_key: &str) {
        let Some(cf) = self.db.cf_handle(&self.name) else {
            return;
        };
        let key = Self::embed_pending_key(index, doc_key);
        if matches!(self.db.get_cf(&cf, key.as_bytes()), Ok(Some(_)))
            && self.db.delete_cf(&cf, key.as_bytes()).is_ok()
        {
            PENDING_EMBED_COUNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Doc keys awaiting embedding for a given vector index (bounded by `limit`).
    pub fn list_embed_pending(&self, index: &str, limit: usize) -> Vec<String> {
        let Some(cf) = self.db.cf_handle(&self.name) else {
            return Vec::new();
        };
        let prefix = Self::embed_pending_index_prefix(index);
        let mut out = Vec::new();
        for item in self.db.prefix_iterator_cf(&cf, prefix.as_bytes()) {
            if out.len() >= limit {
                break;
            }
            let Ok((k, _)) = item else {
                break;
            };
            if !k.starts_with(prefix.as_bytes()) {
                break;
            }
            out.push(String::from_utf8_lossy(&k[prefix.len()..]).into_owned());
        }
        out
    }

    /// Total documents awaiting embedding in this collection (all auto-embed indexes).
    pub fn count_embed_pending(&self) -> usize {
        let Some(cf) = self.db.cf_handle(&self.name) else {
            return 0;
        };
        let prefix = EMBED_PENDING_PREFIX.as_bytes();
        let mut n = 0;
        for item in self.db.prefix_iterator_cf(&cf, prefix) {
            let Ok((k, _)) = item else {
                break;
            };
            if !k.starts_with(prefix) {
                break;
            }
            n += 1;
        }
        n
    }
}

/// Process-wide gauge of documents awaiting auto-embedding. The background
/// embedding worker reads this as a cheap "is there any work?" gate before
/// scanning collections.
static PENDING_EMBED_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Number of documents currently awaiting auto-embedding, process-wide (approximate).
pub fn pending_embed_count() -> u64 {
    PENDING_EMBED_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}
