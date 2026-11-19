//! Vector index module for similarity search.
//!
//! This module provides vector similarity search capabilities for documents
//! containing embedding vectors. Supports two modes:
//!
//! - **Brute-force**: Accurate linear search, fast for small datasets (< 10K vectors)
//! - **HNSW**: Approximate nearest neighbor search for large datasets (10K+ vectors)
//!
//! The index automatically switches to HNSW when the vector count exceeds the
//! configured threshold (default: 10,000).

use crate::error::{DbError, DbResult};
use crate::storage::hnsw::{Graph, LegacyGraph};
use crate::storage::index::{
    VectorIndexConfig, VectorIndexStats, VectorMetric, VectorQuantization,
};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::RwLock;

/// Result of a vector similarity search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorSearchResult {
    /// Document key
    pub doc_key: String,
    /// Similarity/distance score (interpretation depends on metric)
    pub score: f32,
}

/// Default threshold for auto-switching to HNSW
const DEFAULT_HNSW_THRESHOLD: usize = 10_000;

/// Ceiling on `ef` (HNSW search breadth) derived from caller input (audit A2).
const MAX_HNSW_EF: usize = 10_000;

// =============================================================================
// Scalar Quantization
// =============================================================================

/// Parameters for scalar quantization (per-dimension min/max for reconstruction)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScalarQuantParams {
    /// Minimum value per dimension
    pub min_vals: Vec<f32>,
    /// Maximum value per dimension
    pub max_vals: Vec<f32>,
}

/// Quantized vector storage using scalar quantization (u8 per dimension)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantizedVectors {
    /// Quantized vectors: doc_key -> u8 array
    vectors: HashMap<String, Vec<u8>>,
    /// Quantization parameters for reconstruction
    params: ScalarQuantParams,
}

impl QuantizedVectors {
    /// Create new quantized storage from full-precision vectors
    pub fn from_full_vectors(vectors: &HashMap<String, Vec<f32>>, dimension: usize) -> Self {
        let pairs: Vec<(&str, &[f32])> = vectors
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        Self::from_pairs(&pairs, dimension)
    }

    /// The same from `(key, vector)` pairs.
    pub fn from_pairs(vectors: &[(&str, &[f32])], dimension: usize) -> Self {
        if vectors.is_empty() {
            return Self {
                vectors: HashMap::new(),
                params: ScalarQuantParams {
                    min_vals: vec![0.0; dimension],
                    max_vals: vec![1.0; dimension],
                },
            };
        }

        // Compute min/max per dimension across all vectors
        let mut min_vals = vec![f32::MAX; dimension];
        let mut max_vals = vec![f32::MIN; dimension];

        for (_, vec) in vectors {
            for (i, &v) in vec.iter().enumerate() {
                min_vals[i] = min_vals[i].min(v);
                max_vals[i] = max_vals[i].max(v);
            }
        }

        // Quantize all vectors
        let quantized: HashMap<String, Vec<u8>> = vectors
            .iter()
            .map(|(k, v)| {
                (
                    k.to_string(),
                    Self::quantize_vector(v, &min_vals, &max_vals),
                )
            })
            .collect();

        Self {
            vectors: quantized,
            params: ScalarQuantParams { min_vals, max_vals },
        }
    }

    /// Quantize a single f32 vector to u8
    fn quantize_vector(vec: &[f32], min_vals: &[f32], max_vals: &[f32]) -> Vec<u8> {
        vec.iter()
            .enumerate()
            .map(|(i, &v)| {
                let range = max_vals[i] - min_vals[i];
                if range < 1e-10 {
                    127u8 // Middle value if no range
                } else {
                    ((v - min_vals[i]) / range * 255.0).clamp(0.0, 255.0) as u8
                }
            })
            .collect()
    }

    /// Dequantize a single u8 vector back to f32
    #[allow(dead_code)]
    pub fn dequantize_vector(&self, quantized: &[u8]) -> Vec<f32> {
        quantized
            .iter()
            .enumerate()
            .map(|(i, &q)| {
                let range = self.params.max_vals[i] - self.params.min_vals[i];
                self.params.min_vals[i] + (q as f32 / 255.0) * range
            })
            .collect()
    }

    /// Insert a new vector (quantize on insert)
    pub fn insert(&mut self, doc_key: &str, vec: &[f32]) {
        let quantized = Self::quantize_vector(vec, &self.params.min_vals, &self.params.max_vals);
        self.vectors.insert(doc_key.to_string(), quantized);
    }

    /// Remove a vector
    pub fn remove(&mut self, doc_key: &str) {
        self.vectors.remove(doc_key);
    }

    /// Get number of quantized vectors
    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    /// Check if empty
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    /// Get parameters for distance calculation
    pub fn params(&self) -> &ScalarQuantParams {
        &self.params
    }

    /// Get all quantized vectors
    pub fn vectors(&self) -> &HashMap<String, Vec<u8>> {
        &self.vectors
    }

    /// Clear all quantized vectors
    pub fn clear(&mut self) {
        self.vectors.clear();
    }
}

// =============================================================================
// Asymmetric Distance Functions (query f32, DB u8)
// =============================================================================

