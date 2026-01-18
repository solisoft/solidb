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
use crate::storage::index::{VectorIndexConfig, VectorIndexStats, VectorMetric, VectorQuantization};
use rand::prelude::*;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
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

        for vec in vectors.values() {
            for (i, &v) in vec.iter().enumerate() {
                min_vals[i] = min_vals[i].min(v);
                max_vals[i] = max_vals[i].max(v);
            }
        }

        // Quantize all vectors
        let quantized: HashMap<String, Vec<u8>> = vectors
            .iter()
            .map(|(k, v)| (k.clone(), Self::quantize_vector(v, &min_vals, &max_vals)))
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
// HNSW Data Structures
// =============================================================================

/// HNSW node representing a vector in the graph
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HnswNode {
    /// Connections at each level: level -> vec of neighbor doc_keys
    neighbors: Vec<Vec<String>>,
    /// Level this node was assigned during insertion
    level: usize,
}

/// HNSW graph structure for approximate nearest neighbor search
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HnswGraph {
    /// All nodes in the graph: doc_key -> HnswNode
    nodes: HashMap<String, HnswNode>,
    /// Current entry point (highest level node)
    entry_point: Option<String>,
    /// Current maximum level in the graph
    max_level: usize,
    /// M parameter: max connections per node per level (except level 0)
    m: usize,
    /// M0 parameter: max connections at level 0 (typically 2*M)
    m0: usize,
    /// ef_construction: search quality during build
    ef_construction: usize,
    /// Level multiplier: 1/ln(M), used for random level assignment
    level_mult: f64,
}

/// Neighbor entry for priority queue operations during HNSW search
#[derive(Clone)]
struct Neighbor {
    doc_key: String,
    distance: f32,
}

impl PartialEq for Neighbor {
    fn eq(&self, other: &Self) -> bool {
        self.distance == other.distance
    }
}

impl Eq for Neighbor {}

impl PartialOrd for Neighbor {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Neighbor {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering: smaller distance = higher priority (max-heap becomes min-heap)
        other
            .distance
            .partial_cmp(&self.distance)
            .unwrap_or(Ordering::Equal)
    }
}

/// Wrapper for max-heap behavior (largest distance first)
#[derive(Clone)]
struct MaxNeighbor(Neighbor);

impl PartialEq for MaxNeighbor {
    fn eq(&self, other: &Self) -> bool {
        self.0.distance == other.0.distance
    }
}

impl Eq for MaxNeighbor {}

impl PartialOrd for MaxNeighbor {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MaxNeighbor {
    fn cmp(&self, other: &Self) -> Ordering {
        // Normal ordering: larger distance = higher priority
        self.0
            .distance
            .partial_cmp(&other.0.distance)
            .unwrap_or(Ordering::Equal)
    }
}

// =============================================================================
// HnswGraph Implementation
// =============================================================================

impl HnswGraph {
    /// Create a new HNSW graph with the given configuration
    fn new(m: usize, ef_construction: usize) -> Self {
        let m = m.max(2); // Minimum M of 2
        Self {
            nodes: HashMap::new(),
            entry_point: None,
            max_level: 0,
            m,
            m0: m * 2, // Level 0 has double the connections
            ef_construction,
            level_mult: 1.0 / (m as f64).ln(),
        }
    }

    /// Generate a random level for a new node using exponential distribution
    fn random_level(&self) -> usize {
        let mut rng = rand::thread_rng();
        let r: f64 = rng.gen();
        let level = (-r.ln() * self.level_mult).floor() as usize;
        level.min(16) // Cap at reasonable max level
    }

    /// Calculate distance between query and a document's vector
    fn calculate_distance(
        query: &[f32],
        doc_key: &str,
        vectors: &HashMap<String, Vec<f32>>,
        metric: VectorMetric,
    ) -> f32 {
        match vectors.get(doc_key) {
            Some(vec) => match metric {
                // For HNSW we need distance (lower is better), so convert similarity
                VectorMetric::Cosine => 1.0 - cosine_similarity(query, vec),
                VectorMetric::Euclidean => euclidean_distance(query, vec),
                VectorMetric::DotProduct => -dot_product(query, vec), // Negate for "lower is better"
            },
            None => f32::MAX,
        }
    }

