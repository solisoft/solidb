//! Function evaluation for SDBQL executor.
//!
//! This module contains the main evaluate_function method that handles
//! context-aware built-in functions. Simple value-based functions are
//! delegated to the builtins/ submodules.

use serde_json::{json, Value};
use std::collections::HashMap;

use super::types::Context;
use super::utils::number_from_f64;
use super::QueryExecutor;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::Expression;

use super::phonetic::phonetic::{
    caverphone, cologne_phonetic, double_metaphone, metaphone, nysiis, soundex, soundex_el,
    soundex_es, soundex_fr, soundex_it, soundex_ja, soundex_nl, soundex_pt,
};

impl<'a> QueryExecutor<'a> {
    /// Evaluate a function call
    pub(super) fn evaluate_function(
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

        // Try phonetic functions first (SOUNDEX, METAPHONE, etc.)
        if let Some(result) = super::phonetic::evaluate(name, &evaluated_args)? {
            return Ok(result);
        }

        // Try builtins for simple value-based functions
        if let Some(result) = super::builtins::evaluate(name, &evaluated_args)? {
            return Ok(result);
        }

        // Functions that need executor context (self)
        match name.to_uppercase().as_str() {
            // VECTOR_INDEX_STATS(collection, index_name) - get vector index statistics
            "VECTOR_INDEX_STATS" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_INDEX_STATS requires 2 arguments: collection, index_name"
                            .to_string(),
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

                // Find the specific index
                let stats = indexes
                    .into_iter()
                    .find(|idx| idx.name == index_name)
                    .ok_or_else(|| {
                        DbError::ExecutionError(format!(
                            "VECTOR_INDEX_STATS: index '{}' not found in collection '{}'",
                            index_name, coll_name
                        ))
                    })?;

                // Build result object
                let mut result = serde_json::Map::new();
                result.insert("name".to_string(), Value::String(stats.name));
                result.insert("field".to_string(), Value::String(stats.field));
                result.insert(
                    "dimension".to_string(),
                    Value::Number(serde_json::Number::from(stats.dimension)),
                );
                result.insert(
                    "vectors".to_string(),
                    Value::Number(serde_json::Number::from(stats.indexed_vectors)),
                );
                result.insert(
                    "metric".to_string(),
                    Value::String(format!("{:?}", stats.metric).to_lowercase()),
                );
                result.insert(
                    "quantization".to_string(),
                    Value::String(format!("{:?}", stats.quantization).to_lowercase()),
                );
                result.insert(
                    "memory_bytes".to_string(),
                    Value::Number(serde_json::Number::from(stats.memory_bytes)),
                );
                result.insert(
                    "compression_ratio".to_string(),
                    Value::Number(number_from_f64(stats.compression_ratio as f64)),
                );
                result.insert(
                    "m".to_string(),
                    Value::Number(serde_json::Number::from(stats.m)),
                );
                result.insert(
                    "ef_construction".to_string(),
                    Value::Number(serde_json::Number::from(stats.ef_construction)),
                );

                Ok(Value::Object(result))
            }

            // VECTOR_SIMILARITY(v1, v2) - cosine similarity between two vectors
            "VECTOR_SIMILARITY" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_SIMILARITY requires 2 arguments: vector1, vector2".to_string(),
                    ));
                }
                let v1 = Self::extract_vector_arg(&evaluated_args[0], "VECTOR_SIMILARITY")?;
                let v2 = Self::extract_vector_arg(&evaluated_args[1], "VECTOR_SIMILARITY")?;

                let dot: f32 = v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum::<f32>();
                let mag1: f32 = v1.iter().map(|x| x * x).sum::<f32>().sqrt();
                let mag2: f32 = v2.iter().map(|x| x * x).sum::<f32>().sqrt();

                if mag1 == 0.0 || mag2 == 0.0 {
                    Ok(Value::Number(serde_json::Number::from(0)))
                } else {
                    let similarity = dot / (mag1 * mag2);
                    Ok(Value::Number(number_from_f64(similarity as f64)))
                }
            }

            // VECTOR_NORMALIZE(v) - normalize a vector to unit length
            "VECTOR_NORMALIZE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_NORMALIZE requires 1 argument: vector".to_string(),
                    ));
                }
                let v = Self::extract_vector_arg(&evaluated_args[0], "VECTOR_NORMALIZE")?;

                let mag: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if mag == 0.0 {
                    Ok(Value::Array(vec![]))
                } else {
                    let normalized: Vec<Value> = v
                        .iter()
                        .map(|x| Value::Number(number_from_f64((x / mag) as f64)))
                        .collect();
                    Ok(Value::Array(normalized))
                }
            }

            // VECTOR_DISTANCE(v1, v2) or VECTOR_DISTANCE(v1, v2, metric) - distance between two vectors
            "VECTOR_DISTANCE" => {
                if evaluated_args.len() == 2 || evaluated_args.len() == 3 {
                    let v1 = Self::extract_vector_arg(&evaluated_args[0], "VECTOR_DISTANCE")?;
                    let v2 = Self::extract_vector_arg(&evaluated_args[1], "VECTOR_DISTANCE")?;

                    if evaluated_args.len() == 2 {
                        let mut sum = 0.0f32;
                        for (a, b) in v1.iter().zip(v2.iter()) {
                            let diff = a - b;
                            sum += diff * diff;
                        }
                        let distance = sum.sqrt();
                        Ok(Value::Number(number_from_f64(distance as f64)))
                    } else {
                        let metric = evaluated_args[2].as_str().unwrap_or("euclidean");
                        let distance = match metric.to_lowercase().as_str() {
                            "cosine" | "cosineSimilarity" => {
                                let dot: f32 =
                                    v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum::<f32>();
                                let mag1: f32 = v1.iter().map(|x| x * x).sum::<f32>().sqrt();
                                let mag2: f32 = v2.iter().map(|x| x * x).sum::<f32>().sqrt();
                                if mag1 == 0.0 || mag2 == 0.0 {
                                    0.0f32
                                } else {
                                    1.0 - (dot / (mag1 * mag2))
                                }
                            }
                            _ => {
                                let mut sum = 0.0f32;
                                for (a, b) in v1.iter().zip(v2.iter()) {
                                    let diff = a - b;
                                    sum += diff * diff;
                                }
                                sum.sqrt()
                            }
                        };
                        Ok(Value::Number(number_from_f64(distance as f64)))
                    }
                } else {
                    Err(DbError::ExecutionError(
                        "VECTOR_DISTANCE requires 2 or 3 arguments".to_string(),
                    ))
                }
            }

            // LENGTH(array_or_string_or_collection) - get length of array/string or count of collection
            "LENGTH" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "LENGTH requires 1 argument".to_string(),
                    ));
                }
                let len =
                    match &evaluated_args[0] {
                        Value::Array(arr) => arr.len(),
                        Value::String(s) => {
                            // First try to treat it as a collection name
                            match self.get_collection(s) {
                                Ok(collection) => collection.count(),
                                Err(_) => s.len(), // Fallback to string length if not a valid collection
                            }
                        }
                        Value::Object(obj) => obj.len(),
                        _ => return Err(DbError::ExecutionError(
                            "LENGTH: argument must be array, string, object, or collection name"
                                .to_string(),
                        )),
                    };
                Ok(Value::Number(serde_json::Number::from(len)))
            }

            // FULLTEXT(collection, field, query, maxDistance?) - fulltext search with fuzzy matching
            "FULLTEXT" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "FULLTEXT requires 3-4 arguments: collection, field, query, [maxDistance]"
                            .to_string(),
                    ));
                }
                let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("FULLTEXT: collection must be a string".to_string())
                })?;
                let field = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("FULLTEXT: field must be a string".to_string())
                })?;
                let query = evaluated_args[2].as_str().ok_or_else(|| {
                    DbError::ExecutionError("FULLTEXT: query must be a string".to_string())
                })?;
                let _max_distance = if evaluated_args.len() == 4 {
                    evaluated_args[3].as_u64().unwrap_or(2) as usize
                } else {
                    2 // Default Levenshtein distance
                };

                let collection = self.get_collection(collection_name)?;

                // Use a reasonable limit if max_distance is not intended as limit,
                // but since signature takes limit, we pass a default or the value if it makes sense.
                // Assuming max_distance was intended for fuzzy, but fulltext_search doesn't take it?
                // For now, pass 100 as limit to be safe, or just use max_distance as limit if that was the intent.
                // Let's use 100 as default limit.
                let limit = 100;
                match collection.fulltext_search(query, Some(vec![field.to_string()]), limit) {
                    Ok(matches) => {
                        let results: Vec<Value> = matches
                            .iter()
                            .filter_map(|m| {
                                collection.get(&m.doc_key).ok().map(|doc| {
                                    let mut obj = serde_json::Map::new();
                                    obj.insert("doc".to_string(), doc.to_value());
                                    obj.insert("score".to_string(), json!(m.score));
                                    obj.insert("matched".to_string(), json!(m.matched_terms));
                                    Value::Object(obj)
                                })
                            })
                            .collect();
                        Ok(Value::Array(results))
                    }
                    Err(e) => Err(DbError::ExecutionError(format!(
                        "Fulltext search failed: {}",
                        e
                    ))),
                }
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
            // Direct document lookup by _id or collection/key
            "DOCUMENT" => {
                match evaluated_args.len() {
                    // DOCUMENT("collection/key") or DOCUMENT(["col/k1", "col/k2"])
                    1 => {
                        match &evaluated_args[0] {
                            // Single document by _id
                            Value::String(id) => {
                                if let Some((collection_name, key)) = id.split_once('/') {
                                    let collection = if collection_name.contains(':') {
                                        // Absolute path (e.g. "db:col") - bypass context
                                        self.storage.get_collection(collection_name)
                                    } else {
                                        // Relative path - use context
                                        self.get_collection(collection_name)
                                    }?;

                                    match collection.get(key) {
                                        Ok(doc) => Ok(doc.to_value()),
                                        Err(_) => Ok(Value::Null),
                                    }
                                } else {
                                    Err(DbError::ExecutionError(
                                        "DOCUMENT: id must be in format 'collection/key'"
                                            .to_string(),
                                    ))
                                }
                            }
                            // Multiple documents by _id array
                            Value::Array(ids) => {
                                let mut results = Vec::new();
                                for id_val in ids {
                                    if let Some(id) = id_val.as_str() {
                                        if let Some((collection_name, key)) = id.split_once('/') {
                                            let collection_result = if collection_name.contains(':')
                                            {
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
                    // DOCUMENT("collection", "key") or DOCUMENT("collection", ["k1", "k2"])
                    2 => {
                        let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                            DbError::ExecutionError(
                                "DOCUMENT: collection must be a string".to_string(),
                            )
                        })?;
                        let collection = if collection_name.contains(':') {
                            self.storage.get_collection(collection_name)?
                        } else {
                            self.get_collection(collection_name)?
                        };

                        match &evaluated_args[1] {
                            // Single key
                            Value::String(key) => match collection.get(key) {
                                Ok(doc) => Ok(doc.to_value()),
                                Err(_) => Ok(Value::Null),
                            },
                            // Array of keys
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
                            Value::Null => Ok(Value::Null),
                            _ => Err(DbError::ExecutionError(
                                "DOCUMENT: key must be a string or array".to_string(),
                            )),
                        }
                    }
                    _ => Err(DbError::ExecutionError(
                        "DOCUMENT requires 1 or 2 arguments: (id) or (collection, key)".to_string(),
                    )),
                }
            }

            // LEVENSHTEIN(string1, string2) - Levenshtein distance between two strings
            "LEVENSHTEIN" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "LEVENSHTEIN requires 2 arguments: string1, string2".to_string(),
                    ));
                }
                let s1 = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "LEVENSHTEIN: first argument must be a string".to_string(),
                    )
                })?;
                let s2 = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "LEVENSHTEIN: second argument must be a string".to_string(),
                    )
                })?;

                let distance = crate::storage::levenshtein_distance(s1, s2);
                Ok(Value::Number(serde_json::Number::from(distance)))
            }

            // SIMILARITY(string1, string2) - Trigram similarity score (0.0 to 1.0)
            "SIMILARITY" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "SIMILARITY requires 2 arguments: string1, string2".to_string(),
                    ));
                }
                let s1 = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "SIMILARITY: first argument must be a string".to_string(),
                    )
                })?;
                let s2 = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "SIMILARITY: second argument must be a string".to_string(),
                    )
                })?;

                use crate::storage::{generate_ngrams, ngram_similarity, NGRAM_SIZE};
                let ngrams_a = generate_ngrams(s1, NGRAM_SIZE);
                let ngrams_b = generate_ngrams(s2, NGRAM_SIZE);
                let similarity = ngram_similarity(&ngrams_a, &ngrams_b);

                Ok(Value::Number(
                    serde_json::Number::from_f64(similarity)
                        .unwrap_or_else(|| serde_json::Number::from(0)),
                ))
            }

            // FUZZY_MATCH(text, pattern, max_distance?) - Check if text matches pattern within edit distance
            "FUZZY_MATCH" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "FUZZY_MATCH requires 2-3 arguments: text, pattern, [max_distance]"
                            .to_string(),
                    ));
                }
                let text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "FUZZY_MATCH: first argument must be a string".to_string(),
                    )
                })?;
                let pattern = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "FUZZY_MATCH: second argument must be a string".to_string(),
                    )
                })?;
                let max_distance = if evaluated_args.len() == 3 {
                    evaluated_args[2].as_u64().unwrap_or(2) as usize
                } else {
                    2 // Default max distance
                };

                let distance = crate::storage::levenshtein_distance(text, pattern);
                Ok(Value::Bool(distance <= max_distance))
            }

            // SOUNDEX(string, locale?) - Phonetic encoding with optional locale
            // Supported locales: "en" (default), "de" (German), "fr" (French)
            "SOUNDEX" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "SOUNDEX requires 1 or 2 arguments: SOUNDEX(string) or SOUNDEX(string, locale)".to_string(),
                    ));
                }

                let locale = if evaluated_args.len() == 2 {
                    evaluated_args[1].as_str().unwrap_or("en")
                } else {
                    "en"
                };

                match &evaluated_args[0] {
                    Value::String(s) => {
                        let result = match locale {
                            "de" => cologne_phonetic(s),
                            "fr" => soundex_fr(s),
                            "es" => soundex_es(s),
                            "it" => soundex_it(s),
                            "pt" => soundex_pt(s),
                            "nl" => soundex_nl(s),
                            "el" => soundex_el(s),
                            "ja" => soundex_ja(s),
                            _ => soundex(s), // "en" or any other defaults to American
                        };
                        Ok(Value::String(result))
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "SOUNDEX requires a string argument".to_string(),
                    )),
                }
            }

            // METAPHONE(string) - Metaphone phonetic encoding
            // More accurate than Soundex, handles English pronunciation rules
            "METAPHONE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "METAPHONE requires exactly 1 argument".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => Ok(Value::String(metaphone(s))),
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "METAPHONE requires a string argument".to_string(),
                    )),
                }
            }

            // DOUBLE_METAPHONE(string) - Double Metaphone encoding
            // Returns array with [primary, secondary] codes for ambiguous pronunciations
            "DOUBLE_METAPHONE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "DOUBLE_METAPHONE requires exactly 1 argument".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => {
                        let (primary, secondary) = double_metaphone(s);
                        Ok(Value::Array(vec![
                            Value::String(primary),
                            Value::String(secondary),
                        ]))
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "DOUBLE_METAPHONE requires a string argument".to_string(),
                    )),
                }
            }

            // COLOGNE(string) - Cologne Phonetic algorithm for German names
            // Returns numeric phonetic code optimized for German pronunciation
            "COLOGNE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "COLOGNE requires exactly 1 argument".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => Ok(Value::String(cologne_phonetic(s))),
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "COLOGNE requires a string argument".to_string(),
                    )),
                }
            }

            // CAVERPHONE(string) - Caverphone algorithm for European names
            // Returns 10-character phonetic code, good for matching European surnames
            "CAVERPHONE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "CAVERPHONE requires exactly 1 argument".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => Ok(Value::String(caverphone(s))),
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "CAVERPHONE requires a string argument".to_string(),
                    )),
                }
            }

            // NYSIIS(string) - New York State Identification algorithm
            // More accurate than Soundex for various name origins
            "NYSIIS" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "NYSIIS requires exactly 1 argument".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => Ok(Value::String(nysiis(s))),
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "NYSIIS requires a string argument".to_string(),
                    )),
                }
            }

            // BM25(field, query) - BM25 relevance scoring for a document field
            // Returns a numeric score that can be used in SORT clauses
            // Usage: SORT BM25(doc.content, "search query") DESC
            "BM25" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "BM25 requires 2 arguments: field, query".to_string(),
                    ));
                }

                // Get the field value (should be a string from the document)
                let field_text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("BM25: field must be a string".to_string())
                })?;

                let query = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("BM25: query must be a string".to_string())
                })?;

                // Tokenize query and document
                use crate::storage::{bm25_score, tokenize};
                let query_terms = tokenize(query);
                let doc_terms = tokenize(field_text);
                let doc_length = doc_terms.len();

                // For BM25, we need collection statistics
                // Since we don't have access to the collection here, we'll use simplified scoring
                // In a real implementation, we'd need to pass collection context
                // For now, use a simplified version with estimated parameters
                let avg_doc_length = 100.0; // Estimated average
                let total_docs = 1000; // Estimated total

                // Create a simple term document frequency map
                // In a real implementation, this would come from the collection's fulltext index
                let mut term_doc_freq = std::collections::HashMap::new();
                for term in &query_terms {
                    // Estimate: assume each term appears in ~10% of documents
                    term_doc_freq.insert(term.clone(), total_docs / 10);
                }

                let score = bm25_score(
                    &query_terms,
                    &doc_terms,
                    doc_length,
                    avg_doc_length,
                    total_docs,
                    &term_doc_freq,
                );

                Ok(Value::Number(
                    serde_json::Number::from_f64(score).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // MERGE(obj1, obj2, ...) - merge multiple objects (later objects override earlier ones)
            "MERGE" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "MERGE requires at least 1 argument".to_string(),
                    ));
                }

                let mut result = serde_json::Map::new();

                for arg in &evaluated_args {
                    match arg {
                        Value::Object(obj) => {
                            // Merge this object into the result
                            for (key, value) in obj {
                                result.insert(key.clone(), value.clone());
                            }
                        }
                        Value::Null => {
                            // Skip null values
                            continue;
                        }
                        _ => {
                            return Err(DbError::ExecutionError(format!(
                                "MERGE: all arguments must be objects, got: {:?}",
                                arg
                            )));
                        }
                    }
                }

                Ok(Value::Object(result))
            }

            // DATE_NOW() - current timestamp in milliseconds since Unix epoch

            // COLLECTION_COUNT(collection) - get the count of documents in a collection
            "COLLECTION_COUNT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "COLLECTION_COUNT requires 1 argument: collection name".to_string(),
                    ));
                }
                let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "COLLECTION_COUNT: argument must be a string (collection name)".to_string(),
                    )
                })?;

                let collection = self.get_collection(collection_name)?;
                let count = collection.count();
                Ok(Value::Number(serde_json::Number::from(count)))
            }

            // DATE_ISO8601(date) - convert timestamp to ISO 8601 string

            // HYBRID_SEARCH(collection, vector_index, fulltext_field, query_vector, text_query, options?)
            // Combines vector similarity with fulltext search for better RAG results
            // options: { vector_weight: 0.5, text_weight: 0.5, limit: 10, fusion: "weighted" | "rrf" }
            "HYBRID_SEARCH" => {
                if evaluated_args.len() < 5 || evaluated_args.len() > 6 {
                    return Err(DbError::ExecutionError(
                        "HYBRID_SEARCH requires 5-6 arguments: collection, vector_index, fulltext_field, query_vector, text_query, [options]"
                            .to_string(),
                    ));
                }

                // Extract arguments
                let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "HYBRID_SEARCH: collection must be a string".to_string(),
                    )
                })?;
                let vector_index = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "HYBRID_SEARCH: vector_index must be a string".to_string(),
                    )
                })?;
                let fulltext_field = evaluated_args[2].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "HYBRID_SEARCH: fulltext_field must be a string".to_string(),
                    )
                })?;
                let query_vector =
                    Self::extract_vector_arg(&evaluated_args[3], "HYBRID_SEARCH: query_vector")?;
                let text_query = evaluated_args[4].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "HYBRID_SEARCH: text_query must be a string".to_string(),
                    )
                })?;

                // Parse options (defaults)
                let mut vector_weight: f32 = 0.5;
                let mut text_weight: f32 = 0.5;
                let mut limit: usize = 10;
                let mut fusion_method = "weighted";

                if evaluated_args.len() == 6 {
                    if let Some(opts) = evaluated_args[5].as_object() {
                        if let Some(vw) = opts.get("vector_weight").and_then(|v| v.as_f64()) {
                            vector_weight = vw as f32;
                        }
                        if let Some(tw) = opts.get("text_weight").and_then(|v| v.as_f64()) {
                            text_weight = tw as f32;
                        }
                        if let Some(l) = opts.get("limit").and_then(|v| v.as_u64()) {
                            limit = l as usize;
                        }
                        if let Some(f) = opts.get("fusion").and_then(|v| v.as_str()) {
                            fusion_method = f;
                        }
                    }
                }

                let collection = self.get_collection(collection_name)?;

                // Step 1: Vector search (get more candidates than limit for better fusion)
                let vector_results =
                    collection.vector_search(vector_index, &query_vector, limit * 3, None)?;

                // Step 2: Fulltext search
                let fulltext_results = collection
                    .fulltext_search(text_query, Some(vec![fulltext_field.to_string()]), 2)
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

                // Step 5: Combine scores based on fusion method
                #[allow(clippy::type_complexity)]
                let mut combined_results: Vec<(
                    String,
                    f32,
                    Option<f32>,
                    Option<f32>,
                    Vec<String>,
                )> = Vec::new();

                if fusion_method == "rrf" {
                    // Reciprocal Rank Fusion
                    let k: f32 = 60.0;
                    let mut rrf_scores: HashMap<String, f32> = HashMap::new();
                    let mut doc_sources: HashMap<String, Vec<String>> = HashMap::new();
                    let mut doc_vector_scores: HashMap<String, f32> = HashMap::new();
                    let mut doc_text_scores: HashMap<String, f32> = HashMap::new();

                    // Process vector results
                    for (rank, result) in vector_results.iter().enumerate() {
                        let rrf_score = 1.0 / (k + rank as f32 + 1.0);
                        *rrf_scores.entry(result.doc_key.clone()).or_insert(0.0) += rrf_score;
                        doc_sources
                            .entry(result.doc_key.clone())
                            .or_default()
                            .push("vector".to_string());
                        doc_vector_scores.insert(result.doc_key.clone(), result.score);
                    }

                    // Process fulltext results
                    for (rank, result) in fulltext_results.iter().enumerate() {
                        let rrf_score = 1.0 / (k + rank as f32 + 1.0);
                        *rrf_scores.entry(result.doc_key.clone()).or_insert(0.0) += rrf_score;
                        doc_sources
                            .entry(result.doc_key.clone())
                            .or_default()
                            .push("fulltext".to_string());
                        doc_text_scores.insert(result.doc_key.clone(), result.score as f32);
                    }

                    for (doc_key, score) in rrf_scores {
                        let sources = doc_sources.remove(&doc_key).unwrap_or_default();
                        let vec_score = doc_vector_scores.get(&doc_key).copied();
                        let txt_score = doc_text_scores.get(&doc_key).copied();
                        combined_results.push((doc_key, score, vec_score, txt_score, sources));
                    }
                } else {
                    // Weighted sum fusion (default)
                    let mut all_doc_keys: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    all_doc_keys.extend(vector_scores.keys().cloned());
                    all_doc_keys.extend(text_scores.keys().cloned());

                    for doc_key in all_doc_keys {
                        let vec_score = vector_scores.get(&doc_key).copied();
                        let txt_score = text_scores.get(&doc_key).copied();

                        let mut sources = Vec::new();
                        let mut combined_score = 0.0;

                        if let Some(vs) = vec_score {
                            combined_score += vs * vector_weight;
                            sources.push("vector".to_string());
                        }
                        if let Some(ts) = txt_score {
                            combined_score += ts * text_weight;
                            sources.push("fulltext".to_string());
                        }

                        // Get original (non-normalized) scores for output
                        let orig_vec_score = vector_results
                            .iter()
                            .find(|r| r.doc_key == doc_key)
                            .map(|r| r.score);
                        let orig_txt_score = fulltext_results
                            .iter()
                            .find(|r| r.doc_key == doc_key)
                            .map(|r| r.score as f32);

                        combined_results.push((
                            doc_key,
                            combined_score,
                            orig_vec_score,
                            orig_txt_score,
                            sources,
                        ));
                    }
                }

                // Step 6: Sort by combined score and limit
                combined_results
                    .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                combined_results.truncate(limit);

                // Step 7: Build result objects with documents
                let results: Vec<Value> = combined_results
                    .iter()
                    .filter_map(|(doc_key, score, vec_score, txt_score, sources)| {
                        collection.get(doc_key).ok().map(|doc| {
                            let mut obj = serde_json::Map::new();
                            obj.insert("doc".to_string(), doc.to_value());
                            obj.insert("score".to_string(), json!(score));
                            if let Some(vs) = vec_score {
                                obj.insert("vector_score".to_string(), json!(vs));
                            }
                            if let Some(ts) = txt_score {
                                obj.insert("text_score".to_string(), json!(ts));
                            }
                            obj.insert("sources".to_string(), json!(sources));
                            Value::Object(obj)
                        })
                    })
                    .collect();

                Ok(Value::Array(results))
            }

            // Unknown function
            _ => Err(DbError::ExecutionError(format!(
                "Unknown function: {}",
                name
            ))),
        }
    }
}