/// Calculate cosine similarity between full-precision query and quantized vector
pub fn cosine_similarity_asymmetric(
    query: &[f32],
    quantized: &[u8],
    params: &ScalarQuantParams,
) -> f32 {
    let query_norm_sq: f32 = query.iter().map(|x| x * x).sum();
    if query_norm_sq < 1e-10 {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut quantized_norm_sq = 0.0f32;

    for i in 0..query.len() {
        let range = params.max_vals[i] - params.min_vals[i];
        let dequant = params.min_vals[i] + (quantized[i] as f32 / 255.0) * range;
        dot += query[i] * dequant;
        quantized_norm_sq += dequant * dequant;
    }

    let query_norm = query_norm_sq.sqrt();
    let quantized_norm = quantized_norm_sq.sqrt().max(1e-10);
    dot / (query_norm * quantized_norm)
}

/// Calculate Euclidean distance between full-precision query and quantized vector
pub fn euclidean_distance_asymmetric(
    query: &[f32],
    quantized: &[u8],
    params: &ScalarQuantParams,
) -> f32 {
    let mut sum_sq = 0.0f32;

    for i in 0..query.len() {
        let range = params.max_vals[i] - params.min_vals[i];
        let dequant = params.min_vals[i] + (quantized[i] as f32 / 255.0) * range;
        let diff = query[i] - dequant;
        sum_sq += diff * diff;
    }

    sum_sq.sqrt()
}

/// Calculate dot product between full-precision query and quantized vector
pub fn dot_product_asymmetric(query: &[f32], quantized: &[u8], params: &ScalarQuantParams) -> f32 {
    let mut dot = 0.0f32;

    for i in 0..query.len() {
        let range = params.max_vals[i] - params.min_vals[i];
        let dequant = params.min_vals[i] + (quantized[i] as f32 / 255.0) * range;
        dot += query[i] * dequant;
    }

    dot
}

// =============================================================================
// HNSW: see `storage/hnsw.rs`
// =============================================================================

// =============================================================================
// VectorIndex
// =============================================================================

/// Vector index for similarity search
///
/// Supports two modes:
/// - Brute-force: Linear scan, accurate, fast for < 10K vectors
/// - HNSW: Approximate nearest neighbor, fast for 10K+ vectors
///
/// Automatically switches to HNSW when vector count exceeds threshold.
///
/// Supports optional scalar quantization for 4x memory reduction:
/// - Full precision vectors: f32 (4 bytes/dim)
/// - Quantized vectors: u8 (1 byte/dim)
/// - Asymmetric search: query stays f32, DB vectors are quantized
// Lock poisoning: the search paths run distance computations while holding
// these guards, so a panic there used to make every later `.unwrap()` on the
// same lock panic too — one bad request broke vector search on that
// collection for everyone until restart. The data behind a poisoned guard is
// still the data; recover it.
pub struct VectorIndex {
    /// Index configuration
    config: VectorIndexConfig,
    /// Stored vectors: doc_key -> vector (full precision), while the index
    /// has no graph. Once it has one, the graph's arena is their only store
    /// (`Graph::vector_of`, normalised for cosine, which changes no cosine
    /// score) and this map is empty: holding both kept every vector twice
    /// (card #986). Locks are taken in this order: `vectors`,
    /// `quantized_vectors`, `hnsw_graph`.
    vectors: RwLock<HashMap<String, Vec<f32>>>,
    /// Quantized vectors for memory-efficient storage (optional)
    quantized_vectors: RwLock<Option<QuantizedVectors>>,
    /// HNSW graph for approximate search (built when threshold exceeded)
    hnsw_graph: RwLock<Option<Graph>>,
}

impl VectorIndex {
    /// Create a new vector index with the given configuration
    pub fn new(config: VectorIndexConfig) -> DbResult<Self> {
        if config.dimension == 0 {
            return Err(DbError::BadRequest(
                "Vector dimension must be greater than 0".to_string(),
            ));
        }

        Ok(Self {
            config,
            vectors: RwLock::new(HashMap::new()),
            quantized_vectors: RwLock::new(None),
            hnsw_graph: RwLock::new(None),
        })
    }

    /// Get the HNSW threshold for this index
    fn hnsw_threshold(&self) -> usize {
        // Use config threshold if available, otherwise default
        DEFAULT_HNSW_THRESHOLD
    }

    /// Build the HNSW graph from existing vectors
    fn build_hnsw_graph(&self) {
        let mut vectors = self.vectors.write().unwrap_or_else(|e| e.into_inner());
        if vectors.len() < self.hnsw_threshold() {
            return;
        }

        // Every vector at once, on the CPUs the process may use.
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
        let graph = Graph::build(
            self.config.dimension,
            self.config.metric,
            self.config.m,
            self.config.ef_construction,
            &vectors,
            threads,
        );

        // Store the graph; it holds the vectors from now on. A quantized
        // index keeps them as codes in the graph (card #996), not beside it.
        let mut graph = graph;
        let mut quantized = self
            .quantized_vectors
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if quantized.is_some() || self.config.quantization == VectorQuantization::Scalar {
            graph.quantize();
            *quantized = None;
        }
        let mut hnsw = self.hnsw_graph.write().unwrap_or_else(|e| e.into_inner());
        *hnsw = Some(graph);
        *vectors = HashMap::new();
    }

    /// Get the index configuration
    pub fn config(&self) -> &VectorIndexConfig {
        &self.config
    }

    /// Get the number of indexed vectors
    pub fn len(&self) -> usize {
        let vectors = self.vectors.read().unwrap_or_else(|e| e.into_inner());
        let hnsw = self.hnsw_graph.read().unwrap_or_else(|e| e.into_inner());
        hnsw.as_ref().map_or(vectors.len(), |g| g.len())
    }

    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all vectors from the index
    pub fn clear(&self) {
        let mut vectors = self.vectors.write().unwrap_or_else(|e| e.into_inner());
        vectors.clear();

        // Also clear quantized vectors
        let mut quantized = self
            .quantized_vectors
            .write()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(q) = quantized.as_mut() {
            q.clear();
        }
        *quantized = None;

        // Also clear HNSW graph
        let mut hnsw = self.hnsw_graph.write().unwrap_or_else(|e| e.into_inner());
        *hnsw = None;
    }

    /// Insert a vector for a document
    pub fn insert(&self, doc_key: &str, vector: &[f32]) -> DbResult<()> {
        // Validate dimension
        if vector.len() != self.config.dimension {
            return Err(DbError::BadRequest(format!(
                "Vector dimension mismatch: expected {}, got {}",
                self.config.dimension,
                vector.len()
            )));
        }

        let mut vectors = self.vectors.write().unwrap_or_else(|e| e.into_inner());
        {
            let mut quantized = self
                .quantized_vectors
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(q) = quantized.as_mut() {
                q.insert(doc_key, vector);
            }
        }
        let mut hnsw = self.hnsw_graph.write().unwrap_or_else(|e| e.into_inner());
        if let Some(graph) = hnsw.as_mut() {
            // The graph is the store.
            graph.insert(doc_key, vector);
            return Ok(());
        }
        vectors.insert(doc_key.to_string(), vector.to_vec());
        if vectors.len() >= self.hnsw_threshold() {
            // Threshold reached, build the graph
            drop(hnsw);
            drop(vectors);
            self.build_hnsw_graph();
        }
        Ok(())
    }

    /// Insert many vectors at once (an index created over stored documents):
    /// all of them are stored, then, past the HNSW threshold and with no
    /// graph yet, the graph is built once on every CPU (`Graph::build`).
    /// One `insert` after another built the graph at the threshold and then
    /// linked every later vector on one thread: 300,000 vectors took 97 s
    /// on one core of eight (card #983). Vectors of another dimension are
    /// left out. Returns how many were stored.
    pub fn insert_many(&self, items: Vec<(String, Vec<f32>)>) -> usize {
        let dim = self.config.dimension;
        let items: Vec<(String, Vec<f32>)> =
            items.into_iter().filter(|(_, v)| v.len() == dim).collect();
        if items.is_empty() {
            return 0;
        }
        let stored = items.len();
        let had_graph = self
            .hnsw_graph
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        if had_graph {
            // A graph exists: link each one as `insert` does.
            for (key, v) in &items {
                let _ = self.insert(key, v);
            }
            return stored;
        }
        {
            let mut vectors = self.vectors.write().unwrap_or_else(|e| e.into_inner());
            let mut quantized = self
                .quantized_vectors
                .write()
                .unwrap_or_else(|e| e.into_inner());
            for (key, v) in items {
                if let Some(q) = quantized.as_mut() {
                    q.insert(&key, &v);
                }
                vectors.insert(key, v);
            }
        }
        if self.len() >= self.hnsw_threshold() {
            self.build_hnsw_graph();
        }
        stored
    }

    /// Remove a vector by document key
    pub fn remove(&self, doc_key: &str) -> DbResult<bool> {
        let mut vectors = self.vectors.write().unwrap_or_else(|e| e.into_inner());
        let in_map = vectors.remove(doc_key).is_some();
        {
            let mut quantized = self
                .quantized_vectors
                .write()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(q) = quantized.as_mut() {
                q.remove(doc_key);
            }
        }
        let mut hnsw = self.hnsw_graph.write().unwrap_or_else(|e| e.into_inner());
        let in_graph = match hnsw.as_mut() {
            Some(graph) => {
                let had = graph.contains(doc_key);
                graph.remove(doc_key);
                had
            }
            None => false,
        };
        Ok(in_map || in_graph)
    }

    /// Check if a document has a vector in the index
    pub fn contains(&self, doc_key: &str) -> bool {
        self.get_with(doc_key, |_| ()).is_some()
    }

    /// Get a vector by document key. Once the index has a graph, a cosine
    /// index gives it normalised.
    pub fn get(&self, doc_key: &str) -> Option<Vec<f32>> {
        self.get_with(doc_key, |v| v.to_vec())
    }

    /// `f` of the vector kept for `doc_key`, wherever it is kept.
    fn get_with<T>(&self, doc_key: &str, f: impl FnOnce(&[f32]) -> T) -> Option<T> {
        let vectors = self.vectors.read().unwrap_or_else(|e| e.into_inner());
        if let Some(v) = vectors.get(doc_key) {
            return Some(f(v));
        }
        let hnsw = self.hnsw_graph.read().unwrap_or_else(|e| e.into_inner());
        hnsw.as_ref()?.vector_of(doc_key).map(|v| f(&v))
    }

    /// Search for similar vectors
    ///
    /// Uses HNSW approximate search when the graph is built (vector count >= threshold),
    /// otherwise falls back to brute-force linear search.
    ///
    /// # Arguments
    /// * `query` - Query vector
    /// * `limit` - Maximum number of results to return
    /// * `ef` - HNSW search quality parameter (higher = better recall, slower)
    ///
    /// # Returns
    /// Vector of search results sorted by similarity (best first)
    pub fn search(
        &self,
        query: &[f32],
        limit: usize,
        ef: usize,
    ) -> DbResult<Vec<VectorSearchResult>> {
        // Validate dimension
        if query.len() != self.config.dimension {
            return Err(DbError::BadRequest(format!(
                "Query vector dimension mismatch: expected {}, got {}",
                self.config.dimension,
                query.len()
            )));
        }

        // Try HNSW search first if graph exists
        {
            let hnsw = self.hnsw_graph.read().unwrap_or_else(|e| e.into_inner());
            if let Some(graph) = hnsw.as_ref() {
                // Audit A2: `limit` and `ef` come from the query; `limit * 2`
                // wrapped in release builds. The result set is at most the
                // index size anyway, so bound the search breadth.
                let ef_search = ef
                    .max(limit.saturating_mul(2))
                    .max(40)
                    .min(MAX_HNSW_EF.max(limit.min(graph.len())));
                let hnsw_results = graph.search(query, limit, ef_search);

                // Convert HNSW results (distance) back to score format
                let results: Vec<VectorSearchResult> = hnsw_results
                    .into_iter()
                    .map(|(doc_key, distance)| {
                        // Convert distance back to original score
                        let score = match self.config.metric {
                            VectorMetric::Cosine => 1.0 - distance, // distance was 1-similarity
                            VectorMetric::Euclidean => distance,    // keep as distance
                            VectorMetric::DotProduct => -distance,  // distance was -dot_product
                        };
                        VectorSearchResult { doc_key, score }
                    })
                    .collect();

                return Ok(results);
            }
        }

        // Fall back to brute-force search
        self.brute_force_search(query, limit)
    }

    /// [`Self::search`] among the documents whose key `passes`, the filter
    /// followed in the graph's walk ([`Graph::search_filtered`]). `None`
    /// when the index has no graph yet: the caller filters a plain search.
    pub fn search_filtered(
        &self,
        query: &[f32],
        limit: usize,
        ef: usize,
        max_checks: usize,
        passes: &mut dyn FnMut(&str) -> bool,
    ) -> DbResult<Option<Vec<VectorSearchResult>>> {
        if query.len() != self.config.dimension {
            return Err(DbError::BadRequest(format!(
                "Query vector dimension mismatch: expected {}, got {}",
                self.config.dimension,
                query.len()
            )));
        }
        let hnsw = self.hnsw_graph.read().unwrap_or_else(|e| e.into_inner());
        let Some(graph) = hnsw.as_ref() else {
            return Ok(None);
        };
        let ef = ef
            .max(limit)
            .max(40)
            .min(MAX_HNSW_EF.max(limit.min(graph.len())));
        Ok(Some(
            graph
                .search_filtered(query, limit, ef, max_checks, passes)
                .into_iter()
                .map(|(doc_key, distance)| VectorSearchResult {
                    doc_key,
                    score: self.score_of(distance),
                })
                .collect(),
        ))
    }

    /// The score a search reports for a graph distance.
    fn score_of(&self, distance: f32) -> f32 {
        match self.config.metric {
            VectorMetric::Cosine => 1.0 - distance, // distance was 1-similarity
            VectorMetric::Euclidean => distance,    // keep as distance
            VectorMetric::DotProduct => -distance,  // distance was -dot_product
        }
    }

    /// Perform brute-force linear search
    fn brute_force_search(&self, query: &[f32], limit: usize) -> DbResult<Vec<VectorSearchResult>> {
        // Check if we have quantized vectors - use asymmetric search if so
        {
            let quantized = self
                .quantized_vectors
                .read()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(q) = quantized.as_ref() {
                return self.brute_force_search_quantized(query, limit, q);
            }
        }

        // Fall back to full-precision search
        let vectors = self.vectors.read().unwrap_or_else(|e| e.into_inner());

        // Calculate scores for all vectors
        let mut results: Vec<VectorSearchResult> = vectors
            .iter()
            .map(|(doc_key, vec)| {
                let score = match self.config.metric {
                    VectorMetric::Cosine => cosine_similarity(query, vec),
                    VectorMetric::Euclidean => euclidean_distance(query, vec),
                    VectorMetric::DotProduct => dot_product(query, vec),
                };
                VectorSearchResult {
                    doc_key: doc_key.clone(),
                    score,
                }
            })
            .collect();

        // Sort by score
        match self.config.metric {
            VectorMetric::Cosine | VectorMetric::DotProduct => {
                // Higher is better for similarity/dot product
                results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
            }
            VectorMetric::Euclidean => {
                // Lower is better for distance
                results.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(Ordering::Equal));
            }
        }

        // Limit results
        results.truncate(limit);
        Ok(results)
    }

    /// Perform brute-force search using quantized vectors (asymmetric search)
    fn brute_force_search_quantized(
        &self,
        query: &[f32],
        limit: usize,
        quantized: &QuantizedVectors,
    ) -> DbResult<Vec<VectorSearchResult>> {
        let params = quantized.params();

        // Calculate scores for all quantized vectors using asymmetric distance
        let mut results: Vec<VectorSearchResult> = quantized
            .vectors()
            .iter()
            .map(|(doc_key, quantized_vec)| {
                let score = match self.config.metric {
                    VectorMetric::Cosine => {
                        cosine_similarity_asymmetric(query, quantized_vec, params)
                    }
                    VectorMetric::Euclidean => {
                        euclidean_distance_asymmetric(query, quantized_vec, params)
                    }
                    VectorMetric::DotProduct => {
                        dot_product_asymmetric(query, quantized_vec, params)
                    }
                };
                VectorSearchResult {
                    doc_key: doc_key.clone(),
                    score,
                }
            })
            .collect();

        // Sort by score
        match self.config.metric {
            VectorMetric::Cosine | VectorMetric::DotProduct => {
                // Higher is better for similarity/dot product
                results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
            }
            VectorMetric::Euclidean => {
                // Lower is better for distance
                results.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap_or(Ordering::Equal));
            }
        }

        // Limit results
        results.truncate(limit);
        Ok(results)
    }

    /// Calculate similarity between a query vector and a document's vector
    pub fn similarity(&self, doc_key: &str, query: &[f32]) -> DbResult<Option<f32>> {
        if query.len() != self.config.dimension {
            return Err(DbError::BadRequest(format!(
                "Query vector dimension mismatch: expected {}, got {}",
                self.config.dimension,
                query.len()
            )));
        }

        let metric = self.config.metric;
        Ok(self.get_with(doc_key, |doc_vec| match metric {
            VectorMetric::Cosine => cosine_similarity(query, doc_vec),
            VectorMetric::Euclidean => euclidean_distance(query, doc_vec),
            VectorMetric::DotProduct => dot_product(query, doc_vec),
        }))
    }

    /// Serialize the index to bytes for persistence
    ///
    /// Serializes straight from the guarded data (audit D9): cloning the
    /// vector map and the HNSW graph first put peak memory at 3-4x the index
    /// size. `VectorIndexDataV4Ref` encodes as `VectorIndexDataV4` decodes,
    /// behind the `FORMAT_V4` marker.
    pub fn serialize(&self) -> DbResult<Vec<u8>> {
        let mut out = Vec::new();
        self.serialize_into(&mut out)?;
        Ok(out)
    }

    /// [`Self::serialize`] into `out` as it goes: a writer that stores what
    /// it is given in pieces saves the index without holding it whole.
    pub fn serialize_into(&self, mut out: impl std::io::Write) -> DbResult<()> {
        let vectors = self.vectors.read().unwrap_or_else(|e| e.into_inner());
        let quantized = self
            .quantized_vectors
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let hnsw = self.hnsw_graph.read().unwrap_or_else(|e| e.into_inner());

        let sq = hnsw.as_ref().and_then(|g| g.sq());
        let data = VectorIndexDataV6Ref {
            config: &self.config,
            vectors: &vectors,
            quantized_vectors: &quantized,
            hnsw_graph: &hnsw,
            arena: hnsw.as_ref().map_or(&[][..], |g| g.arena()),
            sq_min: sq.map_or(&[][..], |q| &q.min),
            sq_step: sq.map_or(&[][..], |q| &q.step),
            sq_codes: sq.map_or(&[][..], |q| &q.codes),
        };
        out.write_all(FORMAT_V6)
            .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))?;
        bincode::serialize_into(&mut out, &data)
            .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))
    }

    /// An index [`Self::serialize_into`] wrote, read as it comes (V6, or
    /// V5 as 2.15 wrote it).
    pub fn deserialize_from(mut input: impl std::io::Read) -> DbResult<Self> {
        let mut marker = [0u8; 8];
        input
            .read_exact(&mut marker)
            .map_err(|e| DbError::InternalError(format!("Deserialization error: {}", e)))?;
        let err =
            |e: bincode::Error| DbError::InternalError(format!("Deserialization error: {}", e));
        if marker[..] == *FORMAT_V6 {
            return Self::from_v6(bincode::deserialize_from(input).map_err(err)?);
        }
        if marker[..] == *FORMAT_V5 {
            let data: VectorIndexDataV5 = bincode::deserialize_from(input).map_err(err)?;
            return Self::from_v6(data.into());
        }
        Err(DbError::InternalError(
            "Deserialization error: not a V5 or V6 vector index".to_string(),
        ))
    }

    fn from_v6(data: VectorIndexDataV6) -> DbResult<Self> {
        let mut graph = data.hnsw_graph;
        let mut quantized = data.quantized_vectors;
        if let Some(g) = graph.as_mut() {
            // A quantized graph saves its bounds even with no node.
            let restored = if data.sq_min.is_empty() {
                g.restore_arena(data.arena)
            } else {
                g.restore_sq(crate::storage::hnsw::Sq8 {
                    min: data.sq_min,
                    step: data.sq_step,
                    codes: data.sq_codes,
                })
            };
            if !restored {
                return Err(DbError::InternalError(
                    "Deserialization error: the graph's vectors do not match its nodes".to_string(),
                ));
            }
            // An index quantized before 2.16 kept its codes beside an f32
            // graph that searches never left: the graph takes them now.
            if quantized.is_some() {
                g.quantize();
                quantized = None;
            }
        }
        Ok(Self {
            config: data.config,
            vectors: RwLock::new(data.vectors),
            quantized_vectors: RwLock::new(quantized),
            hnsw_graph: RwLock::new(graph),
        })
    }

    /// Deserialize a vector index from bytes
    pub fn deserialize(bytes: &[u8]) -> DbResult<Self> {
        // V6 (card #996): V5 and a quantized graph's codes.
        if let Some(rest) = bytes.strip_prefix(FORMAT_V6) {
            let data: VectorIndexDataV6 = bincode::deserialize(rest)
                .map_err(|e| DbError::InternalError(format!("Deserialization error: {}", e)))?;
            return Self::from_v6(data);
        }
        // V5 (card #986): the graph's vectors saved as its arena, the map
        // only for an index without a graph.
        if let Some(rest) = bytes.strip_prefix(FORMAT_V5) {
            let data: VectorIndexDataV5 = bincode::deserialize(rest)
                .map_err(|e| DbError::InternalError(format!("Deserialization error: {}", e)))?;
            return Self::from_v6(data.into());
        }
        // V4 (numbered graph, card #983): behind its marker.
        if let Some(rest) = bytes.strip_prefix(FORMAT_V4) {
            let data: VectorIndexDataV4 = bincode::deserialize(rest)
                .map_err(|e| DbError::InternalError(format!("Deserialization error: {}", e)))?;
            let mut graph = data.hnsw_graph;
            if let Some(g) = graph.as_mut() {
                g.restore(&data.vectors);
            }
            return Ok(Self::loaded(
                data.config,
                data.vectors,
                data.quantized_vectors,
                graph,
            ));
        }
        // Up to 2.14.0 the graph was keyed by strings: it is renumbered.
        let legacy = |config: &VectorIndexConfig,
                      vectors: &HashMap<String, Vec<f32>>,
                      g: Option<LegacyGraph>| {
            g.map(|g| Graph::from_legacy(g, config.dimension, config.metric, vectors))
        };

        // Try V3 format first (with quantization)
        if let Ok(data) = bincode::deserialize::<VectorIndexDataV3>(bytes) {
            let graph = legacy(&data.config, &data.vectors, data.hnsw_graph);
            return Ok(Self::loaded(
                data.config,
                data.vectors,
                data.quantized_vectors,
                graph,
            ));
        }

        // Try V2 format (with HNSW graph)
        if let Ok(data) = bincode::deserialize::<VectorIndexDataV2>(bytes) {
            let graph = legacy(&data.config, &data.vectors, data.hnsw_graph);
            return Ok(Self::loaded(data.config, data.vectors, None, graph));
        }

        // Fall back to V1 format (without HNSW graph)
        let data: VectorIndexData = bincode::deserialize(bytes)
            .map_err(|e| DbError::InternalError(format!("Deserialization error: {}", e)))?;

        Ok(Self {
            config: data.config,
            vectors: RwLock::new(data.vectors),
            quantized_vectors: RwLock::new(None),
            hnsw_graph: RwLock::new(None),
        })
    }

    /// An index read from an older format: once the graph has its vectors,
    /// the map is not kept.
    fn loaded(
        config: VectorIndexConfig,
        vectors: HashMap<String, Vec<f32>>,
        quantized: Option<QuantizedVectors>,
        graph: Option<Graph>,
    ) -> Self {
        let vectors = if graph.is_some() {
            HashMap::new()
        } else {
            vectors
        };
        let mut graph = graph;
        let mut quantized = quantized;
        if let (Some(g), Some(_)) = (graph.as_mut(), quantized.as_ref()) {
            g.quantize();
            quantized = None;
        }
        Self {
            config,
            vectors: RwLock::new(vectors),
            quantized_vectors: RwLock::new(quantized),
            hnsw_graph: RwLock::new(graph),
        }
    }

    /// Check if HNSW graph is currently active
    pub fn is_hnsw_active(&self) -> bool {
        self.hnsw_graph
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }

    /// Check if scalar quantization is active: codes for the brute-force
    /// search, or a quantized graph.
    pub fn is_quantized(&self) -> bool {
        self.quantized_vectors
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
            || self.graph_is_quantized()
    }

    /// The graph searches codes: what it finds is re-scored against the
    /// exact vectors by the caller, which reads them from the documents.
    pub fn graph_is_quantized(&self) -> bool {
        self.hnsw_graph
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|g| g.is_quantized())
    }

    /// The score a search reports for `vector`, exact: what re-scoring
    /// compares.
    pub fn exact_score(&self, query: &[f32], vector: &[f32]) -> f32 {
        match self.config.metric {
            VectorMetric::Cosine => cosine_similarity(query, vector),
            VectorMetric::Euclidean => euclidean_distance(query, vector),
            VectorMetric::DotProduct => dot_product(query, vector),
        }
    }

    /// Whether score `a` ranks before score `b` for this index's metric.
    pub fn better(&self, a: f32, b: f32) -> Ordering {
        match self.config.metric {
            VectorMetric::Euclidean => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
            _ => b.partial_cmp(&a).unwrap_or(Ordering::Equal),
        }
    }

    /// Get the current quantization type
    pub fn quantization_type(&self) -> VectorQuantization {
        if self.is_quantized() {
            VectorQuantization::Scalar
        } else {
            VectorQuantization::None
        }
    }

    /// Quantize all vectors using scalar quantization
    ///
    /// Computes per-dimension min/max values from all existing vectors,
    /// then quantizes each vector to u8 values. Future searches will use
    /// asymmetric distance computation (query f32, DB u8).
    ///
    /// Returns the number of vectors quantized.
    pub fn quantize(&self) -> DbResult<usize> {
        let vectors = self.vectors.read().unwrap_or_else(|e| e.into_inner());
        let mut quantized_lock = self
            .quantized_vectors
            .write()
            .unwrap_or_else(|e| e.into_inner());
        let mut hnsw = self.hnsw_graph.write().unwrap_or_else(|e| e.into_inner());
        // With a graph, the graph keeps the codes and drops its f32 vectors
        // (card #996): codes beside an f32 graph saved nothing, and the
        // graph's search never read them.
        if let Some(graph) = hnsw.as_mut() {
            graph.quantize();
            *quantized_lock = None;
            return Ok(graph.len());
        }
        drop(hnsw);

        let pairs: Vec<(&str, &[f32])> = vectors
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        let count = pairs.len();
        if count == 0 {
            // No vector to take bounds from. An index created quantized
            // quantizes its graph when it builds it (`build_hnsw_graph`).
            return Ok(0);
        }
        // Codes for the brute-force search.
        *quantized_lock = Some(QuantizedVectors::from_pairs(&pairs, self.config.dimension));
        Ok(count)
    }

    /// Remove quantization (revert to full-precision search). A quantized
    /// graph's vectors come back decoded from their codes, with their
    /// error: rebuild the index from its documents for exact ones.
    pub fn dequantize(&self) {
        let mut quantized = self
            .quantized_vectors
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *quantized = None;
        let mut hnsw = self.hnsw_graph.write().unwrap_or_else(|e| e.into_inner());
        if let Some(graph) = hnsw.as_mut() {
            graph.dequantize();
        }
    }

    /// Get quantization statistics
    pub fn quantization_stats(&self) -> Option<QuantizationStats> {
        let quantized = self
            .quantized_vectors
            .read()
            .unwrap_or_else(|e| e.into_inner());
        let hnsw = self.hnsw_graph.read().unwrap_or_else(|e| e.into_inner());
        let count = match (quantized.as_ref(), hnsw.as_ref()) {
            (_, Some(g)) if g.is_quantized() => Some(g.len()),
            (Some(q), _) => Some(q.len()),
            _ => None,
        };
        count.map(|vector_count| {
            let memory_bytes = vector_count * self.config.dimension; // u8 per dim
            let full_memory_bytes =
                vector_count * self.config.dimension * std::mem::size_of::<f32>();
            QuantizationStats {
                vector_count,
                memory_bytes,
                full_memory_bytes,
                compression_ratio: if memory_bytes > 0 {
                    full_memory_bytes as f32 / memory_bytes as f32
                } else {
                    1.0
                },
            }
        })
    }

    /// Get index statistics
    pub fn stats(&self) -> VectorIndexStats {
        let (memory_bytes, compression_ratio) = if let Some(stats) = self.quantization_stats() {
            (stats.memory_bytes, stats.compression_ratio)
        } else {
            (0, 1.0)
        };

        VectorIndexStats {
            name: self.config.name.clone(),
            field: self.config.field.clone(),
            dimension: self.config.dimension,
            metric: self.config.metric,
            m: self.config.m,
            ef_construction: self.config.ef_construction,
            indexed_vectors: self.len(),
            quantization: self.quantization_type(),
            memory_bytes,
            compression_ratio,
        }
    }
}

