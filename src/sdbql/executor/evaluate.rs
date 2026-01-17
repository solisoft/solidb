//! Function evaluation for SDBQL executor.
//!
//! This module contains the main evaluate_function method that handles
//! context-aware built-in functions. Simple value-based functions are
//! delegated to the builtins/ submodules.

use serde_json::{json, Value};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

use super::builtins;
use super::types::Context;
use super::utils::number_from_f64;
use super::{compare_values, parse_datetime, safe_regex, to_bool, values_equal, QueryExecutor};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::Expression;
use crate::storage::{distance_meters, GeoPoint};

use super::functions::phonetic::{
    caverphone, cologne_phonetic, double_metaphone, metaphone, nysiis, soundex, soundex_el,
    soundex_es, soundex_fr, soundex_it, soundex_ja, soundex_nl, soundex_pt,
};

impl<'a> QueryExecutor<'a> {
    /// Evaluate a function call
    pub(super) fn evaluate_function(&self, name: &str, args: &[Expression], ctx: &Context) -> DbResult<Value> {
        // Evaluate all arguments
        let evaluated_args: Vec<Value> = args
            .iter()
            .map(|arg| self.evaluate_expr_with_context(arg, ctx))
            .collect::<DbResult<Vec<_>>>()?;

        match name.to_uppercase().as_str() {
            // IF(condition, true_val, false_val) - conditional evaluation
            "IF" | "IIF" => {
                if evaluated_args.len() != 3 {
                    return Err(DbError::ExecutionError(
                        "IF requires 3 arguments: condition, true_value, false_value".to_string(),
                    ));
                }
                if to_bool(&evaluated_args[0]) {
                    Ok(evaluated_args[1].clone())
                } else {
                    Ok(evaluated_args[2].clone())
                }
            }

            // Type checking functions
            "IS_ARRAY" | "IS_LIST" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_ARRAY requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::Array(_))))
            }

            "IS_BOOL" | "IS_BOOLEAN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_BOOLEAN requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::Bool(_))))
            }

            "IS_NUMBER" | "IS_NUMERIC" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_NUMBER requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::Number(_))))
            }

            "IS_INTEGER" | "IS_INT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_INTEGER requires 1 argument".to_string(),
                    ));
                }
                let is_int = match &evaluated_args[0] {
                    Value::Number(n) => {
                        // Check if it's an integer (no decimal part)
                        if n.as_i64().is_some() {
                            true
                        } else if let Some(f) = n.as_f64() {
                            f.fract() == 0.0 && f.is_finite()
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                Ok(Value::Bool(is_int))
            }

            "IS_STRING" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_STRING requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::String(_))))
            }

            "IS_OBJECT" | "IS_DOCUMENT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_OBJECT requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::Object(_))))
            }

            "IS_NULL" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_NULL requires 1 argument".to_string(),
                    ));
                }
                Ok(Value::Bool(matches!(evaluated_args[0], Value::Null)))
            }

            "IS_DATETIME" | "IS_DATESTRING" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_DATETIME requires 1 argument".to_string(),
                    ));
                }
                let is_datetime = match &evaluated_args[0] {
                    Value::String(s) => {
                        // Try to parse as ISO 8601 datetime
                        chrono::DateTime::parse_from_rfc3339(s).is_ok()
                    }
                    Value::Number(n) => {
                        // Could be a Unix timestamp - check if it's a reasonable timestamp value
                        if let Some(ts) = n.as_i64() {
                            // Valid timestamp range: 1970-01-01 to 3000-01-01 approximately
                            ts >= 0 && ts < 32503680000000 // Year 3000 in milliseconds
                        } else {
                            false
                        }
                    }
                    _ => false,
                };
                Ok(Value::Bool(is_datetime))
            }

            "TYPENAME" | "TYPE_OF" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "TYPENAME requires 1 argument".to_string(),
                    ));
                }
                let type_name = match &evaluated_args[0] {
                    Value::Null => "null",
                    Value::Bool(_) => "bool",
                    Value::Number(n) => {
                        if n.is_i64() || n.is_u64() {
                            "int"
                        } else {
                            "number"
                        }
                    }
                    Value::String(_) => "string",
                    Value::Array(_) => "array",
                    Value::Object(_) => "object",
                };
                Ok(Value::String(type_name.to_string()))
            }

            // TIME_BUCKET(timestamp, interval) - bucket timestamp into fixed intervals
            "TIME_BUCKET" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "TIME_BUCKET requires 2 arguments: timestamp, interval (e.g. '5m')"
                            .to_string(),
                    ));
                }

                // Parse interval
                let interval_str = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("TIME_BUCKET: interval must be a string".to_string())
                })?;

                let len = interval_str.len();
                if len < 2 {
                    return Err(DbError::ExecutionError(
                        "TIME_BUCKET: invalid interval format".to_string(),
                    ));
                }

                let unit = &interval_str[len - 1..];
                let val_str = &interval_str[..len - 1];
                let val: u64 = val_str.parse().map_err(|_| {
                    DbError::ExecutionError("TIME_BUCKET: invalid interval number".to_string())
                })?;

                let interval_ms = match unit {
                    "s" => val * 1000,
                    "m" => val * 1000 * 60,
                    "h" => val * 1000 * 60 * 60,
                    "d" => val * 1000 * 60 * 60 * 24,
                    _ => {
                        return Err(DbError::ExecutionError(
                            "TIME_BUCKET: valid units are s, m, h, d".to_string(),
                        ))
                    }
                };

                if interval_ms == 0 {
                    return Err(DbError::ExecutionError(
                        "TIME_BUCKET: interval cannot be 0".to_string(),
                    ));
                }

                // Parse timestamp
                match &evaluated_args[0] {
                    Value::Number(n) => {
                        let ts = n.as_i64().ok_or_else(|| {
                            DbError::ExecutionError(
                                "TIME_BUCKET: timestamp must be a valid number".to_string(),
                            )
                        })?;
                        // Bucket (use div_euclid to handle negative timestamps correctly)
                        let bucket = ts.div_euclid(interval_ms as i64) * (interval_ms as i64);
                        Ok(Value::Number(bucket.into()))
                    }
                    Value::String(s) => {
                        let dt = chrono::DateTime::parse_from_rfc3339(s).map_err(|_| {
                            DbError::ExecutionError(
                                "TIME_BUCKET: invalid timestamp string".to_string(),
                            )
                        })?;
                        let ts = dt.timestamp_millis();
                        let bucket_ts = ts.div_euclid(interval_ms as i64) * (interval_ms as i64);

                        // Convert back to string (UTC)
                        // We use basic arithmetic to get seconds/nanos for safe reconstruction
                        let seconds = bucket_ts.div_euclid(1000);
                        let nanos = (bucket_ts.rem_euclid(1000) * 1_000_000) as u32;

                        // Try standard DateTime construction (compatible with most chrono versions)
                        // We rely on Utc being available
                        if let Some(dt) = chrono::DateTime::from_timestamp(seconds, nanos) {
                            Ok(Value::String(dt.to_rfc3339()))
                        } else {
                            // Fallback or error path
                            Err(DbError::ExecutionError(
                                "TIME_BUCKET: failed to construct date".to_string(),
                            ))
                        }
                    }
                    _ => Err(DbError::ExecutionError(
                        "TIME_BUCKET: timestamp must be number or string".to_string(),
                    )),
                }
            }

            // DISTANCE(lat1, lon1, lat2, lon2) - distance between two points in meters
            "DISTANCE" => {
                if evaluated_args.len() != 4 {
                    return Err(DbError::ExecutionError(
                        "DISTANCE requires 4 arguments: lat1, lon1, lat2, lon2".to_string(),
                    ));
                }
                let lat1 = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("DISTANCE: lat1 must be a number".to_string())
                })?;
                let lon1 = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("DISTANCE: lon1 must be a number".to_string())
                })?;
                let lat2 = evaluated_args[2].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("DISTANCE: lat2 must be a number".to_string())
                })?;
                let lon2 = evaluated_args[3].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("DISTANCE: lon2 must be a number".to_string())
                })?;

                let dist = distance_meters(lat1, lon1, lat2, lon2);
                Ok(Value::Number(number_from_f64(dist)))
            }

            // GEO_DISTANCE(geopoint1, geopoint2) - distance between two geo points
            "GEO_DISTANCE" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "GEO_DISTANCE requires 2 arguments: point1, point2".to_string(),
                    ));
                }
                let p1 = GeoPoint::from_value(&evaluated_args[0]).ok_or_else(|| {
                    DbError::ExecutionError(
                        "GEO_DISTANCE: first argument must be a geo point".to_string(),
                    )
                })?;
                let p2 = GeoPoint::from_value(&evaluated_args[1]).ok_or_else(|| {
                    DbError::ExecutionError(
                        "GEO_DISTANCE: second argument must be a geo point".to_string(),
                    )
                })?;

                let dist = distance_meters(p1.lat, p1.lon, p2.lat, p2.lon);
                Ok(Value::Number(number_from_f64(dist)))
            }

            // VECTOR_SIMILARITY(vector1, vector2, metric?) - compute similarity between two vectors
            // metric: "cosine" (default), "euclidean", "dot"
            "VECTOR_SIMILARITY" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_SIMILARITY requires 2-3 arguments: vector1, vector2, [metric]"
                            .to_string(),
                    ));
                }

                let vec1 =
                    Self::extract_vector_arg(&evaluated_args[0], "VECTOR_SIMILARITY: vector1")?;
                let vec2 =
                    Self::extract_vector_arg(&evaluated_args[1], "VECTOR_SIMILARITY: vector2")?;

                if vec1.len() != vec2.len() {
                    return Err(DbError::ExecutionError(format!(
                        "VECTOR_SIMILARITY: vectors must have same dimension ({} vs {})",
                        vec1.len(),
                        vec2.len()
                    )));
                }

                let metric = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_str().unwrap_or("cosine")
                } else {
                    "cosine"
                };

                let score = match metric.to_lowercase().as_str() {
                    "cosine" => crate::storage::vector::cosine_similarity(&vec1, &vec2),
                    "euclidean" => crate::storage::vector::euclidean_distance(&vec1, &vec2),
                    "dot" | "dotproduct" => crate::storage::vector::dot_product(&vec1, &vec2),
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "VECTOR_SIMILARITY: unknown metric '{}', use 'cosine', 'euclidean', or 'dot'",
                            metric
                        )));
                    }
                };

                Ok(Value::Number(number_from_f64(score as f64)))
            }

            // VECTOR_DISTANCE(vector1, vector2, metric?) - compute distance between two vectors
            // metric: "euclidean" (default), "cosine" (returns 1-similarity)
            "VECTOR_DISTANCE" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_DISTANCE requires 2-3 arguments: vector1, vector2, [metric]"
                            .to_string(),
                    ));
                }

                let vec1 =
                    Self::extract_vector_arg(&evaluated_args[0], "VECTOR_DISTANCE: vector1")?;
                let vec2 =
                    Self::extract_vector_arg(&evaluated_args[1], "VECTOR_DISTANCE: vector2")?;

                if vec1.len() != vec2.len() {
                    return Err(DbError::ExecutionError(format!(
                        "VECTOR_DISTANCE: vectors must have same dimension ({} vs {})",
                        vec1.len(),
                        vec2.len()
                    )));
                }

                let metric = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_str().unwrap_or("euclidean")
                } else {
                    "euclidean"
                };

                let distance = match metric.to_lowercase().as_str() {
                    "euclidean" => crate::storage::vector::euclidean_distance(&vec1, &vec2),
                    "cosine" => 1.0 - crate::storage::vector::cosine_similarity(&vec1, &vec2),
                    _ => {
                        return Err(DbError::ExecutionError(format!(
                            "VECTOR_DISTANCE: unknown metric '{}', use 'euclidean' or 'cosine'",
                            metric
                        )));
                    }
                };

                Ok(Value::Number(number_from_f64(distance as f64)))
            }

            // VECTOR_NORMALIZE(vector) - normalize a vector to unit length
            "VECTOR_NORMALIZE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_NORMALIZE requires 1 argument: vector".to_string(),
                    ));
                }

                let vec = Self::extract_vector_arg(&evaluated_args[0], "VECTOR_NORMALIZE: vector")?;

                // Calculate magnitude
                let magnitude: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();

                if magnitude < 1e-10 {
                    // Return zero vector if magnitude is too small
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

            // HAS(doc, attribute) - check if document has attribute
            "HAS" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "HAS requires 2 arguments: document, attribute".to_string(),
                    ));
                }

                let doc = evaluated_args[0].as_object().ok_or_else(|| {
                    DbError::ExecutionError(
                        "HAS: first argument must be a document/object".to_string(),
                    )
                })?;

                let attr_name = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("HAS: second argument must be a string".to_string())
                })?;

                Ok(Value::Bool(doc.contains_key(attr_name)))
            }

            // KEEP(doc, attr1, attr2, ...) OR KEEP(doc, [attr1, attr2, ...])
            "KEEP" => {
                if evaluated_args.len() < 2 {
                    return Err(DbError::ExecutionError(
                        "KEEP requires at least 2 arguments: document, attributes...".to_string(),
                    ));
                }

                let doc = evaluated_args[0].as_object().ok_or_else(|| {
                    DbError::ExecutionError(
                        "KEEP: first argument must be a document/object".to_string(),
                    )
                })?;

                let mut keys_to_keep = Vec::new();

                // Handle second argument as array or varargs
                if evaluated_args.len() == 2 && evaluated_args[1].is_array() {
                    let arr = evaluated_args[1].as_array().unwrap();
                    for val in arr {
                        if let Some(s) = val.as_str() {
                            keys_to_keep.push(s);
                        }
                    }
                } else {
                    for arg in &evaluated_args[1..] {
                        if let Some(s) = arg.as_str() {
                            keys_to_keep.push(s);
                        } else {
                            return Err(DbError::ExecutionError(
                                "KEEP: attribute names must be strings".to_string(),
                            ));
                        }
                    }
                }

                let mut new_doc = serde_json::Map::new();
                for key in keys_to_keep {
                    if let Some(val) = doc.get(key) {
                        new_doc.insert(key.to_string(), val.clone());
                    }
                }

                Ok(Value::Object(new_doc))
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

            // SUM(array) - sum of numeric array elements
            "SUM" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "SUM requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("SUM: argument must be an array".to_string())
                })?;

                let sum: f64 = arr.iter().filter_map(|v| v.as_f64()).sum();

                Ok(Value::Number(
                    serde_json::Number::from_f64(sum).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // AVG(array) - average of numeric array elements
            "AVG" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "AVG requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("AVG: argument must be an array".to_string())
                })?;

                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                let avg = nums.iter().sum::<f64>() / nums.len() as f64;
                Ok(Value::Number(
                    serde_json::Number::from_f64(avg).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // MIN(array) - minimum value in array
            "MIN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "MIN requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("MIN: argument must be an array".to_string())
                })?;

                let min = arr
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                match min {
                    Some(n) => Ok(Value::Number(number_from_f64(n))),
                    None => Ok(Value::Null),
                }
            }

            // MAX(array) - maximum value in array
            "MAX" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "MAX requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("MAX: argument must be an array".to_string())
                })?;

                let max = arr
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                match max {
                    Some(n) => Ok(Value::Number(number_from_f64(n))),
                    None => Ok(Value::Null),
                }
            }

            // COUNT(array) - count elements in array
            "COUNT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "COUNT requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("COUNT: argument must be an array".to_string())
                })?;
                Ok(Value::Number(serde_json::Number::from(arr.len())))
            }

            // COUNT_DISTINCT(array) - count distinct values in array
            "COUNT_DISTINCT" | "COUNT_UNIQUE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "COUNT_DISTINCT requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("COUNT_DISTINCT: argument must be an array".to_string())
                })?;
                let unique: std::collections::HashSet<String> =
                    arr.iter().map(|v| v.to_string()).collect();
                Ok(Value::Number(serde_json::Number::from(unique.len())))
            }

            // VARIANCE_POPULATION(array) - population variance
            "VARIANCE_POPULATION" | "VARIANCE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "VARIANCE_POPULATION requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VARIANCE_POPULATION: argument must be an array".to_string(),
                    )
                })?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance =
                    nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                Ok(Value::Number(
                    serde_json::Number::from_f64(variance).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // VARIANCE_SAMPLE(array) - sample variance (n-1 denominator)
            "VARIANCE_SAMPLE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "VARIANCE_SAMPLE requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VARIANCE_SAMPLE: argument must be an array".to_string(),
                    )
                })?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.len() < 2 {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance =
                    nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64;
                Ok(Value::Number(
                    serde_json::Number::from_f64(variance).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // STDDEV_POPULATION(array) - population standard deviation
            "STDDEV_POPULATION" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "STDDEV_POPULATION requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "STDDEV_POPULATION: argument must be an array".to_string(),
                    )
                })?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance =
                    nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
                let stddev = variance.sqrt();
                Ok(Value::Number(
                    serde_json::Number::from_f64(stddev).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // STDDEV_SAMPLE(array) / STDDEV(array) - sample standard deviation (n-1 denominator)
            "STDDEV_SAMPLE" | "STDDEV" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "STDDEV_SAMPLE requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("STDDEV_SAMPLE: argument must be an array".to_string())
                })?;
                let nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.len() < 2 {
                    return Ok(Value::Null);
                }
                let mean = nums.iter().sum::<f64>() / nums.len() as f64;
                let variance =
                    nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (nums.len() - 1) as f64;
                let stddev = variance.sqrt();
                Ok(Value::Number(
                    serde_json::Number::from_f64(stddev).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // MEDIAN(array) - median value
            "MEDIAN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "MEDIAN requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("MEDIAN: argument must be an array".to_string())
                })?;
                let mut nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let len = nums.len();
                let median = if len % 2 == 0 {
                    (nums[len / 2 - 1] + nums[len / 2]) / 2.0
                } else {
                    nums[len / 2]
                };
                Ok(Value::Number(
                    serde_json::Number::from_f64(median).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // PERCENTILE(array, p) - percentile value (p between 0 and 100)
            "PERCENTILE" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "PERCENTILE requires 2 arguments: array, percentile (0-100)".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "PERCENTILE: first argument must be an array".to_string(),
                    )
                })?;
                let p = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError(
                        "PERCENTILE: second argument must be a number".to_string(),
                    )
                })?;
                if !(0.0..=100.0).contains(&p) {
                    return Err(DbError::ExecutionError(
                        "PERCENTILE: percentile must be between 0 and 100".to_string(),
                    ));
                }
                let mut nums: Vec<f64> = arr.iter().filter_map(|v| v.as_f64()).collect();
                if nums.is_empty() {
                    return Ok(Value::Null);
                }
                nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let index = (p / 100.0) * (nums.len() - 1) as f64;
                let lower = index.floor() as usize;
                let upper = index.ceil() as usize;
                let result = if lower == upper {
                    nums[lower]
                } else {
                    let fraction = index - lower as f64;
                    nums[lower] * (1.0 - fraction) + nums[upper] * fraction
                };
                Ok(Value::Number(
                    serde_json::Number::from_f64(result).unwrap_or(serde_json::Number::from(0)),
                ))
            }

            // UNIQUE and SORTED are implemented in evaluate_function_with_values

            // SORTED_UNIQUE(array) - sort and return unique values
            "SORTED_UNIQUE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "SORTED_UNIQUE requires 1 argument".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("SORTED_UNIQUE: argument must be an array".to_string())
                })?;
                let mut seen = std::collections::HashSet::new();
                let mut unique: Vec<Value> = arr
                    .iter()
                    .filter(|v| seen.insert(v.to_string()))
                    .cloned()
                    .collect();
                unique.sort_by(|a, b| match (a, b) {
                    (Value::Number(n1), Value::Number(n2)) => n1
                        .as_f64()
                        .unwrap_or(0.0)
                        .partial_cmp(&n2.as_f64().unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal),
                    (Value::String(s1), Value::String(s2)) => s1.cmp(s2),
                    _ => a.to_string().cmp(&b.to_string()),
                });
                Ok(Value::Array(unique))
            }

            // REVERSE, FIRST, LAST are implemented in evaluate_function_with_values

            // NTH(array, index) - nth element (0-based)
            "NTH" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "NTH requires 2 arguments: array, index".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("NTH: first argument must be an array".to_string())
                })?;
                let index = if let Some(i) = evaluated_args[1].as_i64() {
                    i
                } else if let Some(f) = evaluated_args[1].as_f64() {
                    f as i64
                } else {
                    return Err(DbError::ExecutionError(
                        "NTH: second argument must be a number".to_string(),
                    ));
                } as usize;
                Ok(arr.get(index).cloned().unwrap_or(Value::Null))
            }

            // SLICE(array, start, length?) - slice array
            "SLICE" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "SLICE requires 2-3 arguments: array, start, [length]".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("SLICE: first argument must be an array".to_string())
                })?;
                let start = evaluated_args[1].as_i64().ok_or_else(|| {
                    DbError::ExecutionError("SLICE: start must be an integer".to_string())
                })?;
                let start = if start < 0 {
                    (arr.len() as i64 + start).max(0) as usize
                } else {
                    start as usize
                };
                let length = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_u64().unwrap_or(arr.len() as u64) as usize
                } else {
                    arr.len().saturating_sub(start)
                };
                let end = (start + length).min(arr.len());
                Ok(Value::Array(arr[start..end].to_vec()))
            }

            // FLATTEN is implemented in evaluate_function_with_values

            // PUSH(array, element, unique?) - add element to array
            "PUSH" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "PUSH requires 2-3 arguments: array, element, [unique]".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("PUSH: first argument must be an array".to_string())
                })?;
                let element = &evaluated_args[1];
                let unique = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_bool().unwrap_or(false)
                } else {
                    false
                };
                let mut result = arr.clone();
                if unique {
                    if !result.iter().any(|v| v.to_string() == element.to_string()) {
                        result.push(element.clone());
                    }
                } else {
                    result.push(element.clone());
                }
                Ok(Value::Array(result))
            }

            // APPEND(array1, array2, unique?) - append arrays
            "APPEND" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "APPEND requires 2-3 arguments: array1, array2, [unique]".to_string(),
                    ));
                }
                let arr1 = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("APPEND: first argument must be an array".to_string())
                })?;
                let arr2 = evaluated_args[1].as_array().ok_or_else(|| {
                    DbError::ExecutionError("APPEND: second argument must be an array".to_string())
                })?;
                let unique = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_bool().unwrap_or(false)
                } else {
                    false
                };
                let mut result = arr1.clone();
                if unique {
                    let existing: std::collections::HashSet<String> =
                        result.iter().map(|v| v.to_string()).collect();
                    for item in arr2 {
                        if !existing.contains(&item.to_string()) {
                            result.push(item.clone());
                        }
                    }
                } else {
                    result.extend(arr2.iter().cloned());
                }
                Ok(Value::Array(result))
            }

            // ZIP(array1, array2) - zip two arrays into array of pairs
            "ZIP" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "ZIP requires 2 arguments: array1, array2".to_string(),
                    ));
                }
                let arr1 = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("ZIP: first argument must be an array".to_string())
                })?;
                let arr2 = evaluated_args[1].as_array().ok_or_else(|| {
                    DbError::ExecutionError("ZIP: second argument must be an array".to_string())
                })?;

                let len = std::cmp::min(arr1.len(), arr2.len());
                let mut result = Vec::with_capacity(len);

                for i in 0..len {
                    result.push(Value::Array(vec![arr1[i].clone(), arr2[i].clone()]));
                }
                Ok(Value::Array(result))
            }

            // REMOVE_VALUE(array, value, limit?) - remove value from array
            "REMOVE_VALUE" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "REMOVE_VALUE requires 2-3 arguments: array, value, [limit]".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "REMOVE_VALUE: first argument must be an array".to_string(),
                    )
                })?;
                let val_to_remove = &evaluated_args[1];

                // Optional limit: number of occurrences to remove (default: -1 = remove all)
                let limit = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_i64().unwrap_or(-1)
                } else {
                    -1
                };

                let mut result = Vec::new();
                let mut removed_count = 0;

                for item in arr {
                    if values_equal(item, val_to_remove) {
                        if limit != -1 && removed_count >= limit {
                            result.push(item.clone());
                        } else {
                            removed_count += 1;
                        }
                    } else {
                        result.push(item.clone());
                    }
                }
                Ok(Value::Array(result))
            }

            // ATTRIBUTES(doc, removeInternal?, sort?) - return top-level attribute keys
            "ATTRIBUTES" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "ATTRIBUTES requires at least 1 argument: document".to_string(),
                    ));
                }

                let doc = evaluated_args[0].as_object().ok_or_else(|| {
                    DbError::ExecutionError(
                        "ATTRIBUTES: first argument must be a document/object".to_string(),
                    )
                })?;

                let remove_internal = if evaluated_args.len() > 1 {
                    evaluated_args[1].as_bool().unwrap_or(false)
                } else {
                    false
                };

                let sort_keys = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_bool().unwrap_or(false)
                } else {
                    false
                };

                let mut keys: Vec<String> = doc
                    .keys()
                    .filter(|k| !remove_internal || !k.starts_with('_'))
                    .cloned()
                    .collect();

                if sort_keys {
                    keys.sort();
                }

                Ok(Value::Array(keys.into_iter().map(Value::String).collect()))
            }

            // VALUES(doc, removeInternal?) - return top-level attribute values
            "VALUES" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "VALUES requires at least 1 argument: document".to_string(),
                    ));
                }

                let doc = evaluated_args[0].as_object().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VALUES: first argument must be a document/object".to_string(),
                    )
                })?;

                let remove_internal = if evaluated_args.len() > 1 {
                    evaluated_args[1].as_bool().unwrap_or(false)
                } else {
                    false
                };

                let values: Vec<Value> = doc
                    .iter()
                    .filter(|(k, _)| !remove_internal || !k.starts_with('_'))
                    .map(|(_, v)| v.clone())
                    .collect();

                Ok(Value::Array(values))
            }

            // UNSET(doc, attr1, attr2, ...) OR UNSET(doc, [attr1, attr2, ...])
            "UNSET" => {
                if evaluated_args.len() < 2 {
                    return Err(DbError::ExecutionError(
                        "UNSET requires at least 2 arguments: document, attributes...".to_string(),
                    ));
                }

                let doc = evaluated_args[0].as_object().ok_or_else(|| {
                    DbError::ExecutionError(
                        "UNSET: first argument must be a document/object".to_string(),
                    )
                })?;

                let mut keys_to_unset = std::collections::HashSet::new();

                // Handle second argument as array or varargs
                if evaluated_args.len() == 2 && evaluated_args[1].is_array() {
                    let arr = evaluated_args[1].as_array().unwrap();
                    for val in arr {
                        if let Some(s) = val.as_str() {
                            keys_to_unset.insert(s);
                        }
                    }
                } else {
                    for arg in &evaluated_args[1..] {
                        if let Some(s) = arg.as_str() {
                            keys_to_unset.insert(s);
                        } else {
                            // ArangoDB UNSET ignores non-string arguments for keys, so we just skip them
                            // but existing KEEP implementation errors. Let's error to be safe/consistent with KEEP for now or be lenient.
                            // Docs say: "All other arguments... are attribute names". If not string?
                            // Usually SDBQL functions are permissive. But KEEP errors.
                            // Let's mirror KEEP behavior but maybe loosen it if needed.
                            // However, strictly following KEEP pattern:
                            return Err(DbError::ExecutionError(
                                "UNSET: attribute names must be strings".to_string(),
                            ));
                        }
                    }
                }

                let mut new_doc = serde_json::Map::new();
                for (key, val) in doc {
                    if !keys_to_unset.contains(key.as_str()) {
                        new_doc.insert(key.clone(), val.clone());
                    }
                }

                Ok(Value::Object(new_doc))
            }

            // REGEX_REPLACE(text, search, replacement, caseInsensitive?)
            "REGEX_REPLACE" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "REGEX_REPLACE requires 3-4 arguments: text, search, replacement, [caseInsensitive]"
                            .to_string(),
                    ));
                }

                let text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "REGEX_REPLACE: first argument must be a string".to_string(),
                    )
                })?;

                let search_pattern = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "REGEX_REPLACE: second argument must be a string (regex)".to_string(),
                    )
                })?;

                let replacement = evaluated_args[2].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "REGEX_REPLACE: third argument must be a string".to_string(),
                    )
                })?;

                let case_insensitive = if evaluated_args.len() > 3 {
                    evaluated_args[3].as_bool().unwrap_or(false)
                } else {
                    false
                };

                let pattern = if case_insensitive {
                    format!("(?i){}", search_pattern)
                } else {
                    search_pattern.to_string()
                };

                // Use safe_regex to prevent DoS from malicious patterns
                let re = safe_regex(&pattern)
                    .map_err(|e| DbError::ExecutionError(format!("REGEX_REPLACE: {}", e)))?;

                let result = re.replace_all(text, replacement).to_string();
                Ok(Value::String(result))
            }

            // CONTAINS(text, search, returnIndex?)
            "CONTAINS" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "CONTAINS requires 2-3 arguments: text, search, [returnIndex]".to_string(),
                    ));
                }

                let text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("CONTAINS: first argument must be a string".to_string())
                })?;

                let search = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "CONTAINS: second argument must be a string".to_string(),
                    )
                })?;

                let return_index = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_bool().unwrap_or(false)
                } else {
                    false
                };

                if return_index {
                    match text.find(search) {
                        Some(index) => Ok(Value::Number(serde_json::Number::from(index))),
                        None => Ok(Value::Number(serde_json::Number::from(-1))),
                    }
                } else {
                    Ok(Value::Bool(text.contains(search)))
                }
            }

            // SUBSTITUTE(value, search, replace, limit?) OR SUBSTITUTE(value, mapping, limit?)
            "SUBSTITUTE" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "SUBSTITUTE requires 2-4 arguments".to_string(),
                    ));
                }

                let text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "SUBSTITUTE: first argument must be a string".to_string(),
                    )
                })?;

                let limit = if evaluated_args[1].is_object() {
                    // Mapping mode: SUBSTITUTE(value, mapping, limit?)
                    if evaluated_args.len() > 3 {
                        return Err(DbError::ExecutionError(
                            "SUBSTITUTE with mapping requires 2-3 arguments".to_string(),
                        ));
                    }
                    if evaluated_args.len() == 3 {
                        evaluated_args[2]
                            .as_i64()
                            .or_else(|| evaluated_args[2].as_f64().map(|f| f as i64))
                    } else {
                        None
                    }
                } else {
                    // Replace mode: SUBSTITUTE(value, search, replace, limit?)
                    if evaluated_args.len() < 3 {
                        return Err(DbError::ExecutionError(
                            "SUBSTITUTE requires search and replace strings".to_string(),
                        ));
                    }
                    if evaluated_args.len() == 4 {
                        evaluated_args[3]
                            .as_i64()
                            .or_else(|| evaluated_args[3].as_f64().map(|f| f as i64))
                    } else {
                        None
                    }
                };

                let count_limit = match limit {
                    Some(n) if n > 0 => Some(n as usize),
                    Some(_) => Some(0), // 0 or negative limit means 0 replacements? Actually ArangoDB might handle 0 as replace nothing? Or all? Docs say "optional limit to restrict the number of replacements". Usually 0 means 0.
                    None => None,       // None means replace all
                };

                // Perform substitution
                if evaluated_args[1].is_object() {
                    let mapping = evaluated_args[1].as_object().unwrap();
                    // For mapping, we need to be careful about overlapping replacements.
                    // Simple approach: multiple passes? No, usually single pass.
                    // But standard approach for simple implementation: iterate over mapping keys.
                    // Note: order is not guaranteed in JSON object. ArangoDB docs say "If mapping is used, the order of the attributes is undefined."
                    // So iterative replacement is acceptable even if order varies.

                    let mut result = text.to_string();
                    let replacements_left = count_limit;

                    for (search, replace_val) in mapping {
                        let replace = replace_val.as_str().unwrap_or(""); // Treat non-string values as empty string or stringify? Docs say "mapping values are converted to strings".
                        let replace_str = if replace_val.is_string() {
                            replace.to_string()
                        } else {
                            replace_val.to_string()
                        };

                        if let Some(limit_val) = replacements_left {
                            if limit_val == 0 {
                                break;
                            }
                            // Rust's replacen doesn't return how many replaced.
                            // We might need to handle this manually if we want global limit across all keys.
                            // But wait, "limit" in mapping mode usually means "limit per search term" or "total replacements"?
                            // Arango docs: "limit argument can be used to restrict the number of replacements". It usually applies *per* operation or total?
                            // "length of the search and replace list must be equal".
                            // Let's assume global limit for now? Or per key?
                            // Actually, if using `replacen`, it's per key.
                            // Let's stick to simple iterative replacement.
                            result = result.replacen(search, &replace_str, limit_val);
                            // To correctly track total replacements we'd need a different approach.
                            // Given ArangoDB's undefined order for keys, maybe it doesn't matter much for complex cases.
                            // Let's assume the limit is applied per key for now as it's the simplest interpretation of iterative application.
                        } else {
                            result = result.replace(search, &replace_str);
                        }
                    }
                    Ok(Value::String(result))
                } else {
                    let search = evaluated_args[1].as_str().ok_or_else(|| {
                        DbError::ExecutionError(
                            "SUBSTITUTE: search argument must be a string".to_string(),
                        )
                    })?;
                    let replace = evaluated_args[2].as_str().ok_or_else(|| {
                        DbError::ExecutionError(
                            "SUBSTITUTE: replace argument must be a string".to_string(),
                        )
                    })?;

                    if let Some(n) = count_limit {
                        Ok(Value::String(text.replacen(search, replace, n)))
                    } else {
                        Ok(Value::String(text.replace(search, replace)))
                    }
                }
            }

            // SPLIT and TRIM are implemented in evaluate_function_with_values

            // LTRIM(value, chars?)
            "LTRIM" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "LTRIM requires 1-2 arguments: value, [chars]".to_string(),
                    ));
                }
                let value = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("LTRIM: first argument must be a string".to_string())
                })?;

                let result = if evaluated_args.len() == 2 {
                    let chars = evaluated_args[1].as_str().ok_or_else(|| {
                        DbError::ExecutionError(
                            "LTRIM: second argument must be a string".to_string(),
                        )
                    })?;
                    value.trim_start_matches(|ch| chars.contains(ch))
                } else {
                    value.trim_start()
                };
                Ok(Value::String(result.to_string()))
            }

            // RTRIM(value, chars?)
            "RTRIM" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "RTRIM requires 1-2 arguments: value, [chars]".to_string(),
                    ));
                }
                let value = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("RTRIM: first argument must be a string".to_string())
                })?;

                let result = if evaluated_args.len() == 2 {
                    let chars = evaluated_args[1].as_str().ok_or_else(|| {
                        DbError::ExecutionError(
                            "RTRIM: second argument must be a string".to_string(),
                        )
                    })?;
                    value.trim_end_matches(|ch| chars.contains(ch))
                } else {
                    value.trim_end()
                };
                Ok(Value::String(result.to_string()))
            }

            // JSON_PARSE(text)
            "JSON_PARSE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "JSON_PARSE requires 1 argument: text".to_string(),
                    ));
                }
                let text = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("JSON_PARSE: argument must be a string".to_string())
                })?;

                match serde_json::from_str::<Value>(text) {
                    Ok(v) => Ok(v),
                    Err(_) => Ok(Value::Null), // ArangoDB spec: invalid JSON returns NULL
                }
            }

            // JSON_STRINGIFY(value)
            "JSON_STRINGIFY" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "JSON_STRINGIFY requires 1 argument: value".to_string(),
                    ));
                }
                match serde_json::to_string(&evaluated_args[0]) {
                    Ok(s) => Ok(Value::String(s)),
                    Err(_) => Ok(Value::Null),
                }
            }

            // UUIDV4()
            "UUIDV4" => {
                if !evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "UUIDV4 requires 0 arguments".to_string(),
                    ));
                }
                Ok(Value::String(Uuid::new_v4().to_string()))
            }

            // UUIDV7()
            "UUIDV7" => {
                if !evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "UUIDV7 requires 0 arguments".to_string(),
                    ));
                }
                Ok(Value::String(Uuid::now_v7().to_string()))
            }

            // MD5(string)
            "MD5" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "MD5 requires 1 argument".to_string(),
                    ));
                }
                let input = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("MD5: argument must be a string".to_string())
                })?;
                let digest = md5::compute(input.as_bytes());
                Ok(Value::String(hex::encode(*digest)))
            }

            // SHA256(string)
            "SHA256" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "SHA256 requires 1 argument".to_string(),
                    ));
                }
                let input = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("SHA256: argument must be a string".to_string())
                })?;
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(input.as_bytes());
                Ok(Value::String(hex::encode(hasher.finalize())))
            }

            // BASE64_ENCODE(string)
            "BASE64_ENCODE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "BASE64_ENCODE requires 1 argument".to_string(),
                    ));
                }
                let input = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("BASE64_ENCODE: argument must be a string".to_string())
                })?;
                use base64::{engine::general_purpose, Engine as _};
                Ok(Value::String(general_purpose::STANDARD.encode(input)))
            }

            // BASE64_DECODE(string)
            "BASE64_DECODE" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "BASE64_DECODE requires 1 argument".to_string(),
                    ));
                }
                let input = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("BASE64_DECODE: argument must be a string".to_string())
                })?;
                use base64::{engine::general_purpose, Engine as _};
                match general_purpose::STANDARD.decode(input) {
                    Ok(bytes) => {
                        let s = String::from_utf8(bytes).map_err(|_| {
                            DbError::ExecutionError(
                                "BASE64_DECODE: result is not valid utf8".to_string(),
                            )
                        })?;
                        Ok(Value::String(s))
                    }
                    Err(_) => Err(DbError::ExecutionError(
                        "BASE64_DECODE: invalid base64".to_string(),
                    )),
                }
            }

            // ARGON2_HASH(password) - Hash password using Argon2id
            "ARGON2_HASH" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "ARGON2_HASH requires 1 argument (password)".to_string(),
                    ));
                }
                let password = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("ARGON2_HASH: argument must be a string".to_string())
                })?;

                use argon2::{
                    password_hash::{rand_core::OsRng, PasswordHasher, SaltString},
                    Argon2,
                };

                let salt = SaltString::generate(&mut OsRng);
                let argon2 = Argon2::default();

                match argon2.hash_password(password.as_bytes(), &salt) {
                    Ok(hash) => Ok(Value::String(hash.to_string())),
                    Err(e) => Err(DbError::ExecutionError(format!(
                        "ARGON2_HASH: failed to hash password: {}",
                        e
                    ))),
                }
            }

            // ARGON2_VERIFY(hash, password) - Verify password against Argon2 hash
            "ARGON2_VERIFY" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "ARGON2_VERIFY requires 2 arguments (hash, password)".to_string(),
                    ));
                }
                let hash = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("ARGON2_VERIFY: hash must be a string".to_string())
                })?;
                let password = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("ARGON2_VERIFY: password must be a string".to_string())
                })?;

                use argon2::{
                    password_hash::{PasswordHash, PasswordVerifier},
                    Argon2,
                };

                match PasswordHash::new(hash) {
                    Ok(parsed_hash) => {
                        let result = Argon2::default()
                            .verify_password(password.as_bytes(), &parsed_hash)
                            .is_ok();
                        Ok(Value::Bool(result))
                    }
                    Err(_) => Err(DbError::ExecutionError(
                        "ARGON2_VERIFY: invalid hash format".to_string(),
                    )),
                }
            }

            // SLEEP(ms)
            "SLEEP" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "SLEEP requires 1 argument".to_string(),
                    ));
                }
                let ms = evaluated_args[0].as_u64().ok_or_else(|| {
                    DbError::ExecutionError("SLEEP: argument must be a positive number".to_string())
                })?;
                std::thread::sleep(std::time::Duration::from_millis(ms));
                Ok(Value::Bool(true))
            }

            // ASSERT(condition, message)
            "ASSERT" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "ASSERT requires 2 arguments".to_string(),
                    ));
                }
                let condition = to_bool(&evaluated_args[0]);
                if !condition {
                    let msg = evaluated_args[1].as_str().unwrap_or("Assertion failed");
                    return Err(DbError::ExecutionError(msg.to_string()));
                }
                Ok(Value::Bool(true))
            }

            // TO_BOOL(value)
            "TO_BOOL" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "TO_BOOL requires 1 argument: value".to_string(),
                    ));
                }
                let val = &evaluated_args[0];
                let bool_val = match val {
                    Value::Null => false,
                    Value::Bool(b) => *b,
                    Value::Number(n) => {
                        // 0 is false, everything else is true
                        if let Some(i) = n.as_i64() {
                            i != 0
                        } else if let Some(f) = n.as_f64() {
                            f != 0.0
                        } else {
                            true // Should be covered
                        }
                    }
                    Value::String(s) => !s.is_empty(),
                    Value::Array(_) => true,
                    Value::Object(_) => true,
                };
                Ok(Value::Bool(bool_val))
            }

            // TO_NUMBER and TO_STRING are implemented in evaluate_function_with_values

            // TO_ARRAY(value)
            "TO_ARRAY" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "TO_ARRAY requires 1 argument: value".to_string(),
                    ));
                }
                let val = &evaluated_args[0];
                match val {
                    Value::Null => Ok(Value::Array(vec![])),
                    Value::Array(arr) => Ok(Value::Array(arr.clone())),
                    Value::Object(obj) => {
                        let values: Vec<Value> = obj.values().cloned().collect();
                        Ok(Value::Array(values))
                    }
                    _ => Ok(Value::Array(vec![val.clone()])),
                }
            }

            // UNION(array1, array2, ...) - union of arrays (unique values)
            "UNION" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "UNION requires at least 1 argument".to_string(),
                    ));
                }
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                for arg in &evaluated_args {
                    let arr = arg.as_array().ok_or_else(|| {
                        DbError::ExecutionError("UNION: all arguments must be arrays".to_string())
                    })?;
                    for item in arr {
                        if seen.insert(item.to_string()) {
                            result.push(item.clone());
                        }
                    }
                }
                Ok(Value::Array(result))
            }

            // UNION_DISTINCT(array1, array2, ...) - same as UNION (for compatibility)
            "UNION_DISTINCT" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "UNION_DISTINCT requires at least 1 argument".to_string(),
                    ));
                }
                let mut seen = std::collections::HashSet::new();
                let mut result = Vec::new();
                for arg in &evaluated_args {
                    let arr = arg.as_array().ok_or_else(|| {
                        DbError::ExecutionError(
                            "UNION_DISTINCT: all arguments must be arrays".to_string(),
                        )
                    })?;
                    for item in arr {
                        if seen.insert(item.to_string()) {
                            result.push(item.clone());
                        }
                    }
                }
                Ok(Value::Array(result))
            }

            // MINUS(array1, array2) - elements in array1 not in array2
            "MINUS" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "MINUS requires 2 arguments: array1, array2".to_string(),
                    ));
                }
                let arr1 = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("MINUS: first argument must be an array".to_string())
                })?;
                let arr2 = evaluated_args[1].as_array().ok_or_else(|| {
                    DbError::ExecutionError("MINUS: second argument must be an array".to_string())
                })?;
                let set2: std::collections::HashSet<String> =
                    arr2.iter().map(|v| v.to_string()).collect();
                let mut seen = std::collections::HashSet::new();
                let result: Vec<Value> = arr1
                    .iter()
                    .filter(|v| {
                        let key = v.to_string();
                        !set2.contains(&key) && seen.insert(key)
                    })
                    .cloned()
                    .collect();
                Ok(Value::Array(result))
            }

            // INTERSECTION(array1, array2, ...) - common elements in all arrays
            "INTERSECTION" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "INTERSECTION requires at least 1 argument".to_string(),
                    ));
                }
                let first = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "INTERSECTION: all arguments must be arrays".to_string(),
                    )
                })?;

                if evaluated_args.len() == 1 {
                    return Ok(Value::Array(first.clone()));
                }

                // Build sets for all other arrays
                let mut sets: Vec<std::collections::HashSet<String>> = Vec::new();
                for arg in &evaluated_args[1..] {
                    let arr = arg.as_array().ok_or_else(|| {
                        DbError::ExecutionError(
                            "INTERSECTION: all arguments must be arrays".to_string(),
                        )
                    })?;
                    sets.push(arr.iter().map(|v| v.to_string()).collect());
                }

                let mut seen = std::collections::HashSet::new();
                let result: Vec<Value> = first
                    .iter()
                    .filter(|v| {
                        let key = v.to_string();
                        sets.iter().all(|s| s.contains(&key)) && seen.insert(key)
                    })
                    .cloned()
                    .collect();
                Ok(Value::Array(result))
            }

            // POSITION(array, search, start?) - find position of element in array (0-based, -1 if not found)
            "POSITION" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "POSITION requires 2-3 arguments: array, search, [start]".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("POSITION: first argument must be an array".to_string())
                })?;
                let search = &evaluated_args[1];
                let start = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_i64().unwrap_or(0) as usize
                } else {
                    0
                };
                let position = arr
                    .iter()
                    .skip(start)
                    .position(|v| v.to_string() == search.to_string())
                    .map(|p| p + start);
                Ok(match position {
                    Some(p) => Value::Number(serde_json::Number::from(p)),
                    None => Value::Number(serde_json::Number::from(-1)),
                })
            }

            // CONTAINS_ARRAY(array, search) - check if array contains element
            "CONTAINS_ARRAY" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "CONTAINS_ARRAY requires 2 arguments: array, search".to_string(),
                    ));
                }
                let arr = evaluated_args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError(
                        "CONTAINS_ARRAY: first argument must be an array".to_string(),
                    )
                })?;
                let search = &evaluated_args[1];
                let contains = arr.iter().any(|v| v.to_string() == search.to_string());
                Ok(Value::Bool(contains))
            }

            // ROUND and ABS are implemented in evaluate_function_with_values

            // SQRT(n) - square root

            // SLUGIFY(text) - Convert text to URL-friendly slug

            // ============================================================
            // STRING FUNCTIONS
            // ============================================================

            // STARTS_WITH(str, prefix) - Check if string starts with prefix
            "STARTS_WITH" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "STARTS_WITH requires 2 arguments (string, prefix)".to_string(),
                    ));
                }
                match (&evaluated_args[0], &evaluated_args[1]) {
                    (Value::String(s), Value::String(prefix)) => {
                        Ok(Value::Bool(s.starts_with(prefix.as_str())))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "STARTS_WITH requires string arguments".to_string(),
                    )),
                }
            }

            // ENDS_WITH(str, suffix) - Check if string ends with suffix
            "ENDS_WITH" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "ENDS_WITH requires 2 arguments (string, suffix)".to_string(),
                    ));
                }
                match (&evaluated_args[0], &evaluated_args[1]) {
                    (Value::String(s), Value::String(suffix)) => {
                        Ok(Value::Bool(s.ends_with(suffix.as_str())))
                    }
                    (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "ENDS_WITH requires string arguments".to_string(),
                    )),
                }
            }

            // PAD_LEFT(str, len, char?) - Pad string from left to length
            "PAD_LEFT" | "LPAD" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "PAD_LEFT requires 2-3 arguments (string, length, char?)".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => {
                        let len = evaluated_args[1].as_u64().ok_or_else(|| {
                            DbError::ExecutionError("PAD_LEFT: length must be a number".to_string())
                        })? as usize;
                        let pad_char = if evaluated_args.len() == 3 {
                            evaluated_args[2]
                                .as_str()
                                .and_then(|s| s.chars().next())
                                .unwrap_or(' ')
                        } else {
                            ' '
                        };
                        if s.len() >= len {
                            Ok(Value::String(s.clone()))
                        } else {
                            let padding: String =
                                std::iter::repeat(pad_char).take(len - s.len()).collect();
                            Ok(Value::String(format!("{}{}", padding, s)))
                        }
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "PAD_LEFT requires a string as first argument".to_string(),
                    )),
                }
            }

            // PAD_RIGHT(str, len, char?) - Pad string from right to length
            "PAD_RIGHT" | "RPAD" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "PAD_RIGHT requires 2-3 arguments (string, length, char?)".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => {
                        let len = evaluated_args[1].as_u64().ok_or_else(|| {
                            DbError::ExecutionError(
                                "PAD_RIGHT: length must be a number".to_string(),
                            )
                        })? as usize;
                        let pad_char = if evaluated_args.len() == 3 {
                            evaluated_args[2]
                                .as_str()
                                .and_then(|s| s.chars().next())
                                .unwrap_or(' ')
                        } else {
                            ' '
                        };
                        if s.len() >= len {
                            Ok(Value::String(s.clone()))
                        } else {
                            let padding: String =
                                std::iter::repeat(pad_char).take(len - s.len()).collect();
                            Ok(Value::String(format!("{}{}", s, padding)))
                        }
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "PAD_RIGHT requires a string as first argument".to_string(),
                    )),
                }
            }

            // REPEAT(str, count) - Repeat string n times
            "REPEAT" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "REPEAT requires 2 arguments (string, count)".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => {
                        let count = evaluated_args[1].as_u64().ok_or_else(|| {
                            DbError::ExecutionError(
                                "REPEAT: count must be a positive integer".to_string(),
                            )
                        })? as usize;
                        if count > 10000 {
                            return Err(DbError::ExecutionError(
                                "REPEAT: count cannot exceed 10000".to_string(),
                            ));
                        }
                        Ok(Value::String(s.repeat(count)))
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "REPEAT requires a string as first argument".to_string(),
                    )),
                }
            }

            // IS_EMPTY(val) - Check if null, "", [], {}
            "IS_EMPTY" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "IS_EMPTY requires 1 argument".to_string(),
                    ));
                }
                let is_empty = match &evaluated_args[0] {
                    Value::Null => true,
                    Value::String(s) => s.is_empty(),
                    Value::Array(arr) => arr.is_empty(),
                    Value::Object(obj) => obj.is_empty(),
                    _ => false,
                };
                Ok(Value::Bool(is_empty))
            }

            // IS_BLANK(str) - Check if string is blank (whitespace only)

            // ============================================================
            // ARRAY FUNCTIONS
            // ============================================================

            // INDEX_OF(arr, value) - Find index of value in array
            "INDEX_OF" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "INDEX_OF requires 2 arguments (array, value)".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::Array(arr) => {
                        let search = &evaluated_args[1];
                        for (i, item) in arr.iter().enumerate() {
                            if item == search {
                                return Ok(Value::Number(serde_json::Number::from(i)));
                            }
                        }
                        Ok(Value::Number(serde_json::Number::from(-1i64)))
                    }
                    Value::String(s) => {
                        // Also support finding substring in string
                        if let Value::String(search) = &evaluated_args[1] {
                            match s.find(search.as_str()) {
                                Some(idx) => Ok(Value::Number(serde_json::Number::from(idx))),
                                None => Ok(Value::Number(serde_json::Number::from(-1i64))),
                            }
                        } else {
                            Ok(Value::Number(serde_json::Number::from(-1i64)))
                        }
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "INDEX_OF requires an array or string as first argument".to_string(),
                    )),
                }
            }

            // CHUNK(arr, size) - Split array into chunks
            "CHUNK" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "CHUNK requires 2 arguments (array, size)".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::Array(arr) => {
                        let size = evaluated_args[1].as_u64().ok_or_else(|| {
                            DbError::ExecutionError(
                                "CHUNK: size must be a positive integer".to_string(),
                            )
                        })? as usize;
                        if size == 0 {
                            return Err(DbError::ExecutionError(
                                "CHUNK: size must be greater than 0".to_string(),
                            ));
                        }
                        let chunks: Vec<Value> = arr
                            .chunks(size)
                            .map(|chunk| Value::Array(chunk.to_vec()))
                            .collect();
                        Ok(Value::Array(chunks))
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "CHUNK requires an array as first argument".to_string(),
                    )),
                }
            }

            // TAKE(arr, n) - Take first n elements
            "TAKE" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "TAKE requires 2 arguments (array, count)".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::Array(arr) => {
                        let n = evaluated_args[1].as_u64().ok_or_else(|| {
                            DbError::ExecutionError(
                                "TAKE: count must be a positive integer".to_string(),
                            )
                        })? as usize;
                        let result: Vec<Value> = arr.iter().take(n).cloned().collect();
                        Ok(Value::Array(result))
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "TAKE requires an array as first argument".to_string(),
                    )),
                }
            }

            // DROP(arr, n) - Drop first n elements
            "DROP" | "SKIP" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "DROP requires 2 arguments (array, count)".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::Array(arr) => {
                        let n = evaluated_args[1].as_u64().ok_or_else(|| {
                            DbError::ExecutionError(
                                "DROP: count must be a positive integer".to_string(),
                            )
                        })? as usize;
                        let result: Vec<Value> = arr.iter().skip(n).cloned().collect();
                        Ok(Value::Array(result))
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "DROP requires an array as first argument".to_string(),
                    )),
                }
            }

            // ============================================================
            // TEXT PROCESSING FUNCTIONS
            // ============================================================

            // MASK(str, start?, end?, char?) - Mask string for PII protection
            "MASK" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "MASK requires 1-4 arguments (string, start?, end?, char?)".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => {
                        let len = s.len() as i64;
                        let start = if evaluated_args.len() > 1 {
                            evaluated_args[1].as_i64().unwrap_or(0)
                        } else {
                            0
                        };
                        let end = if evaluated_args.len() > 2 {
                            evaluated_args[2].as_i64().unwrap_or(len)
                        } else {
                            len
                        };
                        let mask_char = if evaluated_args.len() > 3 {
                            evaluated_args[3]
                                .as_str()
                                .and_then(|s| s.chars().next())
                                .unwrap_or('*')
                        } else {
                            '*'
                        };

                        // Handle negative indices
                        let start_idx = if start < 0 {
                            (len + start).max(0) as usize
                        } else {
                            start.min(len) as usize
                        };
                        let end_idx = if end < 0 {
                            (len + end).max(0) as usize
                        } else {
                            end.min(len) as usize
                        };

                        let chars: Vec<char> = s.chars().collect();
                        let result: String = chars
                            .iter()
                            .enumerate()
                            .map(|(i, c)| {
                                if i >= start_idx && i < end_idx {
                                    mask_char
                                } else {
                                    *c
                                }
                            })
                            .collect();
                        Ok(Value::String(result))
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "MASK requires a string as first argument".to_string(),
                    )),
                }
            }

            // TRUNCATE_TEXT(str, len, suffix?) - Truncate with ellipsis
            "TRUNCATE_TEXT" | "ELLIPSIS" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "TRUNCATE_TEXT requires 2-3 arguments (string, length, suffix?)"
                            .to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => {
                        let max_len = evaluated_args[1].as_u64().ok_or_else(|| {
                            DbError::ExecutionError(
                                "TRUNCATE_TEXT: length must be a positive integer".to_string(),
                            )
                        })? as usize;
                        let suffix = if evaluated_args.len() > 2 {
                            evaluated_args[2].as_str().unwrap_or("...").to_string()
                        } else {
                            "...".to_string()
                        };

                        if s.len() <= max_len {
                            Ok(Value::String(s.clone()))
                        } else {
                            let truncate_at = max_len.saturating_sub(suffix.len());
                            let truncated: String = s.chars().take(truncate_at).collect();
                            Ok(Value::String(format!("{}{}", truncated, suffix)))
                        }
                    }
                    Value::Null => Ok(Value::Null),
                    _ => Err(DbError::ExecutionError(
                        "TRUNCATE_TEXT requires a string as first argument".to_string(),
                    )),
                }
            }

            // WORD_COUNT(str) - Count words in string
            "WORD_COUNT" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "WORD_COUNT requires 1 argument".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::String(s) => {
                        let count = s.split_whitespace().count();
                        Ok(Value::Number(serde_json::Number::from(count)))
                    }
                    Value::Null => Ok(Value::Number(serde_json::Number::from(0))),
                    _ => Err(DbError::ExecutionError(
                        "WORD_COUNT requires a string argument".to_string(),
                    )),
                }
            }

            // ============================================================
            // MATH FUNCTIONS
            // ============================================================

            // CLAMP(val, min, max) - Clamp value to range
            "CLAMP" => {
                if evaluated_args.len() != 3 {
                    return Err(DbError::ExecutionError(
                        "CLAMP requires 3 arguments (value, min, max)".to_string(),
                    ));
                }
                let val = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("CLAMP: value must be a number".to_string())
                })?;
                let min_val = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("CLAMP: min must be a number".to_string())
                })?;
                let max_val = evaluated_args[2].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("CLAMP: max must be a number".to_string())
                })?;
                let clamped = val.max(min_val).min(max_val);
                Ok(Value::Number(number_from_f64(clamped)))
            }

            // SIGN(num) - Sign of number (-1, 0, 1)
            "SIGN" | "SIGNUM" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "SIGN requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("SIGN: argument must be a number".to_string())
                })?;
                let sign = if num > 0.0 {
                    1
                } else if num < 0.0 {
                    -1
                } else {
                    0
                };
                Ok(Value::Number(serde_json::Number::from(sign)))
            }

            // MOD(a, b) - Modulo operation
            "MOD" | "MODULO" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "MOD requires 2 arguments".to_string(),
                    ));
                }
                let a = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("MOD: first argument must be a number".to_string())
                })?;
                let b = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("MOD: second argument must be a number".to_string())
                })?;
                if b == 0.0 {
                    return Err(DbError::ExecutionError("MOD: division by zero".to_string()));
                }
                Ok(Value::Number(number_from_f64(a % b)))
            }

            // RANDOM_INT(min, max) - Random integer in range
            "RANDOM_INT" | "RAND_INT" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "RANDOM_INT requires 2 arguments (min, max)".to_string(),
                    ));
                }
                let min_val = evaluated_args[0].as_i64().ok_or_else(|| {
                    DbError::ExecutionError("RANDOM_INT: min must be an integer".to_string())
                })?;
                let max_val = evaluated_args[1].as_i64().ok_or_else(|| {
                    DbError::ExecutionError("RANDOM_INT: max must be an integer".to_string())
                })?;
                if min_val > max_val {
                    return Err(DbError::ExecutionError(
                        "RANDOM_INT: min must be less than or equal to max".to_string(),
                    ));
                }
                use rand::Rng;
                let result = rand::thread_rng().gen_range(min_val..=max_val);
                Ok(Value::Number(serde_json::Number::from(result)))
            }

            // ============================================================
            // OBJECT FUNCTIONS
            // ============================================================

            // GET(obj, path, default?) - Get nested value by path
            "GET" | "GET_VALUE" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "GET requires 2-3 arguments (object, path, default?)".to_string(),
                    ));
                }
                let default_val = if evaluated_args.len() == 3 {
                    evaluated_args[2].clone()
                } else {
                    Value::Null
                };

                match (&evaluated_args[0], &evaluated_args[1]) {
                    (Value::Object(obj), Value::String(path)) => {
                        let parts: Vec<&str> = path.split('.').collect();
                        let mut current: &Value = &Value::Object(obj.clone());

                        for part in parts {
                            match current {
                                Value::Object(o) => {
                                    current = o.get(part).unwrap_or(&Value::Null);
                                }
                                Value::Array(arr) => {
                                    if let Ok(idx) = part.parse::<usize>() {
                                        current = arr.get(idx).unwrap_or(&Value::Null);
                                    } else {
                                        return Ok(default_val);
                                    }
                                }
                                _ => return Ok(default_val),
                            }
                        }

                        if current == &Value::Null {
                            Ok(default_val)
                        } else {
                            Ok(current.clone())
                        }
                    }
                    (Value::Null, _) => Ok(default_val),
                    _ => Ok(default_val),
                }
            }

            // DEEP_MERGE(obj1, obj2) - Deep merge objects
            "DEEP_MERGE" | "MERGE_RECURSIVE" => {
                if evaluated_args.len() < 2 {
                    return Err(DbError::ExecutionError(
                        "DEEP_MERGE requires at least 2 arguments".to_string(),
                    ));
                }

                fn deep_merge(base: &Value, overlay: &Value) -> Value {
                    match (base, overlay) {
                        (Value::Object(base_obj), Value::Object(overlay_obj)) => {
                            let mut result = base_obj.clone();
                            for (key, overlay_val) in overlay_obj {
                                let merged = if let Some(base_val) = base_obj.get(key) {
                                    deep_merge(base_val, overlay_val)
                                } else {
                                    overlay_val.clone()
                                };
                                result.insert(key.clone(), merged);
                            }
                            Value::Object(result)
                        }
                        _ => overlay.clone(),
                    }
                }

                let mut result = evaluated_args[0].clone();
                for arg in &evaluated_args[1..] {
                    result = deep_merge(&result, arg);
                }
                Ok(result)
            }

            // ENTRIES(obj) - Object to [key, value] pairs
            "ENTRIES" | "OBJECT_ENTRIES" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "ENTRIES requires 1 argument".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::Object(obj) => {
                        let entries: Vec<Value> = obj
                            .iter()
                            .map(|(k, v)| Value::Array(vec![Value::String(k.clone()), v.clone()]))
                            .collect();
                        Ok(Value::Array(entries))
                    }
                    Value::Null => Ok(Value::Array(vec![])),
                    _ => Err(DbError::ExecutionError(
                        "ENTRIES requires an object argument".to_string(),
                    )),
                }
            }

            // FROM_ENTRIES(arr) - Array to object
            "FROM_ENTRIES" | "OBJECT_FROM_ENTRIES" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "FROM_ENTRIES requires 1 argument".to_string(),
                    ));
                }
                match &evaluated_args[0] {
                    Value::Array(arr) => {
                        let mut obj = serde_json::Map::new();
                        for item in arr {
                            if let Value::Array(pair) = item {
                                if pair.len() >= 2 {
                                    if let Value::String(key) = &pair[0] {
                                        obj.insert(key.clone(), pair[1].clone());
                                    }
                                }
                            }
                        }
                        Ok(Value::Object(obj))
                    }
                    Value::Null => Ok(Value::Object(serde_json::Map::new())),
                    _ => Err(DbError::ExecutionError(
                        "FROM_ENTRIES requires an array argument".to_string(),
                    )),
                }
            }

            // LOG(x) - natural logarithm (ln)
            "LOG" | "LN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "LOG requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("LOG: argument must be a number".to_string())
                })?;
                if num <= 0.0 {
                    return Err(DbError::ExecutionError(
                        "LOG: argument must be positive".to_string(),
                    ));
                }
                Ok(Value::Number(number_from_f64(num.ln())))
            }

            // LOG10(x) - base-10 logarithm
            "LOG10" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "LOG10 requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("LOG10: argument must be a number".to_string())
                })?;
                if num <= 0.0 {
                    return Err(DbError::ExecutionError(
                        "LOG10: argument must be positive".to_string(),
                    ));
                }
                Ok(Value::Number(number_from_f64(num.log10())))
            }

            // LOG2(x) - base-2 logarithm
            "LOG2" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "LOG2 requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("LOG2: argument must be a number".to_string())
                })?;
                if num <= 0.0 {
                    return Err(DbError::ExecutionError(
                        "LOG2: argument must be positive".to_string(),
                    ));
                }
                Ok(Value::Number(number_from_f64(num.log2())))
            }

            // EXP(x) - e^x
            "EXP" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "EXP requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("EXP: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(num.exp())))
            }

            // SIN(x) - sine (x in radians)
            "SIN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "SIN requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("SIN: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(num.sin())))
            }

            // COS(x) - cosine (x in radians)
            "COS" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "COS requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("COS: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(num.cos())))
            }

            // TAN(x) - tangent (x in radians)
            "TAN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "TAN requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("TAN: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(num.tan())))
            }

            // ASIN(x) - arc sine
            "ASIN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "ASIN requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ASIN: argument must be a number".to_string())
                })?;
                if num < -1.0 || num > 1.0 {
                    return Err(DbError::ExecutionError(
                        "ASIN: argument must be between -1 and 1".to_string(),
                    ));
                }
                Ok(Value::Number(number_from_f64(num.asin())))
            }

            // ACOS(x) - arc cosine
            "ACOS" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "ACOS requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ACOS: argument must be a number".to_string())
                })?;
                if num < -1.0 || num > 1.0 {
                    return Err(DbError::ExecutionError(
                        "ACOS: argument must be between -1 and 1".to_string(),
                    ));
                }
                Ok(Value::Number(number_from_f64(num.acos())))
            }

            // ATAN(x) - arc tangent
            "ATAN" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "ATAN requires 1 argument".to_string(),
                    ));
                }
                let num = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ATAN: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(num.atan())))
            }

            // ATAN2(y, x) - arc tangent of y/x
            "ATAN2" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "ATAN2 requires 2 arguments".to_string(),
                    ));
                }
                let y = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ATAN2: y must be a number".to_string())
                })?;
                let x = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("ATAN2: x must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(y.atan2(x))))
            }

            // PI() - returns pi constant
            "PI" => {
                if !evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError("PI takes no arguments".to_string()));
                }
                Ok(Value::Number(number_from_f64(std::f64::consts::PI)))
            }

            // DEGREES(radians) / DEG(radians) - convert radians to degrees
            "DEGREES" | "DEG" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "DEGREES requires 1 argument".to_string(),
                    ));
                }
                let radians = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("DEGREES: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(radians.to_degrees())))
            }

            // RADIANS(degrees) / RAD(degrees) - convert degrees to radians
            "RADIANS" | "RAD" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "RADIANS requires 1 argument".to_string(),
                    ));
                }
                let degrees = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("RADIANS: argument must be a number".to_string())
                })?;
                Ok(Value::Number(number_from_f64(degrees.to_radians())))
            }

            // COALESCE(a, b, ...) - return first non-null value
            "COALESCE" | "NOT_NULL" | "FIRST_NOT_NULL" => {
                for arg in &evaluated_args {
                    if !arg.is_null() {
                        return Ok(arg.clone());
                    }
                }
                Ok(Value::Null)
            }

            // LEFT(str, n) - get first n characters
            "LEFT" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "LEFT requires 2 arguments".to_string(),
                    ));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("LEFT: first argument must be a string".to_string())
                })?;
                let n = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("LEFT: second argument must be a number".to_string())
                })? as usize;
                let result: String = s.chars().take(n).collect();
                Ok(Value::String(result))
            }

            // RIGHT(str, n) - get last n characters
            "RIGHT" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "RIGHT requires 2 arguments".to_string(),
                    ));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("RIGHT: first argument must be a string".to_string())
                })?;
                let n = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("RIGHT: second argument must be a number".to_string())
                })? as usize;
                let chars: Vec<char> = s.chars().collect();
                let start = chars.len().saturating_sub(n);
                let result: String = chars[start..].iter().collect();
                Ok(Value::String(result))
            }

            // CHAR_LENGTH(str) - character count (unicode-aware)
            "CHAR_LENGTH" => {
                if evaluated_args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "CHAR_LENGTH requires 1 argument".to_string(),
                    ));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("CHAR_LENGTH: argument must be a string".to_string())
                })?;
                Ok(Value::Number(serde_json::Number::from(s.chars().count())))
            }

            // FIND_FIRST(str, search, start?) - find first occurrence, return index or -1
            "FIND_FIRST" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "FIND_FIRST requires 2-3 arguments".to_string(),
                    ));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "FIND_FIRST: first argument must be a string".to_string(),
                    )
                })?;
                let search = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "FIND_FIRST: second argument must be a string".to_string(),
                    )
                })?;
                let start = if evaluated_args.len() == 3 {
                    evaluated_args[2].as_f64().unwrap_or(0.0) as usize
                } else {
                    0
                };

                if start >= s.len() {
                    return Ok(Value::Number(serde_json::Number::from(-1)));
                }

                match s[start..].find(search) {
                    Some(idx) => Ok(Value::Number(serde_json::Number::from(start + idx))),
                    None => Ok(Value::Number(serde_json::Number::from(-1))),
                }
            }

            // FIND_LAST(str, search, end?) - find last occurrence, return index or -1
            "FIND_LAST" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "FIND_LAST requires 2-3 arguments".to_string(),
                    ));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "FIND_LAST: first argument must be a string".to_string(),
                    )
                })?;
                let search = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "FIND_LAST: second argument must be a string".to_string(),
                    )
                })?;
                let end = if evaluated_args.len() == 3 {
                    evaluated_args[2].as_f64().unwrap_or(s.len() as f64) as usize
                } else {
                    s.len()
                };

                let search_str = &s[..end.min(s.len())];
                match search_str.rfind(search) {
                    Some(idx) => Ok(Value::Number(serde_json::Number::from(idx))),
                    None => Ok(Value::Number(serde_json::Number::from(-1))),
                }
            }

            // REGEX_TEST(str, pattern) - test if string matches regex pattern
            "REGEX_TEST" | "REGEX_MATCH" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "REGEX_TEST requires 2 arguments".to_string(),
                    ));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "REGEX_TEST: first argument must be a string".to_string(),
                    )
                })?;
                let pattern = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "REGEX_TEST: second argument must be a string (pattern)".to_string(),
                    )
                })?;

                use regex::Regex;
                let re = Regex::new(pattern).map_err(|e| {
                    DbError::ExecutionError(format!(
                        "REGEX_TEST: invalid regex '{}': {}",
                        pattern, e
                    ))
                })?;
                Ok(Value::Bool(re.is_match(s)))
            }

            // DATE_YEAR(date) - extract year from date

            // RANGE(start, end, step?) - generate array of numbers
            "RANGE" => {
                if evaluated_args.len() < 2 || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "RANGE requires 2-3 arguments".to_string(),
                    ));
                }
                let start = evaluated_args[0].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("RANGE: start must be a number".to_string())
                })? as i64;
                let end = evaluated_args[1].as_f64().ok_or_else(|| {
                    DbError::ExecutionError("RANGE: end must be a number".to_string())
                })? as i64;
                let step = if evaluated_args.len() == 3 {
                    evaluated_args[2].as_f64().ok_or_else(|| {
                        DbError::ExecutionError("RANGE: step must be a number".to_string())
                    })? as i64
                } else {
                    1
                };

                if step == 0 {
                    return Err(DbError::ExecutionError(
                        "RANGE: step cannot be 0".to_string(),
                    ));
                }

                let mut result = Vec::new();
                if step > 0 {
                    let mut i = start;
                    while i <= end {
                        result.push(Value::Number(serde_json::Number::from(i)));
                        i += step;
                    }
                } else {
                    let mut i = start;
                    while i >= end {
                        result.push(Value::Number(serde_json::Number::from(i)));
                        i += step;
                    }
                }
                Ok(Value::Array(result))
            }

            // UPPER and LOWER are implemented in evaluate_function_with_values

            // CONCAT(str1, str2, ...) - concatenate strings
            "CONCAT" => {
                let mut result = String::new();
                for arg in &evaluated_args {
                    match arg {
                        Value::String(s) => result.push_str(s),
                        Value::Number(n) => result.push_str(&n.to_string()),
                        Value::Bool(b) => result.push_str(&b.to_string()),
                        Value::Null => result.push_str("null"),
                        _ => {
                            return Err(DbError::ExecutionError(
                                "CONCAT: arguments must be strings or primitives".to_string(),
                            ))
                        }
                    }
                }
                Ok(Value::String(result))
            }

            // CONCAT_SEPARATOR(separator, array) - join array elements with separator
            "CONCAT_SEPARATOR" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "CONCAT_SEPARATOR requires 2 arguments: separator and array".to_string(),
                    ));
                }
                let separator = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "CONCAT_SEPARATOR: first argument (separator) must be a string".to_string(),
                    )
                })?;

                let array = match &evaluated_args[1] {
                    Value::Array(arr) => arr,
                    _ => {
                        return Err(DbError::ExecutionError(
                            "CONCAT_SEPARATOR: second argument must be an array".to_string(),
                        ))
                    }
                };

                let strings: Vec<String> = array
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        Value::Null => "null".to_string(),
                        _ => format!("{}", v),
                    })
                    .collect();

                Ok(Value::String(strings.join(separator)))
            }

            // SUBSTRING(string, start, length?) - substring
            "SUBSTRING" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "SUBSTRING requires 2-3 arguments".to_string(),
                    ));
                }
                let s = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "SUBSTRING: first argument must be a string".to_string(),
                    )
                })?;
                let start = evaluated_args[1].as_i64().ok_or_else(|| {
                    DbError::ExecutionError("SUBSTRING: start must be a number".to_string())
                })? as usize;
                let len = if evaluated_args.len() > 2 {
                    evaluated_args[2].as_i64().unwrap_or(s.len() as i64) as usize
                } else {
                    s.len() - start
                };

                let result: String = s.chars().skip(start).take(len).collect();
                Ok(Value::String(result))
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
                let max_distance = if evaluated_args.len() == 4 {
                    evaluated_args[3].as_u64().unwrap_or(2) as usize
                } else {
                    2 // Default Levenshtein distance
                };

                let collection = self.get_collection(collection_name)?;

                match collection.fulltext_search(field, query, max_distance) {
                    Some(matches) => {
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
                    None => Err(DbError::ExecutionError(format!(
                        "No fulltext index found on field '{}' in collection '{}'",
                        field, collection_name
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
                    .fulltext_search(fulltext_field, text_query, 2)
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

            // Fallback: delegate to evaluate_function_with_values for simple value-based functions
            _ => self.evaluate_function_with_values(&name.to_uppercase(), &evaluated_args),
        }
    }
    pub(super) fn evaluate_function_with_values(&self, name: &str, args: &[Value]) -> DbResult<Value> {
        // Try phonetic functions first
        if let Some(val) = super::functions::evaluate(name, args)? {
            return Ok(val);
        }

        // Try modular builtins
        if let Some(val) = super::builtins::evaluate(name, args)? {
            return Ok(val);
        }

        // Helper to convert value to f64
        fn val_to_f64(v: &Value) -> f64 {
            match v {
                Value::Number(n) => n.as_f64().unwrap_or(0.0),
                Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
                Value::Bool(true) => 1.0,
                Value::Bool(false) => 0.0,
                _ => 0.0,
            }
        }

        // Helper to flatten array
        fn flatten_arr(arr: &[Value], depth: usize) -> Vec<Value> {
            if depth == 0 {
                return arr.to_vec();
            }
            let mut result = Vec::new();
            for item in arr {
                if let Value::Array(inner) = item {
                    result.extend(flatten_arr(inner, depth - 1));
                } else {
                    result.push(item.clone());
                }
            }
            result
        }

        match name {
            // Array functions
            "FIRST" => {
                if args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "FIRST requires 1 argument".to_string(),
                    ));
                }
                let arr = args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("FIRST: argument must be an array".to_string())
                })?;
                Ok(arr.first().cloned().unwrap_or(Value::Null))
            }
            "LAST" => {
                if args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "LAST requires 1 argument".to_string(),
                    ));
                }
                let arr = args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("LAST: argument must be an array".to_string())
                })?;
                Ok(arr.last().cloned().unwrap_or(Value::Null))
            }
            "LENGTH" => {
                if args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "LENGTH requires 1 argument".to_string(),
                    ));
                }
                let len = match &args[0] {
                    Value::Array(arr) => arr.len(),
                    Value::String(s) => s.len(),
                    Value::Object(obj) => obj.len(),
                    Value::Null => 0,
                    _ => {
                        return Err(DbError::ExecutionError(
                            "LENGTH: argument must be array, string, or object".to_string(),
                        ))
                    }
                };
                Ok(Value::Number(serde_json::Number::from(len)))
            }
            "REVERSE" => {
                if args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "REVERSE requires 1 argument".to_string(),
                    ));
                }
                let arr = args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("REVERSE: argument must be an array".to_string())
                })?;
                let mut reversed = arr.clone();
                reversed.reverse();
                Ok(Value::Array(reversed))
            }
            "SORTED" => {
                if args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "SORTED requires 1 argument".to_string(),
                    ));
                }
                let arr = args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("SORTED: argument must be an array".to_string())
                })?;
                let mut sorted = arr.clone();
                sorted.sort_by(|a, b| compare_values(a, b));
                Ok(Value::Array(sorted))
            }
            "UNIQUE" => {
                if args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "UNIQUE requires 1 argument".to_string(),
                    ));
                }
                let arr = args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("UNIQUE: argument must be an array".to_string())
                })?;
                let mut seen = std::collections::HashSet::new();
                let unique: Vec<Value> = arr
                    .iter()
                    .filter(|v| seen.insert(v.to_string()))
                    .cloned()
                    .collect();
                Ok(Value::Array(unique))
            }
            "FLATTEN" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "FLATTEN requires 1-2 arguments: array, [depth]".to_string(),
                    ));
                }
                let arr = args[0].as_array().ok_or_else(|| {
                    DbError::ExecutionError("FLATTEN: first argument must be an array".to_string())
                })?;
                let depth = if args.len() > 1 {
                    args[1].as_u64().unwrap_or(1) as usize
                } else {
                    1
                };
                Ok(Value::Array(flatten_arr(arr, depth)))
            }
            // String functions
            // Numeric functions

            // Geo functions
            "GEO_WITHIN" => {
                let point = args.get(0);
                let polygon = args.get(1);

                if let (Some(point_val), Some(Value::Array(poly_coords))) = (point, polygon) {
                    let (px, py) = match point_val {
                        Value::Array(arr) if arr.len() >= 2 => {
                            (val_to_f64(&arr[0]), val_to_f64(&arr[1]))
                        }
                        Value::Object(obj) => (
                            obj.get("lon")
                                .or_else(|| obj.get("x"))
                                .map(val_to_f64)
                                .unwrap_or(0.0),
                            obj.get("lat")
                                .or_else(|| obj.get("y"))
                                .map(val_to_f64)
                                .unwrap_or(0.0),
                        ),
                        _ => return Ok(Value::Bool(false)),
                    };

                    let mut inside = false;
                    let n = poly_coords.len();
                    if n > 0 {
                        let mut j = n - 1;
                        for i in 0..n {
                            let (xi, yi) = match &poly_coords[i] {
                                Value::Array(arr) if arr.len() >= 2 => {
                                    (val_to_f64(&arr[0]), val_to_f64(&arr[1]))
                                }
                                Value::Object(obj) => (
                                    obj.get("lon")
                                        .or_else(|| obj.get("x"))
                                        .map(val_to_f64)
                                        .unwrap_or(0.0),
                                    obj.get("lat")
                                        .or_else(|| obj.get("y"))
                                        .map(val_to_f64)
                                        .unwrap_or(0.0),
                                ),
                                _ => (0.0, 0.0),
                            };
                            let (xj, yj) = match &poly_coords[j] {
                                Value::Array(arr) if arr.len() >= 2 => {
                                    (val_to_f64(&arr[0]), val_to_f64(&arr[1]))
                                }
                                Value::Object(obj) => (
                                    obj.get("lon")
                                        .or_else(|| obj.get("x"))
                                        .map(val_to_f64)
                                        .unwrap_or(0.0),
                                    obj.get("lat")
                                        .or_else(|| obj.get("y"))
                                        .map(val_to_f64)
                                        .unwrap_or(0.0),
                                ),
                                _ => (0.0, 0.0),
                            };

                            let intersect = ((yi > py) != (yj > py))
                                && (px < (xj - xi) * (py - yi) / (yj - yi) + xi);
                            if intersect {
                                inside = !inside;
                            }
                            j = i;
                        }
                    }
                    Ok(Value::Bool(inside))
                } else {
                    Ok(Value::Null)
                }
            }
            // Type conversion
            "TO_NUMBER" => {
                if args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "TO_NUMBER requires 1 argument: value".to_string(),
                    ));
                }

                let mut current = &args[0];
                // Unwrap arrays with single element
                while let Value::Array(arr) = current {
                    if arr.len() == 1 {
                        current = &arr[0];
                    } else {
                        return Ok(Value::Number(number_from_f64(0.0)));
                    }
                }

                let num_val = match current {
                    Value::Null => 0.0,
                    Value::Bool(true) => 1.0,
                    Value::Bool(false) => 0.0,
                    Value::Number(n) => n.as_f64().unwrap_or(0.0),
                    Value::String(s) => s.parse::<f64>().unwrap_or(0.0),
                    Value::Array(_) => 0.0,
                    Value::Object(_) => 0.0,
                };

                Ok(Value::Number(number_from_f64(num_val)))
            }
            // Date functions
            "HUMAN_TIME" => {
                if args.len() != 1 {
                    return Err(DbError::ExecutionError(
                        "HUMAN_TIME requires 1 argument: datetime".to_string(),
                    ));
                }
                let dt = parse_datetime(&args[0]).map_err(|_| {
                    DbError::ExecutionError(
                        "HUMAN_TIME: argument must be a valid datetime".to_string(),
                    )
                })?;
                let now = Utc::now();
                let diff = now.signed_duration_since(dt);
                let seconds = diff.num_seconds();

                let s = if seconds < 0 {
                    "in the future".to_string()
                } else if seconds < 60 {
                    "just now".to_string()
                } else if seconds < 3600 {
                    format!("{} minutes ago", seconds / 60)
                } else if seconds < 86400 {
                    format!("{} hours ago", seconds / 3600)
                } else if seconds < 2592000 {
                    format!("{} days ago", seconds / 86400)
                } else {
                    dt.to_rfc3339()
                };
                Ok(Value::String(s))
            }
            _ => Err(DbError::ExecutionError(format!(
                "Unknown function: {}",
                name
            ))),
        }
    }

}
