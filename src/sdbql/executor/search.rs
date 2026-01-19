//! Search and vector functions for SDBQL executor.
//!
//! This module contains search-related functions that require executor context:
//! - Vector search and operations
//! - Fulltext search
//! - Hybrid search
//! - Document sampling and lookup

use serde_json::{json, Value};
use std::collections::HashMap;

use crate::error::{DbError, DbResult};
use crate::sdbql::ast::Expression;

use super::types::Context;
use super::utils::number_from_f64;
use super::QueryExecutor;

/// Extension trait for search operations on QueryExecutor
pub trait SearchOperations<'a> {
    /// Evaluate search-related functions
    fn evaluate_search_function(
        &self,
        name: &str,
        args: &[Expression],
        ctx: &Context,
    ) -> DbResult<Value>;
}

impl<'a> SearchOperations<'a> for QueryExecutor<'a> {
    fn evaluate_search_function(
        &self,
        name: &str,
        args: &[Expression],
        ctx: &Context,
    ) -> DbResult<Value> {
        // Evaluate all arguments
        let evaluated_args: Vec<Value> = args
            .iter()
            .map(|arg| self.evaluate_expr_with_context(arg, ctx))
            .collect::<DbResult<Vec<_>>>()?;

        match name.to_uppercase().as_str() {
            // VECTOR_DISTANCE(collection, index_name, query_vector, target_vector) - calculate distance
            "VECTOR_DISTANCE" | "VECTOR_SIMILARITY" => {
                if evaluated_args.len() != 4 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_DISTANCE requires 4 arguments: collection, index_name, query_vector, target_vector"
                            .to_string(),
                    ));
                }

                let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_DISTANCE: first argument must be a string (collection name)"
                            .to_string(),
                    )
                })?;

                let index_name = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_DISTANCE: second argument must be a string (index name)"
                            .to_string(),
                    )
                })?;

                let query_vec =
                    Self::extract_vector_arg(&evaluated_args[2], "VECTOR_DISTANCE: query_vector")?;
                let target_vec =
                    Self::extract_vector_arg(&evaluated_args[3], "VECTOR_DISTANCE: target_vector")?;

                let collection = self.get_collection(collection_name)?;
                let indexes = collection.list_vector_indexes();

                // Find the index
                let Some(index_info) = indexes.into_iter().find(|idx| idx.name == index_name)
                else {
                    return Err(DbError::ExecutionError(format!(
                        "VECTOR_DISTANCE: index '{}' not found in collection '{}'",
                        index_name, collection_name
                    )));
                };

                // Calculate distance based on metric
                let distance = match index_info.metric {
                    crate::storage::index::VectorMetric::Euclidean => {
                        let mut sum = 0.0f32;
                        for (a, b) in query_vec.iter().zip(target_vec.iter()) {
                            let diff = a - b;
                            sum += diff * diff;
                        }
                        sum.sqrt()
                    }
                    crate::storage::index::VectorMetric::Cosine => {
                        let dot: f32 = query_vec
                            .iter()
                            .zip(target_vec.iter())
                            .map(|(a, b)| a * b)
                            .sum();
                        let mag1: f32 = query_vec.iter().map(|x| x * x).sum::<f32>().sqrt();
                        let mag2: f32 = target_vec.iter().map(|x| x * x).sum::<f32>().sqrt();
                        if mag1 > 0.0 && mag2 > 0.0 {
                            1.0 - (dot / (mag1 * mag2)).clamp(-1.0, 1.0)
                        } else {
                            1.0
                        }
                    }
                    crate::storage::index::VectorMetric::DotProduct => {
                        let dot: f32 = query_vec
                            .iter()
                            .zip(target_vec.iter())
                            .map(|(a, b)| a * b)
                            .sum();
                        // For similarity, negate dot product so lower is closer
                        if name.to_uppercase().as_str() == "VECTOR_SIMILARITY" {
                            -dot
                        } else {
                            -dot
                        }
                    }
                };

                Ok(Value::Number(number_from_f64(distance as f64)))
            }

            // VECTOR_SEARCH(collection, index_name, query_vector, limit, options)
            "VECTOR_SEARCH" | "VECTOR_QUERY" | "ANN_SEARCH" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 5 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_SEARCH requires 3-5 arguments: collection, index_name, query_vector, [limit], [options]"
                            .to_string(),
                    ));
                }

                let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_SEARCH: first argument must be a string (collection name)"
                            .to_string(),
                    )
                })?;

                let index_name = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_SEARCH: second argument must be a string (index name)".to_string(),
                    )
                })?;

                let query_vec =
                    Self::extract_vector_arg(&evaluated_args[2], "VECTOR_SEARCH: query_vector")?;

                let limit = evaluated_args.get(3).and_then(|v| v.as_u64()).unwrap_or(10) as usize;

                let options = evaluated_args.get(4).and_then(|v| v.as_object()).cloned();

                // Parse options
                let mut ef_search = 50usize;
                let _include_vectors = false; // Reserved for future use
                if let Some(opts) = options {
                    if let Some(ef) = opts.get("efSearch").or(opts.get("ef")) {
                        if let Some(v) = ef.as_u64() {
                            ef_search = v as usize;
                        }
                    }
                    // Reserved for future use: include_vectors option
                }

                let collection = self.get_collection(collection_name)?;

                // Perform vector search
                let results =
                    collection.vector_search(index_name, &query_vec, limit * 2, Some(ef_search))?;

                // Format results
                let search_results: Vec<Value> = results
                    .into_iter()
                    .take(limit)
                    .map(|r| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("_key".to_string(), json!(r.doc_key));
                        obj.insert("score".to_string(), json!(r.score));
                        // Note: Vector field not available in search results
                        // Include document data
                        if let Ok(doc) = collection.get(&r.doc_key) {
                            obj.insert("document".to_string(), doc.to_value());
                        }
                        Value::Object(obj)
                    })
                    .collect();

                Ok(Value::Array(search_results))
            }

            // VECTOR_HYBRID_SEARCH - hybrid search combining vector and fulltext
            "VECTOR_HYBRID_SEARCH" | "HYBRID_SEARCH" | "FUSION_SEARCH" => {
                if evaluated_args.len() < 5 || evaluated_args.len() > 7 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_HYBRID_SEARCH requires 5-7 arguments: collection, vector_index, text_field, query_vector, text_query, [limit], [options]"
                            .to_string(),
                    ));
                }

                let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_HYBRID_SEARCH: first argument must be a string (collection name)"
                            .to_string(),
                    )
                })?;

                let vector_index = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_HYBRID_SEARCH: second argument must be a string (vector index name)"
                            .to_string(),
                    )
                })?;

                let text_field = evaluated_args[2].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_HYBRID_SEARCH: third argument must be a string (text field)"
                            .to_string(),
                    )
                })?;

                let query_vector = Self::extract_vector_arg(
                    &evaluated_args[3],
                    "VECTOR_HYBRID_SEARCH: query_vector",
                )?;

                let text_query = evaluated_args[4].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_HYBRID_SEARCH: fourth argument must be a string (text query)"
                            .to_string(),
                    )
                })?;

                let limit = evaluated_args.get(5).and_then(|v| v.as_u64()).unwrap_or(10) as usize;

                let options = evaluated_args.get(6).and_then(|v| v.as_object()).cloned();

                let collection = self.get_collection(collection_name)?;

                // Parse options
                let mut vector_weight = 0.5f32;
                let mut text_weight = 0.5f32;
                let _fusion_method = "rrf"; // Reserved for future fusion method selection

                if let Some(opts) = options {
                    if let Some(vw) = opts.get("vectorWeight").or(opts.get("vector_weight")) {
                        if let Some(v) = vw.as_f64() {
                            vector_weight = v as f32;
                        }
                    }
                    if let Some(tw) = opts.get("textWeight").or(opts.get("text_weight")) {
                        if let Some(v) = tw.as_f64() {
                            text_weight = v as f32;
                        }
                    }
                    // Reserved for future: fusion method selection
                }

                // Step 1: Vector search
                let vector_results =
                    collection.vector_search(vector_index, &query_vector, limit * 3, None)?;

                // Step 2: Fulltext search
                let fulltext_results = collection
                    .fulltext_search(text_query, Some(vec![text_field.to_string()]), limit * 3)?
                    .into_iter()
                    .filter(|r| r.doc_key != "placeholder") // Filter out placeholder results
                    .collect::<Vec<_>>();

                // Step 3: Normalize scores
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

                // Step 4: Fuse results
                let mut combined_scores: HashMap<String, f32> = HashMap::new();
                let mut doc_sources: HashMap<String, Vec<String>> = HashMap::new();
                let mut doc_vector_scores: HashMap<String, f32> = HashMap::new();
                let mut doc_text_scores: HashMap<String, f32> = HashMap::new();

                // Process vector results
                for (rank, result) in vector_results.iter().enumerate() {
                    let rrf_score = 1.0 / (60.0 + rank as f32 + 1.0);
                    *combined_scores.entry(result.doc_key.clone()).or_insert(0.0) +=
                        rrf_score * vector_weight;
                    doc_sources
                        .entry(result.doc_key.clone())
                        .or_default()
                        .push("vector".to_string());
                    doc_vector_scores.insert(
                        result.doc_key.clone(),
                        vector_scores.get(&result.doc_key).copied().unwrap_or(0.0),
                    );
                }

                // Process text results
                for (rank, result) in fulltext_results.iter().enumerate() {
                    let rrf_score = 1.0 / (60.0 + rank as f32 + 1.0);
                    *combined_scores.entry(result.doc_key.clone()).or_insert(0.0) +=
                        rrf_score * text_weight;
                    doc_sources
                        .entry(result.doc_key.clone())
                        .or_default()
                        .push("text".to_string());
                    doc_text_scores.insert(
                        result.doc_key.clone(),
                        text_scores.get(&result.doc_key).copied().unwrap_or(0.0),
                    );
                }

                // Sort by combined score
                let mut ranked: Vec<_> = combined_scores.into_iter().collect();
                ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

                // Build results
                let results: Vec<Value> = ranked
                    .into_iter()
                    .take(limit)
                    .map(|(doc_key, score)| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("_key".to_string(), json!(doc_key));
                        obj.insert("score".to_string(), json!(score));
                        obj.insert(
                            "vectorScore".to_string(),
                            json!(doc_vector_scores.get(&doc_key).copied().unwrap_or(0.0)),
                        );
                        obj.insert(
                            "textScore".to_string(),
                            json!(doc_text_scores.get(&doc_key).copied().unwrap_or(0.0)),
                        );
                        obj.insert(
                            "sources".to_string(),
                            json!(doc_sources.get(&doc_key).cloned().unwrap_or_default()),
                        );

                        if let Ok(doc) = collection.get(&doc_key) {
                            obj.insert("document".to_string(), doc.to_value());
                        }

                        Value::Object(obj)
                    })
                    .collect();

                Ok(Value::Array(results))
            }

            // SAMPLE(collection, count) - Return random documents from a collection
            "SAMPLE" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "SAMPLE requires 2 arguments: collection, count".to_string(),
                    ));
                }
                let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("SAMPLE: collection must be a string".to_string())
                })?;
                let count = evaluated_args[1].as_u64().ok_or_else(|| {
                    DbError::ExecutionError("SAMPLE: count must be a number".to_string())
                })? as usize;

                let collection = self.get_collection(collection_name)?;
                let all_docs = collection.all();

                if all_docs.is_empty() || count == 0 {
                    return Ok(Value::Array(vec![]));
                }

                use rand::seq::SliceRandom;
                let mut rng = rand::thread_rng();
                let mut docs: Vec<Value> = all_docs.iter().map(|d| d.to_value()).collect();
                docs.shuffle(&mut rng);
                let sampled: Vec<Value> = docs.into_iter().take(count).collect();

                Ok(Value::Array(sampled))
            }

            // DOCUMENT(id) or DOCUMENT(collection, key) or DOCUMENT(collection, [keys])
            "DOCUMENT" => {
                match evaluated_args.len() {
                    1 => {
                        match &evaluated_args[0] {
                            Value::String(id) => {
                                if let Some((collection_name, key)) = id.split_once('/') {
                                    let collection = if collection_name.contains(':') {
                                        self.storage.get_collection(collection_name)
                                    } else {
                                        self.get_collection(collection_name)
                                    }?;

                                    match collection.get(key) {
                                        Ok(doc) => Ok(doc.to_value()),
                                        Err(_) => Ok(Value::Null),
                                    }
                                } else {
                                    Err(DbError::ExecutionError(
                                        "DOCUMENT: id must be in format 'collection/key'".to_string(),
                                    ))
                                }
                            }
                            Value::Array(ids) => {
                                let mut results = Vec::new();
                                for id_val in ids {
                                    if let Some(id) = id_val.as_str() {
                                        if let Some((collection_name, key)) = id.split_once('/') {
                                            let collection_result = if collection_name.contains(':') {
                                                self.storage.get_collection(collection_name)
                                            } else {
                                                self.get_collection(collection_name)
                                            };

                                            if let Ok(collection) = collection_result {
                                                if let Ok(doc) = collection.get(key) {
                                                    results.push(doc.to_value());
                                                }
                                            }
                                        }
                                    }
                                }
                                Ok(Value::Array(results))
                            }
                            Value::Null => Ok(Value::Null),
                            _ => Err(DbError::ExecutionError(
                                "DOCUMENT: first argument must be a string or array".to_string(),
                            )),
                        }
                    }
                    2 => {
                        let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                            DbError::ExecutionError(
                                "DOCUMENT: collection name must be a string".to_string(),
                            )
                        })?;
                        let collection = if collection_name.contains(':') {
                            self.storage.get_collection(collection_name)?
                        } else {
                            self.get_collection(collection_name)?
                        };

                        match &evaluated_args[1] {
                            Value::String(key) => match collection.get(key) {
                                Ok(doc) => Ok(doc.to_value()),
                                Err(_) => Ok(Value::Null),
                            },
                            Value::Array(keys) => {
                                let mut results = Vec::new();
                                for key_val in keys {
                                    if let Some(key) = key_val.as_str() {
                                        if let Ok(doc) = collection.get(key) {
                                            results.push(doc.to_value());
                                        }
                                    }
                                }
                                Ok(Value::Array(results))
                            }
                            _ => Err(DbError::ExecutionError(
                                "DOCUMENT: second argument must be a string or array of keys"
                                    .to_string(),
                            )),
                        }
                    }
                    _ => Err(DbError::ExecutionError(
                        "DOCUMENT requires 1 or 2 arguments".to_string(),
                    )),
                }
            }

            // VECTOR_INDEX_STATS(collection, index_name) - get vector index statistics
            "VECTOR_INDEX_STATS" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_INDEX_STATS requires 2 arguments: collection, index_name".to_string(),
                    ));
                }

                let coll_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_INDEX_STATS: first argument must be a string (collection name)"
                            .to_string(),
                    )
                })?;

                let index_name = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_INDEX_STATS: second argument must be a string (index name)"
                            .to_string(),
                    )
                })?;

                let collection = self.get_collection(coll_name)?;
                let indexes = collection.list_vector_indexes();

                let stats = indexes
                    .into_iter()
                    .find(|idx| idx.name == index_name)
                    .ok_or_else(|| {
                        DbError::ExecutionError(format!(
                            "VECTOR_INDEX_STATS: index '{}' not found in collection '{}'",
                            index_name, coll_name
                        ))
                    })?;

                let mut result = serde_json::Map::new();
                result.insert("name".to_string(), Value::String(stats.name));
                result.insert("field".to_string(), Value::String(stats.field));
                result.insert("dimension".to_string(), Value::Number(serde_json::Number::from(stats.dimension)));
                result.insert("vectors".to_string(), Value::Number(serde_json::Number::from(stats.indexed_vectors)));
                result.insert("metric".to_string(), Value::String(format!("{:?}", stats.metric).to_lowercase()));
                result.insert("quantization".to_string(), Value::String(format!("{:?}", stats.quantization).to_lowercase()));
                result.insert("memory_bytes".to_string(), Value::Number(serde_json::Number::from(stats.memory_bytes)));
                result.insert("compression_ratio".to_string(), Value::Number(number_from_f64(stats.compression_ratio as f64)));
                result.insert("m".to_string(), Value::Number(serde_json::Number::from(stats.m)));
                result.insert("ef_construction".to_string(), Value::Number(serde_json::Number::from(stats.ef_construction)));

                Ok(Value::Object(result))
            }

            // VECTOR_NORMALIZE(vector) - normalize a vector
            "VECTOR_NORMALIZE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_NORMALIZE requires 1 argument: vector".to_string(),
                    ));
                }

                let vec = Self::extract_vector_arg(&evaluated_args[0], "VECTOR_NORMALIZE: vector")?;

                let magnitude: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();

                if magnitude < 1e-10 {
                    let result: Vec<Value> = vec
                        .iter()
                        .map(|_| Value::Number(number_from_f64(0.0)))
                        .collect();
                    return Ok(Value::Array(result));
                }

                let normalized: Vec<Value> = vec
                    .iter()
                    .map(|x| Value::Number(number_from_f64((x / magnitude) as f64)))
                    .collect();

                Ok(Value::Array(normalized))
            }

            _ => Err(DbError::ExecutionError(format!(
                "Unknown search function: {}",
                name
            ))),
        }
    }
}