/// Statistics about quantized vector storage
#[derive(Debug, Clone, Serialize)]
pub struct QuantizationStats {
    /// Number of quantized vectors
    pub vector_count: usize,
    /// Memory usage in bytes (quantized)
    pub memory_bytes: usize,
    /// Memory usage in bytes if full precision
    pub full_memory_bytes: usize,
    /// Compression ratio (full / quantized)
    pub compression_ratio: f32,
}

/// Serializable vector index data (V1 format - backward compat)
#[derive(Serialize, Deserialize)]
struct VectorIndexData {
    config: VectorIndexConfig,
    vectors: HashMap<String, Vec<f32>>,
}

/// Serializable vector index data (V2 format - with HNSW)
#[derive(Serialize, Deserialize)]
struct VectorIndexDataV2 {
    config: VectorIndexConfig,
    vectors: HashMap<String, Vec<f32>>,
    #[serde(default)]
    hnsw_graph: Option<LegacyGraph>,
}

/// Serializable vector index data (V3 format - with quantization)
#[derive(Serialize, Deserialize)]
struct VectorIndexDataV3 {
    config: VectorIndexConfig,
    vectors: HashMap<String, Vec<f32>>,
    #[serde(default)]
    quantized_vectors: Option<QuantizedVectors>,
    #[serde(default)]
    hnsw_graph: Option<LegacyGraph>,
}