    /// Search within a single layer, returning up to ef nearest neighbors
    fn search_layer(
        &self,
        query: &[f32],
        entry_points: &[String],
        ef: usize,
        level: usize,
        vectors: &HashMap<String, Vec<f32>>,
        metric: VectorMetric,
    ) -> Vec<Neighbor> {
        let mut visited: HashSet<String> = HashSet::new();
        // Min-heap: best candidates (smallest distance first)
        let mut candidates: BinaryHeap<Neighbor> = BinaryHeap::new();
        // Max-heap: worst of the current best (largest distance first)
        let mut result: BinaryHeap<MaxNeighbor> = BinaryHeap::new();

        // Initialize with entry points
        for ep in entry_points {
            if visited.insert(ep.clone()) {
                let dist = Self::calculate_distance(query, ep, vectors, metric);
                let neighbor = Neighbor {
                    doc_key: ep.clone(),
                    distance: dist,
                };
                candidates.push(neighbor.clone());
                result.push(MaxNeighbor(neighbor));
            }
        }

        while let Some(current) = candidates.pop() {
            // Get the furthest element in result
            let furthest_dist = result.peek().map(|n| n.0.distance).unwrap_or(f32::MAX);

            // If current is further than the furthest result, we're done
            if current.distance > furthest_dist {
                break;
            }

            // Get neighbors at this level
            if let Some(node) = self.nodes.get(&current.doc_key) {
                if level < node.neighbors.len() {
                    for neighbor_key in &node.neighbors[level] {
                        if visited.insert(neighbor_key.clone()) {
                            let dist =
                                Self::calculate_distance(query, neighbor_key, vectors, metric);
                            let furthest_dist =
                                result.peek().map(|n| n.0.distance).unwrap_or(f32::MAX);

                            if dist < furthest_dist || result.len() < ef {
                                let neighbor = Neighbor {
                                    doc_key: neighbor_key.clone(),
                                    distance: dist,
                                };
                                candidates.push(neighbor.clone());
                                result.push(MaxNeighbor(neighbor));

                                // Keep only ef best
                                while result.len() > ef {
                                    result.pop();
                                }
                            }
                        }
                    }
                }
            }
        }

        // Convert result to vec, sorted by distance (best first)
        let mut results: Vec<Neighbor> = result.into_iter().map(|mn| mn.0).collect();
        results.sort_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(Ordering::Equal)
        });
        results
    }

    /// Select M best neighbors using the simple heuristic
    fn select_neighbors(&self, candidates: &[Neighbor], m: usize) -> Vec<String> {
        // Simple heuristic: just take the M closest
        candidates
            .iter()
            .take(m)
            .map(|n| n.doc_key.clone())
            .collect()
    }

    /// Insert a new vector into the graph
    fn insert(
        &mut self,
        doc_key: &str,
        _vector: &[f32],
        vectors: &HashMap<String, Vec<f32>>,
        metric: VectorMetric,
    ) {
        // If this key already exists, remove it first (update case)
        if self.nodes.contains_key(doc_key) {
            self.remove(doc_key);
        }

        // Get the vector for distance calculations
        let query = match vectors.get(doc_key) {
            Some(v) => v,
            None => return, // Vector not found, skip
        };

        // Assign random level to this node
        let node_level = self.random_level();

        // Create the new node with empty neighbor lists
        let mut new_node = HnswNode {
            neighbors: vec![Vec::new(); node_level + 1],
            level: node_level,
        };

        // If graph is empty, this becomes the entry point
        if self.entry_point.is_none() {
            self.entry_point = Some(doc_key.to_string());
            self.max_level = node_level;
            self.nodes.insert(doc_key.to_string(), new_node);
            return;
        }

        let entry_point = self.entry_point.clone().unwrap();
        let mut current_ep = vec![entry_point];

        // Phase 1: Descend from top to node_level+1, using greedy search (ef=1)
        for level in (node_level + 1..=self.max_level).rev() {
            let nearest = self.search_layer(query, &current_ep, 1, level, vectors, metric);
            if !nearest.is_empty() {
                current_ep = vec![nearest[0].doc_key.clone()];
            }
        }

        // Phase 2: From node_level down to 0, find and connect neighbors
        for level in (0..=node_level.min(self.max_level)).rev() {
            let m_level = if level == 0 { self.m0 } else { self.m };

            // Search for nearest neighbors at this level
            let candidates = self.search_layer(
                query,
                &current_ep,
                self.ef_construction,
                level,
                vectors,
                metric,
            );

            // Select best neighbors
            let selected = self.select_neighbors(&candidates, m_level);

            // Add connections from new node to selected neighbors
            if level < new_node.neighbors.len() {
                new_node.neighbors[level] = selected.clone();
            }

            // Add reverse connections from neighbors to new node
            for neighbor_key in &selected {
                if let Some(neighbor_node) = self.nodes.get_mut(neighbor_key) {
                    if level < neighbor_node.neighbors.len() {
                        neighbor_node.neighbors[level].push(doc_key.to_string());

                        // Prune if exceeds limit
                        if neighbor_node.neighbors[level].len() > m_level {
                            // Keep only the best M neighbors
                            let mut neighbor_candidates: Vec<Neighbor> = neighbor_node.neighbors
                                [level]
                                .iter()
                                .filter_map(|k| {
                                    vectors.get(neighbor_key).map(|nv| Neighbor {
                                        doc_key: k.clone(),
                                        distance: Self::calculate_distance(nv, k, vectors, metric),
                                    })
                                })
                                .collect();
                            neighbor_candidates.sort_by(|a, b| {
                                a.distance
                                    .partial_cmp(&b.distance)
                                    .unwrap_or(Ordering::Equal)
                            });
                            // Select best M neighbors (inline to avoid borrow conflict)
                            neighbor_node.neighbors[level] = neighbor_candidates
                                .iter()
                                .take(m_level)
                                .map(|n| n.doc_key.clone())
                                .collect();
                        }
                    }
                }
            }

            // Update entry points for next level
            if !candidates.is_empty() {
                current_ep = candidates.iter().map(|n| n.doc_key.clone()).collect();
            }
        }

        // Insert the new node
        self.nodes.insert(doc_key.to_string(), new_node);

        // Update entry point if new node has higher level
        if node_level > self.max_level {
            self.entry_point = Some(doc_key.to_string());
            self.max_level = node_level;
        }
    }

    /// Remove a node from the graph (lazy removal - just delete the node)
    fn remove(&mut self, doc_key: &str) {
        // Remove the node
        if let Some(removed_node) = self.nodes.remove(doc_key) {
            // Remove references from neighbors
            for level in 0..removed_node.neighbors.len() {
                for neighbor_key in &removed_node.neighbors[level] {
                    if let Some(neighbor_node) = self.nodes.get_mut(neighbor_key) {
                        if level < neighbor_node.neighbors.len() {
                            neighbor_node.neighbors[level].retain(|k| k != doc_key);
                        }
                    }
                }
            }

            // Update entry point if removed
            if self.entry_point.as_ref() == Some(&doc_key.to_string()) {
                // Find a new entry point (any node at max_level, or reduce max_level)
                self.entry_point = self.nodes.keys().next().cloned();
                if let Some(ep) = &self.entry_point {
                    if let Some(node) = self.nodes.get(ep) {
                        self.max_level = node.level;
                    }
                } else {
                    self.max_level = 0;
                }
            }
        }
    }

    /// Search for k nearest neighbors
    fn search(
        &self,
        query: &[f32],
        k: usize,
        ef: usize,
        vectors: &HashMap<String, Vec<f32>>,
        metric: VectorMetric,
    ) -> Vec<Neighbor> {
        if self.entry_point.is_none() {
            return vec![];
        }

        let entry_point = self.entry_point.clone().unwrap();
        let mut current_ep = vec![entry_point];

        // Descend from top level to level 1 using greedy search
        for level in (1..=self.max_level).rev() {
            let nearest = self.search_layer(query, &current_ep, 1, level, vectors, metric);
            if !nearest.is_empty() {
                current_ep = vec![nearest[0].doc_key.clone()];
            }
        }

        // Search at level 0 with ef
        let ef_search = ef.max(k);
        let mut results = self.search_layer(query, &current_ep, ef_search, 0, vectors, metric);

        // Return top k
        results.truncate(k);
        results
    }

    /// Clear all nodes from the graph
    fn clear(&mut self) {
        self.nodes.clear();
        self.entry_point = None;
        self.max_level = 0;
    }
}

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
pub struct VectorIndex {
    /// Index configuration
    config: VectorIndexConfig,
    /// Stored vectors: doc_key -> vector (full precision)
    vectors: RwLock<HashMap<String, Vec<f32>>>,
    /// Quantized vectors for memory-efficient storage (optional)
    quantized_vectors: RwLock<Option<QuantizedVectors>>,
    /// HNSW graph for approximate search (built when threshold exceeded)
    hnsw_graph: RwLock<Option<HnswGraph>>,
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
        let vectors = self.vectors.read().unwrap();
        if vectors.len() < self.hnsw_threshold() {
            return;
        }

