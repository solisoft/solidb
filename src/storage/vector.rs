//! Vector index module for similarity search.
//!
//! This module provides vector similarity search capabilities for documents
//! containing embedding vectors. Currently uses brute-force search which is
//! accurate and fast for moderate dataset sizes (< 100K vectors).
//!
//! TODO: Add HNSW approximate nearest neighbor search for larger datasets.

use crate::error::{DbError, DbResult};
use crate::storage::index::{VectorIndexConfig, VectorMetric};
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

/// Vector index for similarity search
pub struct VectorIndex {
    /// Index configuration
    config: VectorIndexConfig,
    /// Stored vectors: doc_key -> vector
    vectors: RwLock<HashMap<String, Vec<f32>>>,
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
        })
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

        let mut vectors = self.vectors.write().unwrap();
        vectors.insert(doc_key.to_string(), vector.to_vec());
        Ok(())
    }

    /// Remove a vector by document key
    pub fn remove(&self, doc_key: &str) -> DbResult<bool> {
        let mut vectors = self.vectors.write().unwrap();
        Ok(vectors.remove(doc_key).is_some())
    }

    /// Check if a document has a vector in the index
    pub fn contains(&self, doc_key: &str) -> bool {
        self.vectors.read().unwrap().contains_key(doc_key)
    }

    /// Get a vector by document key
    pub fn get(&self, doc_key: &str) -> Option<Vec<f32>> {
        self.vectors.read().unwrap().get(doc_key).cloned()
    }

    /// Search for similar vectors using brute-force
    ///
    /// # Arguments
    /// * `query` - Query vector
    /// * `limit` - Maximum number of results to return
    /// * `_ef` - Unused (reserved for HNSW search quality parameter)
    ///
    /// # Returns
    /// Vector of search results sorted by similarity (best first)
    pub fn search(&self, query: &[f32], limit: usize, _ef: usize) -> DbResult<Vec<VectorSearchResult>> {
        // Validate dimension
        if query.len() != self.config.dimension {
            return Err(DbError::BadRequest(format!(
                "Query vector dimension mismatch: expected {}, got {}",
                self.config.dimension,
                query.len()
            )));
        }

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
        let data = VectorIndexData {
            config: self.config.clone(),
            vectors: vectors.clone(),
        };
        bincode::serialize(&data).map_err(|e| DbError::InternalError(format!("Serialization error: {}", e)))
    }

    /// Deserialize a vector index from bytes
    pub fn deserialize(bytes: &[u8]) -> DbResult<Self> {
        let data: VectorIndexData =
            bincode::deserialize(bytes).map_err(|e| DbError::InternalError(format!("Deserialization error: {}", e)))?;

        Ok(Self {
            config: data.config,
            vectors: RwLock::new(data.vectors),
        })
    }
}

/// Serializable vector index data
#[derive(Serialize, Deserialize)]
struct VectorIndexData {
    config: VectorIndexConfig,
    vectors: HashMap<String, Vec<f32>>,
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