/// Borrowing twin of `VectorIndexDataV4`, which 2.14.x wrote; kept to
/// write V4 indexes in tests. Field order and types must match it exactly:
/// serde encodes `&T` as `T`.
#[cfg(test)]
#[derive(Serialize)]
struct VectorIndexDataV4Ref<'a> {
    config: &'a VectorIndexConfig,
    vectors: &'a HashMap<String, Vec<f32>>,
    quantized_vectors: &'a Option<QuantizedVectors>,
    hnsw_graph: &'a Option<Graph>,
}

/// V6 as the index writes it, borrowed from its guarded data. Field order
/// and types as `VectorIndexDataV6`, which bincode depends on.
#[derive(Serialize)]
struct VectorIndexDataV6Ref<'a> {
    config: &'a VectorIndexConfig,
    vectors: &'a HashMap<String, Vec<f32>>,
    quantized_vectors: &'a Option<QuantizedVectors>,
    hnsw_graph: &'a Option<Graph>,
    arena: &'a [f32],
    sq_min: &'a [f32],
    sq_step: &'a [f32],
    sq_codes: &'a [u8],
}

/// V6 (card #996): V5 and a quantized graph's vectors as codes (`sq_*`,
/// empty when the graph is not quantized; `arena` is empty when it is).
#[derive(Deserialize)]
struct VectorIndexDataV6 {
    config: VectorIndexConfig,
    vectors: HashMap<String, Vec<f32>>,
    quantized_vectors: Option<QuantizedVectors>,
    hnsw_graph: Option<Graph>,
    arena: Vec<f32>,
    sq_min: Vec<f32>,
    sq_step: Vec<f32>,
    sq_codes: Vec<u8>,
}