        let mut graph = HnswGraph::new(self.config.m, self.config.ef_construction);

        // Insert all vectors into the graph
        for (doc_key, vector) in vectors.iter() {
            graph.insert(doc_key, vector, &vectors, self.config.metric);
        }

        // Store the graph
        let mut hnsw = self.hnsw_graph.write().unwrap();
        *hnsw = Some(graph);
    }

    /// Get the index configuration
    pub fn config(&self) -> &VectorIndexConfig {
        &self.config
    }

    /// Get the number of indexed vectors
    pub fn len(&self) -> usize {
        self.vectors.read().unwrap().len()
    }

    /// Check if the index is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Clear all vectors from the index
    pub fn clear(&self) {
        let mut vectors = self.vectors.write().unwrap();
        vectors.clear();

        // Also clear quantized vectors
        let mut quantized = self.quantized_vectors.write().unwrap();
        if let Some(q) = quantized.as_mut() {
            q.clear();
        }
        *quantized = None;

        // Also clear HNSW graph
        let mut hnsw = self.hnsw_graph.write().unwrap();
        if let Some(graph) = hnsw.as_mut() {
            graph.clear();
        }
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

        // Insert into vectors HashMap
        {
            let mut vectors = self.vectors.write().unwrap();
            vectors.insert(doc_key.to_string(), vector.to_vec());
        }

        // Also insert into quantized vectors if they exist
        {
            let mut quantized = self.quantized_vectors.write().unwrap();
            if let Some(q) = quantized.as_mut() {
                q.insert(doc_key, vector);
            }
        }

        // Update HNSW graph if it exists or threshold is reached
        let vectors = self.vectors.read().unwrap();
        let count = vectors.len();
        drop(vectors);

        let mut hnsw = self.hnsw_graph.write().unwrap();
        if let Some(graph) = hnsw.as_mut() {
            // Graph exists, insert into it
            let vectors = self.vectors.read().unwrap();
            graph.insert(doc_key, vector, &vectors, self.config.metric);
        } else if count >= self.hnsw_threshold() {
            // Threshold reached, build the graph
            drop(hnsw);
            self.build_hnsw_graph();
        }

        Ok(())
    }

    /// Remove a vector by document key
    pub fn remove(&self, doc_key: &str) -> DbResult<bool> {
        // Remove from vectors HashMap
        let removed = {
            let mut vectors = self.vectors.write().unwrap();
            vectors.remove(doc_key).is_some()
        };

        if removed {
            // Also remove from quantized vectors if they exist
            {
                let mut quantized = self.quantized_vectors.write().unwrap();
                if let Some(q) = quantized.as_mut() {
                    q.remove(doc_key);
                }
            }

            // Also remove from HNSW graph if it exists
            let mut hnsw = self.hnsw_graph.write().unwrap();
            if let Some(graph) = hnsw.as_mut() {
                graph.remove(doc_key);
            }
        }

        Ok(removed)
    }

    /// Check if a document has a vector in the index
    pub fn contains(&self, doc_key: &str) -> bool {
        self.vectors.read().unwrap().contains_key(doc_key)
    }

    /// Get a vector by document key
    pub fn get(&self, doc_key: &str) -> Option<Vec<f32>> {
        self.vectors.read().unwrap().get(doc_key).cloned()
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
            let hnsw = self.hnsw_graph.read().unwrap();
            if let Some(graph) = hnsw.as_ref() {
                let vectors = self.vectors.read().unwrap();
                let ef_search = ef.max(limit * 2).max(40);
                let hnsw_results =
                    graph.search(query, limit, ef_search, &vectors, self.config.metric);

                // Convert HNSW results (distance) back to score format
                let results: Vec<VectorSearchResult> = hnsw_results
                    .into_iter()
                    .map(|n| {
                        // Convert distance back to original score
                        let score = match self.config.metric {
                            VectorMetric::Cosine => 1.0 - n.distance, // distance was 1-similarity
                            VectorMetric::Euclidean => n.distance,    // keep as distance
                            VectorMetric::DotProduct => -n.distance,  // distance was -dot_product
                        };
                        VectorSearchResult {
                            doc_key: n.doc_key,
                            score,
                        }
                    })
                    .collect();

                return Ok(results);
            }
        }

        // Fall back to brute-force search
        self.brute_force_search(query, limit)
    }

    /// Perform brute-force linear search
    fn brute_force_search(&self, query: &[f32], limit: usize) -> DbResult<Vec<VectorSearchResult>> {
        // Check if we have quantized vectors - use asymmetric search if so
        {
            let quantized = self.quantized_vectors.read().unwrap();
            if let Some(q) = quantized.as_ref() {
                return self.brute_force_search_quantized(query, limit, q);
            }
        }

        // Fall back to full-precision search
        let vectors = self.vectors.read().unwrap();

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

        let vectors = self.vectors.read().unwrap();
        let doc_vec = match vectors.get(doc_key) {
            Some(v) => v,
            None => return Ok(None),
        };

        let score = match self.config.metric {
            VectorMetric::Cosine => cosine_similarity(query, doc_vec),
            VectorMetric::Euclidean => euclidean_distance(query, doc_vec),
            VectorMetric::DotProduct => dot_product(query, doc_vec),
        };

        Ok(Some(score))
    }

    /// Serialize the index to bytes for persistence
    pub fn serialize(&self) -> DbResult<Vec<u8>> {
        let vectors = self.vectors.read().unwrap();
        let quantized = self.quantized_vectors.read().unwrap();
        let hnsw = self.hnsw_graph.read().unwrap();

        let data = VectorIndexDataV3 {
            config: self.config.clone(),
            vectors: vectors.clone(),
            quantized_vectors: quantized.clone(),
            hnsw_graph: hnsw.clone(),
        };
        bincode::serialize(&data)
            .map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))
    }

    /// Deserialize a vector index from bytes
    pub fn deserialize(bytes: &[u8]) -> DbResult<Self> {
        // Try V3 format first (with quantization)
        if let Ok(data) = bincode::deserialize::<VectorIndexDataV3>(bytes) {
            return Ok(Self {
                config: data.config,
                vectors: RwLock::new(data.vectors),
                quantized_vectors: RwLock::new(data.quantized_vectors),
                hnsw_graph: RwLock::new(data.hnsw_graph),
            });
        }

        // Try V2 format (with HNSW graph)
        if let Ok(data) = bincode::deserialize::<VectorIndexDataV2>(bytes) {
            return Ok(Self {
                config: data.config,
                vectors: RwLock::new(data.vectors),
                quantized_vectors: RwLock::new(None),
                hnsw_graph: RwLock::new(data.hnsw_graph),
            });
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

    /// Check if HNSW graph is currently active
    pub fn is_hnsw_active(&self) -> bool {
        self.hnsw_graph.read().unwrap().is_some()
    }

    /// Check if scalar quantization is active
    pub fn is_quantized(&self) -> bool {
        self.quantized_vectors.read().unwrap().is_some()
    }

    /// Get the current quantization type
    pub fn quantization_type(&self) -> VectorQuantization {
        if self.quantized_vectors.read().unwrap().is_some() {
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
        let vectors = self.vectors.read().unwrap();
        let count = vectors.len();

        if count == 0 {
            return Ok(0);
        }

        // Build quantized storage from full-precision vectors
        let quantized = QuantizedVectors::from_full_vectors(&vectors, self.config.dimension);
        drop(vectors);

        // Store the quantized vectors
        let mut quantized_lock = self.quantized_vectors.write().unwrap();
        *quantized_lock = Some(quantized);

        Ok(count)
    }

    /// Remove quantization (revert to full-precision search)
    pub fn dequantize(&self) {
        let mut quantized = self.quantized_vectors.write().unwrap();
        *quantized = None;
    }

    /// Get quantization statistics
    pub fn quantization_stats(&self) -> Option<QuantizationStats> {
        let quantized = self.quantized_vectors.read().unwrap();
        quantized.as_ref().map(|q| {
            let vector_count = q.len();
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
        let (memory_bytes, compression_ratio) = 
            if let Some(stats) = self.quantization_stats() {
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
    hnsw_graph: Option<HnswGraph>,
}

/// Serializable vector index data (V3 format - with quantization)
#[derive(Serialize, Deserialize)]
struct VectorIndexDataV3 {
    config: VectorIndexConfig,
    vectors: HashMap<String, Vec<f32>>,
    #[serde(default)]
    quantized_vectors: Option<QuantizedVectors>,
    #[serde(default)]
    hnsw_graph: Option<HnswGraph>,
}

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

        // Deserialize
        let restored = VectorIndex::deserialize(&bytes).unwrap();
        assert_eq!(restored.len(), 2);
        assert!(restored.contains("doc1"));
        assert!(restored.contains("doc2"));
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
