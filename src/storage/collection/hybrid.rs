use super::*;
use crate::error::DbResult;
use serde::Serialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Score fusion method for hybrid search
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FusionMethod {
    #[default]
    Weighted,
    Rrf,
}

impl FusionMethod {
    /// Strict parse: only the documented values. Callers that want the
    /// historical lenient behavior (anything unknown means weighted) can
    /// fall back with `unwrap_or_default()`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "weighted" => Some(FusionMethod::Weighted),
            "rrf" => Some(FusionMethod::Rrf),
            _ => None,
        }
    }
}

/// Options for hybrid search (defaults match the documented SDBQL/REST defaults)
#[derive(Debug, Clone)]
pub struct HybridSearchOptions {
    pub vector_weight: f32,
    pub text_weight: f32,
    pub limit: usize,
    pub fusion: FusionMethod,
}

impl Default for HybridSearchOptions {
    fn default() -> Self {
        Self {
            vector_weight: 0.5,
            text_weight: 0.5,
            limit: 10,
            fusion: FusionMethod::Weighted,
        }
    }
}

/// One hybrid search hit. `vector_score`/`text_score` are the ORIGINAL
/// (non-normalized) per-source scores and are `None` when that source
/// didn't match the document.
#[derive(Debug, Clone, Serialize)]
pub struct HybridSearchResult {
    pub doc_key: String,
    pub score: f32,
    pub vector_score: Option<f32>,
    pub text_score: Option<f32>,
    pub sources: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<Value>,
}

impl Collection {
    // ==================== Hybrid Search ====================

    /// Combined vector + fulltext search with score fusion.
    ///
    /// Both legs over-fetch `limit * 3` candidates, scores are min-max
    /// normalized to [0,1], fused (weighted sum or RRF with k=60), sorted
    /// descending and truncated to `limit`. Documents are hydrated via
    /// `get()`; a hit whose document vanished keeps `document: None`.
    pub fn hybrid_search(
        &self,
        vector_index: &str,
        fulltext_field: &str,
        query_vector: &[f32],
        text_query: &str,
        opts: &HybridSearchOptions,
    ) -> DbResult<Vec<HybridSearchResult>> {
        let limit = opts.limit;

        // Step 1: Vector search (get more candidates than limit for better fusion)
        let vector_results = self.vector_search(vector_index, query_vector, limit * 3, None)?;

        // Step 2: Fulltext search (same over-fetch as the vector leg)
        let fulltext_results = self
            .fulltext_search(
                text_query,
                Some(vec![fulltext_field.to_string()]),
                limit * 3,
            )
            .unwrap_or_default();

        // Step 3: Normalize vector scores to 0-1 range
        let mut vector_scores: HashMap<String, f32> = HashMap::new();
        if !vector_results.is_empty() {
            let max_vec = vector_results
                .iter()
                .map(|r| r.score)
                .fold(f32::NEG_INFINITY, f32::max);
            let min_vec = vector_results
                .iter()
                .map(|r| r.score)
                .fold(f32::INFINITY, f32::min);
            let range = max_vec - min_vec;
            for result in &vector_results {
                let normalized = if range > 0.0 {
                    (result.score - min_vec) / range
                } else {
                    1.0
                };
                vector_scores.insert(result.doc_key.clone(), normalized);
            }
        }

        // Step 4: Normalize text scores to 0-1 range
        let mut text_scores: HashMap<String, f32> = HashMap::new();
        if !fulltext_results.is_empty() {
            let max_text = fulltext_results
                .iter()
                .map(|r| r.score)
                .fold(f64::NEG_INFINITY, f64::max);
            let min_text = fulltext_results
                .iter()
                .map(|r| r.score)
                .fold(f64::INFINITY, f64::min);
            let range = max_text - min_text;
            for result in &fulltext_results {
                let normalized = if range > 0.0 {
                    ((result.score - min_text) / range) as f32
                } else {
                    1.0
                };
                text_scores.insert(result.doc_key.clone(), normalized);
            }
        }

        // Original (non-normalized) per-source scores for output
        let orig_vector_scores: HashMap<String, f32> = vector_results
            .iter()
            .map(|r| (r.doc_key.clone(), r.score))
            .collect();
        let orig_text_scores: HashMap<String, f32> = fulltext_results
            .iter()
            .map(|r| (r.doc_key.clone(), r.score as f32))
            .collect();

        // Step 5: Combine scores based on fusion method
        #[allow(clippy::type_complexity)]
        let mut combined_results: Vec<(
            String,
            f32,
            Option<f32>,
            Option<f32>,
            Vec<String>,
        )> = Vec::new();

        match opts.fusion {
            FusionMethod::Rrf => {
                // Reciprocal Rank Fusion
                let k: f32 = 60.0;
                let mut rrf_scores: HashMap<String, f32> = HashMap::new();
                let mut doc_sources: HashMap<String, Vec<String>> = HashMap::new();

                for (rank, result) in vector_results.iter().enumerate() {
                    let rrf_score = 1.0 / (k + rank as f32 + 1.0);
                    *rrf_scores.entry(result.doc_key.clone()).or_insert(0.0) += rrf_score;
                    doc_sources
                        .entry(result.doc_key.clone())
                        .or_default()
                        .push("vector".to_string());
                }

                for (rank, result) in fulltext_results.iter().enumerate() {
                    let rrf_score = 1.0 / (k + rank as f32 + 1.0);
                    *rrf_scores.entry(result.doc_key.clone()).or_insert(0.0) += rrf_score;
                    doc_sources
                        .entry(result.doc_key.clone())
                        .or_default()
                        .push("fulltext".to_string());
                }

                for (doc_key, score) in rrf_scores {
                    let sources = doc_sources.remove(&doc_key).unwrap_or_default();
                    let vec_score = orig_vector_scores.get(&doc_key).copied();
                    let txt_score = orig_text_scores.get(&doc_key).copied();
                    combined_results.push((doc_key, score, vec_score, txt_score, sources));
                }
            }
            FusionMethod::Weighted => {
                let mut all_doc_keys: HashSet<String> = HashSet::new();
                all_doc_keys.extend(vector_scores.keys().cloned());
                all_doc_keys.extend(text_scores.keys().cloned());

                for doc_key in all_doc_keys {
                    let vec_score = vector_scores.get(&doc_key).copied();
                    let txt_score = text_scores.get(&doc_key).copied();

                    let mut sources = Vec::new();
                    let mut combined_score = 0.0;

                    if let Some(vs) = vec_score {
                        combined_score += vs * opts.vector_weight;
                        sources.push("vector".to_string());
                    }
                    if let Some(ts) = txt_score {
                        combined_score += ts * opts.text_weight;
                        sources.push("fulltext".to_string());
                    }

                    combined_results.push((
                        doc_key.clone(),
                        combined_score,
                        orig_vector_scores.get(&doc_key).copied(),
                        orig_text_scores.get(&doc_key).copied(),
                        sources,
                    ));
                }
            }
        }

        // Step 6: Sort by combined score and limit
        combined_results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        combined_results.truncate(limit);

        // Step 7: Hydrate documents
        Ok(combined_results
            .into_iter()
            .map(
                |(doc_key, score, vector_score, text_score, sources)| HybridSearchResult {
                    document: self.get(&doc_key).ok().map(|d| d.to_value()),
                    doc_key,
                    score,
                    vector_score,
                    text_score,
                    sources,
                },
            )
            .collect())
    }
}