impl From<VectorIndexDataV5> for VectorIndexDataV6 {
    fn from(v5: VectorIndexDataV5) -> Self {
        VectorIndexDataV6 {
            config: v5.config,
            vectors: v5.vectors,
            quantized_vectors: v5.quantized_vectors,
            hnsw_graph: v5.hnsw_graph,
            arena: v5.arena,
            sq_min: Vec::new(),
            sq_step: Vec::new(),
            sq_codes: Vec::new(),
        }
    }
}

#[derive(Deserialize)]
struct VectorIndexDataV5 {
    config: VectorIndexConfig,
    vectors: HashMap<String, Vec<f32>>,
    quantized_vectors: Option<QuantizedVectors>,
    hnsw_graph: Option<Graph>,
    arena: Vec<f32>,
}

/// V4 (card #983): the graph numbered (`storage/hnsw.rs`), behind
/// `FORMAT_V4`; earlier formats have no marker and are tried in turn.
#[derive(Deserialize)]
struct VectorIndexDataV4 {
    config: VectorIndexConfig,
    vectors: HashMap<String, Vec<f32>>,
    quantized_vectors: Option<QuantizedVectors>,
    hnsw_graph: Option<Graph>,
}

/// What a V4 index's bytes start with. Bincode of an older format starts
/// with the config's name length as a u64, which this cannot be.
const FORMAT_V4: &[u8] = b"\xffSVIDX4\x00";
/// What a V5 index's bytes start with.
const FORMAT_V5: &[u8] = b"\xffSVIDX5\x00";
/// What a V6 index's bytes start with.
const FORMAT_V6: &[u8] = b"\xffSVIDX6\x00";

/// Calculate cosine similarity between two vectors
///
/// Returns a value between -1 and 1, where 1 means identical direction,
/// 0 means orthogonal, and -1 means opposite direction.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let norm = (norm_a.sqrt() * norm_b.sqrt()).max(1e-10);
    dot / norm
}

/// Calculate Euclidean distance between two vectors
///
/// Returns a non-negative value where 0 means identical vectors.
pub fn euclidean_distance(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return f32::MAX;
    }

    let mut sum = 0.0f32;
    for i in 0..a.len() {
        let diff = a[i] - b[i];
        sum += diff * diff;
    }
    sum.sqrt()
}

/// Calculate dot product between two vectors
pub fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }

    let mut dot = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
    }
    dot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);

        let c = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &c).abs() < 1e-6);

        let d = vec![-1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &d) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_euclidean_distance() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((euclidean_distance(&a, &b) - 1.0).abs() < 1e-6);

        let c = vec![3.0, 4.0, 0.0];
        assert!((euclidean_distance(&a, &c) - 5.0).abs() < 1e-6);
    }

    #[test]
    fn test_dot_product() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![4.0, 5.0, 6.0];
        assert!((dot_product(&a, &b) - 32.0).abs() < 1e-6);
    }

    #[test]
    fn test_vector_index_creation() {
        let config = VectorIndexConfig::new("test_idx".to_string(), "embedding".to_string(), 3);
        let index = VectorIndex::new(config).unwrap();
        assert!(index.is_empty());
    }

    #[test]
    fn test_vector_index_insert_and_search() {
        let config = VectorIndexConfig::new("test_idx".to_string(), "embedding".to_string(), 3);
        let index = VectorIndex::new(config).unwrap();

        // Insert some vectors
        index.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        index.insert("doc2", &[0.0, 1.0, 0.0]).unwrap();
        index.insert("doc3", &[0.9, 0.1, 0.0]).unwrap();

        assert_eq!(index.len(), 3);

        // Search for similar vectors
        let results = index.search(&[1.0, 0.0, 0.0], 2, 10).unwrap();
        assert_eq!(results.len(), 2);

        // doc1 should be most similar to [1, 0, 0]
        assert_eq!(results[0].doc_key, "doc1");
        assert!((results[0].score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_vector_index_dimension_validation() {
        let config = VectorIndexConfig::new("test_idx".to_string(), "embedding".to_string(), 3);
        let index = VectorIndex::new(config).unwrap();

        // Wrong dimension should fail
        let result = index.insert("doc1", &[1.0, 0.0]);
        assert!(result.is_err());
    }

    #[test]
    fn test_vector_index_remove() {
        let config = VectorIndexConfig::new("test_idx".to_string(), "embedding".to_string(), 3);
        let index = VectorIndex::new(config).unwrap();

        index.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        assert_eq!(index.len(), 1);

        let removed = index.remove("doc1").unwrap();
        assert!(removed);
        assert_eq!(index.len(), 0);

        // Removing non-existent doc should return false
        let removed = index.remove("nonexistent").unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_vector_index_with_euclidean() {
        let config = VectorIndexConfig::new("test_idx".to_string(), "embedding".to_string(), 3)
            .with_metric(VectorMetric::Euclidean);
        let index = VectorIndex::new(config).unwrap();

        index.insert("doc1", &[0.0, 0.0, 0.0]).unwrap();
        index.insert("doc2", &[1.0, 0.0, 0.0]).unwrap();
        index.insert("doc3", &[10.0, 0.0, 0.0]).unwrap();

        let results = index.search(&[0.0, 0.0, 0.0], 3, 10).unwrap();

        // doc1 should be closest (distance 0)
        assert_eq!(results[0].doc_key, "doc1");
        assert!(results[0].score < 0.1);
    }

    /// On clustered vectors (100 tight clusters, as embeddings are), the
    /// HNSW search finds most of the exact ten nearest neighbours of points
    /// it was not given, and more of them with a larger `ef`. With the nearest
    /// M as links the graph fell apart by cluster and recall@10 was about 0.5
    /// whatever `ef` (card #979).
    #[test]
    fn hnsw_recall_on_clustered_vectors() {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let dim = 32;
        let centres: Vec<Vec<f32>> = (0..100)
            .map(|_| (0..dim).map(|_| rng.gen_range(-1.0..1.0)).collect())
            .collect();
        let point = |rng: &mut rand::rngs::StdRng| -> Vec<f32> {
            let c = &centres[rng.gen_range(0..centres.len())];
            let v: Vec<f32> = c.iter().map(|x| x + rng.gen_range(-0.3..0.3)).collect();
            let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.into_iter().map(|x| x / n).collect()
        };
        let config = VectorIndexConfig::new("idx".to_string(), "emb".to_string(), dim);
        let index = VectorIndex::new(config).unwrap();
        let data: Vec<Vec<f32>> = (0..12_000).map(|_| point(&mut rng)).collect();
        for (i, v) in data.iter().enumerate() {
            index.insert(&format!("d{i}"), v).unwrap();
        }
        let queries: Vec<Vec<f32>> = (0..100).map(|_| point(&mut rng)).collect();
        let recall = |ef: usize| -> f64 {
            let mut found = 0;
            for q in &queries {
                let mut exact: Vec<(f32, usize)> = data
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (cosine_similarity(q, v), i))
                    .collect();
                exact.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                let want: Vec<String> = exact[..10].iter().map(|(_, i)| format!("d{i}")).collect();
                let got = index.search(q, 10, ef).unwrap();
                found += got.iter().filter(|r| want.contains(&r.doc_key)).count();
            }
            found as f64 / (10 * queries.len()) as f64
        };
        let low = recall(40);
        let high = recall(200);
        assert!(low >= 0.85, "recall@10 at ef 40: {low}");
        assert!(high >= 0.95, "recall@10 at ef 200: {high}");
        assert!(high >= low, "ef 200 {high} below ef 40 {low}");
    }

    /// Data vectors, query vectors, and each query's exact top-10 keys.
    type Clustered = (Vec<Vec<f32>>, Vec<Vec<f32>>, Vec<Vec<String>>);

    /// Clustered unit vectors as `hnsw_recall_on_clustered_vectors` makes
    /// them, and the keys of each query's exact top 10.
    fn clustered(n: usize, queries: usize, dim: usize) -> Clustered {
        use rand::{Rng, SeedableRng};
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        let centres: Vec<Vec<f32>> = (0..100)
            .map(|_| (0..dim).map(|_| rng.gen_range(-1.0..1.0)).collect())
            .collect();
        let point = |rng: &mut rand::rngs::StdRng| -> Vec<f32> {
            let c = &centres[rng.gen_range(0..centres.len())];
            let v: Vec<f32> = c.iter().map(|x| x + rng.gen_range(-0.3..0.3)).collect();
            let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.into_iter().map(|x| x / n).collect()
        };
        let data: Vec<Vec<f32>> = (0..n).map(|_| point(&mut rng)).collect();
        let qs: Vec<Vec<f32>> = (0..queries).map(|_| point(&mut rng)).collect();
        let want = qs
            .iter()
            .map(|q| {
                let mut exact: Vec<(f32, usize)> = data
                    .iter()
                    .enumerate()
                    .map(|(i, v)| (cosine_similarity(q, v), i))
                    .collect();
                exact.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                exact[..10].iter().map(|(_, i)| format!("d{i}")).collect()
            })
            .collect();
        (data, qs, want)
    }

    /// A quantized graph keeps one byte a dimension and no f32 vector
    /// (card #996): its own walk, on the codes, still finds the nearest;
    /// it survives a save, takes new vectors and comes back from
    /// `dequantize`.
    #[test]
    fn a_quantized_graph_searches_its_codes() {
        let dim = 32;
        let (data, queries, want) = clustered(12_000, 50, dim);
        let config = VectorIndexConfig::new("idx".to_string(), "emb".to_string(), dim);
        let index = VectorIndex::new(config).unwrap();
        index.insert_many(
            data.iter()
                .enumerate()
                .map(|(i, v)| (format!("d{i}"), v.clone()))
                .collect(),
        );
        assert!(index.is_hnsw_active());
        assert_eq!(index.quantize().unwrap(), 12_000);
        assert!(index.graph_is_quantized());
        assert!(index.is_quantized());
        {
            let graph = index.hnsw_graph.read().unwrap();
            let graph = graph.as_ref().unwrap();
            assert!(graph.arena().is_empty(), "no f32 copy is kept");
            assert_eq!(graph.sq().unwrap().codes.len(), 12_000 * dim);
        }
        assert!(index.quantized_vectors.read().unwrap().is_none());
        let stats = index.quantization_stats().unwrap();
        assert_eq!(stats.vector_count, 12_000);
        assert_eq!(stats.memory_bytes, 12_000 * dim);

        let recall = |index: &VectorIndex, ef: usize| -> f64 {
            let mut found = 0;
            for (q, want) in queries.iter().zip(&want) {
                let got = index.search(q, 10, ef).unwrap();
                found += got.iter().filter(|r| want.contains(&r.doc_key)).count();
            }
            found as f64 / (10 * queries.len()) as f64
        };
        // The codes alone, before any re-scoring.
        let raw = recall(&index, 200);
        assert!(raw >= 0.85, "recall@10 on codes at ef 200: {raw}");

        // V6 round trip keeps the codes.
        let restored = VectorIndex::deserialize(&index.serialize().unwrap()).unwrap();
        assert!(restored.graph_is_quantized());
        assert_eq!(restored.len(), 12_000);
        assert_eq!(recall(&restored, 200), raw);

        // A vector inserted after quantization is coded and found.
        restored.insert("new", &data[3]).unwrap();
        let top = restored.search(&data[3], 2, 64).unwrap();
        assert!(top.iter().any(|r| r.doc_key == "new"), "{top:?}");

        // Back to f32 (decoded from the codes).
        restored.dequantize();
        assert!(!restored.is_quantized());
        assert!(recall(&restored, 200) >= 0.85);
    }

    /// An index quantized before 2.16 saved its codes beside an f32 graph
    /// (V5): loading it moves the codes into the graph.
    #[test]
    fn a_v5_quantized_index_is_upgraded_on_load() {
        #[derive(Serialize)]
        struct V5Out<'a> {
            config: &'a VectorIndexConfig,
            vectors: &'a HashMap<String, Vec<f32>>,
            quantized_vectors: &'a Option<QuantizedVectors>,
            hnsw_graph: &'a Option<Graph>,
            arena: &'a [f32],
        }
        let dim = 16;
        let (data, _, _) = clustered(10_500, 0, dim);
        let config = VectorIndexConfig::new("idx".to_string(), "emb".to_string(), dim);
        let index = VectorIndex::new(config.clone()).unwrap();
        index.insert_many(
            data.iter()
                .enumerate()
                .map(|(i, v)| (format!("d{i}"), v.clone()))
                .collect(),
        );
        let graph = index.hnsw_graph.read().unwrap();
        let pairs: Vec<(String, Vec<f32>)> = graph
            .as_ref()
            .unwrap()
            .live_vectors()
            .map(|(k, v)| (k.to_string(), v.to_vec()))
            .collect();
        let refs: Vec<(&str, &[f32])> = pairs.iter().map(|(k, v)| (k.as_str(), &v[..])).collect();
        let old_codes = Some(QuantizedVectors::from_pairs(&refs, dim));
        let mut bytes = FORMAT_V5.to_vec();
        bincode::serialize_into(
            &mut bytes,
            &V5Out {
                config: &config,
                vectors: &HashMap::new(),
                quantized_vectors: &old_codes,
                hnsw_graph: &graph,
                arena: graph.as_ref().unwrap().arena(),
            },
        )
        .unwrap();
        drop(graph);

        let loaded = VectorIndex::deserialize(&bytes).unwrap();
        assert!(loaded.graph_is_quantized());
        assert!(loaded.quantized_vectors.read().unwrap().is_none());
        assert_eq!(loaded.search(&data[42], 1, 64).unwrap()[0].doc_key, "d42");
        // The streaming reader takes it too.
        let streamed = VectorIndex::deserialize_from(&bytes[..]).unwrap();
        assert!(streamed.graph_is_quantized());
    }

    #[test]
    fn test_vector_index_similarity() {
        let config = VectorIndexConfig::new("test_idx".to_string(), "embedding".to_string(), 3);
        let index = VectorIndex::new(config).unwrap();

        index.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();

        let sim = index.similarity("doc1", &[1.0, 0.0, 0.0]).unwrap();
        assert!(sim.is_some());
        assert!((sim.unwrap() - 1.0).abs() < 1e-6);

        let sim = index.similarity("nonexistent", &[1.0, 0.0, 0.0]).unwrap();
        assert!(sim.is_none());
    }

    #[test]
    fn test_vector_index_serialize_deserialize() {
        let config = VectorIndexConfig::new("test_idx".to_string(), "embedding".to_string(), 3);
        let index = VectorIndex::new(config).unwrap();

        index.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();
        index.insert("doc2", &[0.0, 1.0, 0.0]).unwrap();

        // Serialize
        let bytes = index.serialize().unwrap();

        // The borrowed serializer writes the V6 encoding behind its marker.
        let rest = bytes.strip_prefix(FORMAT_V6).expect("the V6 marker");
        let v6: VectorIndexDataV6 = bincode::deserialize(rest).unwrap();
        assert_eq!(v6.vectors.len(), 2);
        assert!(v6.arena.is_empty());
        assert!(v6.sq_codes.is_empty());

        // Deserialize
        let restored = VectorIndex::deserialize(&bytes).unwrap();
        assert_eq!(restored.len(), 2);
        assert!(restored.contains("doc1"));
        assert!(restored.contains("doc2"));
    }

    /// A batch past the threshold builds the graph once and answers as
    /// inserts one by one do; one under it stays brute force.
    #[test]
    fn a_batch_insert_builds_the_graph_once() {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let config = VectorIndexConfig::new("idx".to_string(), "emb".to_string(), 8);
        let small = VectorIndex::new(config.clone()).unwrap();
        assert_eq!(
            small.insert_many(vec![
                ("a".into(), vec![1.0; 8]),
                ("bad".into(), vec![1.0; 3])
            ]),
            1
        );
        assert!(!small.is_hnsw_active());
        let index = VectorIndex::new(config).unwrap();
        let items: Vec<(String, Vec<f32>)> = (0..12_000)
            .map(|i| {
                (
                    format!("d{i}"),
                    (0..8).map(|_| rng.gen_range(-1.0..1.0)).collect(),
                )
            })
            .collect();
        let q = items[123].1.clone();
        assert_eq!(index.insert_many(items), 12_000);
        assert!(index.is_hnsw_active());
        assert_eq!(index.len(), 12_000);
        assert_eq!(index.search(&q, 1, 64).unwrap()[0].doc_key, "d123");
    }

    /// An index written by 2.14.0 or earlier (V3, no marker) is still read.
    #[test]
    fn a_v3_index_is_still_read() {
        let config = VectorIndexConfig::new("idx".to_string(), "emb".to_string(), 3);
        let mut vectors = HashMap::new();
        vectors.insert("a".to_string(), vec![1.0, 0.0, 0.0]);
        vectors.insert("b".to_string(), vec![0.0, 1.0, 0.0]);
        let old = VectorIndexDataV3 {
            config,
            vectors,
            quantized_vectors: None,
            hnsw_graph: None,
        };
        let restored = VectorIndex::deserialize(&bincode::serialize(&old).unwrap()).unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(
            restored.search(&[1.0, 0.0, 0.0], 1, 10).unwrap()[0].doc_key,
            "a"
        );
    }

    /// Card #986: once the graph exists it is the only store of the
    /// vectors; reads, updates and removals go through it, and an index
    /// saved as V4 (the map beside the graph) loads without the map.
    #[test]
    fn the_graph_holds_the_vectors_once() {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let config = VectorIndexConfig::new("idx".to_string(), "emb".to_string(), 8);
        let index = VectorIndex::new(config.clone()).unwrap();
        let mut kept = HashMap::new();
        for i in 0..10_050 {
            let v: Vec<f32> = (0..8).map(|_| rng.gen_range(-1.0..1.0)).collect();
            index.insert(&format!("d{i}"), &v).unwrap();
            kept.insert(format!("d{i}"), v);
        }
        assert!(index.is_hnsw_active());
        assert!(index.vectors.read().unwrap().is_empty());
        assert_eq!(index.len(), 10_050);
        // A cosine index gives a vector back normalised: same direction.
        let q = &kept["d77"];
        assert!((index.similarity("d77", q).unwrap().unwrap() - 1.0).abs() < 1e-5);
        assert!(index.contains("d77"));
        // Replace, remove, insert past the graph.
        index.insert("d77", &[1.0; 8]).unwrap();
        assert!((index.similarity("d77", &[2.0; 8]).unwrap().unwrap() - 1.0).abs() < 1e-5);
        assert!(index.remove("d78").unwrap());
        assert!(!index.remove("d78").unwrap());
        assert!(!index.contains("d78"));
        index.insert("new", &[0.5; 8]).unwrap();
        assert_eq!(index.len(), 10_050);
        assert!(index.vectors.read().unwrap().is_empty());
        let first = |i: &VectorIndex| i.search(&kept["d500"], 1, 64).unwrap()[0].doc_key.clone();

        // An index saved by 2.14.x: the map beside the graph.
        let graph = index.hnsw_graph.read().unwrap();
        let mut map = HashMap::new();
        for (key, v) in graph.as_ref().unwrap().live_vectors() {
            map.insert(key.to_string(), v.to_vec());
        }
        let mut v4 = FORMAT_V4.to_vec();
        bincode::serialize_into(
            &mut v4,
            &VectorIndexDataV4Ref {
                config: &config,
                vectors: &map,
                quantized_vectors: &None,
                hnsw_graph: &graph,
            },
        )
        .unwrap();
        drop(graph);
        let old = VectorIndex::deserialize(&v4).unwrap();
        assert!(old.vectors.read().unwrap().is_empty());
        assert_eq!(old.len(), 10_050);
        assert_eq!(first(&old), "d500");
        // Normalised again on the way in: equal to the last bit or so.
        let (a, b) = (old.get("new").unwrap(), index.get("new").unwrap());
        assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6));

        // Quantized, then a V6 round trip: the codes come back as they were.
        assert_eq!(index.quantize().unwrap(), 10_050);
        let restored = VectorIndex::deserialize(&index.serialize().unwrap()).unwrap();
        assert_eq!(restored.len(), 10_050);
        assert!(!restored.contains("d78"));
        assert_eq!(restored.get("new"), index.get("new"));
        assert_eq!(first(&restored), "d500");
    }

    /// A graph built past the threshold, saved and loaded, answers as before.
    #[test]
    fn a_saved_graph_answers_after_a_load() {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let config = VectorIndexConfig::new("idx".to_string(), "emb".to_string(), 8);
        let index = VectorIndex::new(config).unwrap();
        for i in 0..10_050 {
            let v: Vec<f32> = (0..8).map(|_| rng.gen_range(-1.0..1.0)).collect();
            index.insert(&format!("d{i}"), &v).unwrap();
        }
        assert!(index.is_hnsw_active());
        let q = index.get("d77").unwrap();
        let before: Vec<String> = index
            .search(&q, 5, 64)
            .unwrap()
            .into_iter()
            .map(|r| r.doc_key)
            .collect();
        let restored = VectorIndex::deserialize(&index.serialize().unwrap()).unwrap();
        assert!(restored.is_hnsw_active());
        let after: Vec<String> = restored
            .search(&q, 5, 64)
            .unwrap()
            .into_iter()
            .map(|r| r.doc_key)
            .collect();
        assert_eq!(before, after);
        assert_eq!(after[0], "d77");
    }

    #[test]
    fn test_vector_index_update() {
        let config = VectorIndexConfig::new("test_idx".to_string(), "embedding".to_string(), 3);
        let index = VectorIndex::new(config).unwrap();

        index.insert("doc1", &[1.0, 0.0, 0.0]).unwrap();

        // Update the vector
        index.insert("doc1", &[0.0, 1.0, 0.0]).unwrap();

        // Should still have only one entry
        assert_eq!(index.len(), 1);

        // Check the vector was updated
        let vec = index.get("doc1").unwrap();
        assert!((vec[0] - 0.0).abs() < 1e-6);
        assert!((vec[1] - 1.0).abs() < 1e-6);
    }
}
