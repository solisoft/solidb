//! Function evaluation for SDBQL executor.
//!
//! This module contains the main evaluate_function method that handles
//! context-aware built-in functions. Simple value-based functions are
//! delegated to the builtins/ submodules.

use serde_json::{json, Value};

use super::builtins::Route;
use super::types::Context;
use super::utils::number_from_f64;
use super::{compare_values, get_field_ref, to_bool, QueryExecutor};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::Expression;

/// Ceiling on a search's result count (`VECTOR_SEARCH` k and ef,
/// `HYBRID_SEARCH` limit). These size allocations downstream (audit A2).
const MAX_SEARCH_K: usize = 10_000;

/// Ceiling on `VECTOR_SEARCH`'s over-fetch multiplier; k × overfetch is the
/// candidate pool.
const MAX_VECTOR_OVERFETCH: usize = 100;

/// `FULLTEXT`'s default result count (it used to be a hard cap).
const FULLTEXT_DEFAULT_LIMIT: usize = 100;

/// Largest n-gram size `NGRAM_SIMILARITY` / `NGRAM_MATCH` accept.
const MAX_NGRAM_SIZE: u64 = 16;

impl<'a> QueryExecutor<'a> {
    /// Evaluate a function call
    pub(super) fn evaluate_function(
        &self,
        name: &str,
        args: &[Expression],
        ctx: &Context,
    ) -> DbResult<Value> {
        let name_upper = super::builtins::upper_name(name);
        let name: &str = &name_upper;
        if args.iter().any(|a| matches!(a, Expression::Lambda { .. })) {
            let mut evaluated_args = Vec::with_capacity(args.len());
            for arg in args {
                if matches!(arg, Expression::Lambda { .. }) {
                    continue;
                }
                evaluated_args.push(self.evaluate_expr_with_context(arg, ctx)?);
            }
            return self.evaluate_hof_with_lambda(name, evaluated_args, args, ctx);
        }

        // Lazy forms: evaluate only the arguments the result depends on, so
        // `IF(c, DOCUMENT(...), null)` does not read when `c` is false.
        match name {
            "IF" if args.len() == 3 => {
                let cond = self.evaluate_expr_with_context(&args[0], ctx)?;
                let branch = if to_bool(&cond) { &args[1] } else { &args[2] };
                return self.evaluate_expr_with_context(branch, ctx);
            }
            "COALESCE" | "NOT_NULL" => {
                for arg in args {
                    let v = self.evaluate_expr_with_context(arg, ctx)?;
                    if !v.is_null() {
                        return Ok(v);
                    }
                }
                return Ok(Value::Null);
            }
            "TRY" if args.len() == 1 || args.len() == 2 => {
                return match self.evaluate_expr_with_context(&args[0], ctx) {
                    Ok(v) => Ok(v),
                    Err(e) if is_recoverable(&e) => {
                        // A deadline that passed while the failing expression
                        // ran is not the expression's error to swallow.
                        self.check_budget(0)?;
                        match args.get(1) {
                            Some(fallback) => self.evaluate_expr_with_context(fallback, ctx),
                            None => Ok(Value::Null),
                        }
                    }
                    Err(e) => Err(e),
                };
            }
            "TRY" => {
                return Err(DbError::ExecutionError(
                    "TRY requires 1-2 arguments: expression, [fallback]".to_string(),
                ))
            }
            "EXISTS" if !args.is_empty() => {
                if let Some(present) = self.attribute_present(&args[0], ctx)? {
                    let extra = args[1..]
                        .iter()
                        .map(|a| self.evaluate_expr_with_context(a, ctx))
                        .collect::<DbResult<Vec<_>>>()?;
                    return match present {
                        None => Ok(Value::Bool(false)),
                        Some(v) => Ok(Value::Bool(exists_type_matches(&v, &extra)?)),
                    };
                }
            }
            _ => {}
        }

        let evaluated_args: Vec<Value> = args
            .iter()
            .map(|arg| self.evaluate_expr_with_context(arg, ctx))
            .collect::<DbResult<Vec<_>>>()?;
        self.call_function(name, evaluated_args, ctx)
    }

    /// Call a function on already-evaluated arguments. `name` must be upper
    /// case. Shared by direct calls, the pipeline operator (so `x |> MERGE()`
    /// and other executor functions work there too) and APPLY / CALL.
    pub(super) fn call_function(
        &self,
        name: &str,
        args: Vec<Value>,
        ctx: &Context,
    ) -> DbResult<Value> {
        match super::builtins::route(name) {
            Some(Route::Context) => return self.call_context_function(name, args, ctx),
            Some(route) => {
                if let Some(v) = super::builtins::call_route(route, name, &args)? {
                    return Ok(v);
                }
            }
            None => {}
        }
        if let Some(v) = super::builtins::evaluate_unrouted(name, &args)? {
            return Ok(v);
        }
        self.call_context_function(name, args, ctx)
    }

    /// For `EXISTS(path)`: when `expr` is an attribute access, whether the
    /// attribute is present (`Some(Some(value))`) or absent (`Some(None)`) —
    /// a stored `null` is present. `None` when `expr` is not an attribute
    /// access, and the caller falls back to "is the value non-null".
    fn attribute_present(
        &self,
        expr: &Expression,
        ctx: &Context,
    ) -> DbResult<Option<Option<Value>>> {
        let (base, key): (&Expression, Value) = match expr {
            Expression::FieldAccess(base, field) | Expression::OptionalFieldAccess(base, field) => {
                (&**base, Value::String(field.clone()))
            }
            Expression::DynamicFieldAccess(base, key) | Expression::ArrayAccess(base, key) => {
                (&**base, self.evaluate_expr_with_context(key, ctx)?)
            }
            _ => return Ok(None),
        };
        let base_val = self.evaluate_expr_with_context(base, ctx)?;
        let found = match (&base_val, &key) {
            (Value::Object(_), Value::String(k)) => {
                if matches!(
                    expr,
                    Expression::FieldAccess(..) | Expression::OptionalFieldAccess(..)
                ) {
                    get_field_ref(&base_val, k).cloned()
                } else {
                    base_val.get(k.as_str()).cloned()
                }
            }
            (Value::Array(a), Value::Number(n)) => n
                .as_i64()
                .and_then(|i| {
                    if i < 0 {
                        i.checked_add(a.len() as i64)
                    } else {
                        Some(i)
                    }
                })
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| a.get(i).cloned()),
            _ => None,
        };
        Ok(Some(found))
    }

    /// Functions that need the executor: collections, the principal, the
    /// row context. Reached through [`Self::call_function`].
    fn call_context_function(
        &self,
        name: &str,
        evaluated_args: Vec<Value>,
        ctx: &Context,
    ) -> DbResult<Value> {
        match name {
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
                check_same_dimension(&v1, &v2, "VECTOR_SIMILARITY")?;
                Ok(Value::Number(number_from_f64(
                    cosine_similarity(&v1, &v2) as f64
                )))
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

            // VECTOR_DISTANCE(v1, v2 [, metric]) - distance between two vectors.
            // metric: "euclidean" (default, alias "l2"), "cosine" (1 - cosine
            // similarity; aliases "cosine_distance", "cosineSimilarity"), or
            // "dot" (the dot product; aliases "dot_product", "inner_product").
            "VECTOR_DISTANCE" => {
                if evaluated_args.len() != 2 && evaluated_args.len() != 3 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_DISTANCE requires 2 or 3 arguments".to_string(),
                    ));
                }
                let v1 = Self::extract_vector_arg(&evaluated_args[0], "VECTOR_DISTANCE")?;
                let v2 = Self::extract_vector_arg(&evaluated_args[1], "VECTOR_DISTANCE")?;
                // Zipping vectors of different lengths silently truncated the
                // longer one.
                check_same_dimension(&v1, &v2, "VECTOR_DISTANCE")?;
                let metric = match evaluated_args.get(2) {
                    None | Some(Value::Null) => "euclidean".to_string(),
                    Some(Value::String(m)) => m.to_ascii_lowercase(),
                    Some(other) => {
                        return Err(DbError::ExecutionError(format!(
                            "VECTOR_DISTANCE: metric must be a string, got {}",
                            other
                        )))
                    }
                };
                let distance = match metric.as_str() {
                    "euclidean" | "l2" => v1
                        .iter()
                        .zip(v2.iter())
                        .map(|(a, b)| (a - b) * (a - b))
                        .sum::<f32>()
                        .sqrt(),
                    "cosine" | "cosine_distance" | "cosinesimilarity" => {
                        let (m1, m2) = (magnitude(&v1), magnitude(&v2));
                        if m1 == 0.0 || m2 == 0.0 {
                            0.0
                        } else {
                            1.0 - dot(&v1, &v2) / (m1 * m2)
                        }
                    }
                    "dot" | "dot_product" | "inner_product" => dot(&v1, &v2),
                    other => {
                        return Err(DbError::ExecutionError(format!(
                        "VECTOR_DISTANCE: unknown metric '{}' (expected euclidean, cosine or dot)",
                        other
                    )))
                    }
                };
                Ok(Value::Number(number_from_f64(distance as f64)))
            }

            // FULLTEXT(collection, field, query [, distance | {distance, limit}])
            // Fulltext search. `distance` is the edit distance at which a
            // document term still counts as a (fuzzy) match of a query term
            // when scoring; `limit` caps the result count (default 100).
            "FULLTEXT" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 4 {
                    return Err(DbError::ExecutionError(
                        "FULLTEXT requires 3-4 arguments: collection, field, query, [distance | options]"
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
                let mut max_distance: usize = 2;
                let mut limit = FULLTEXT_DEFAULT_LIMIT;
                match evaluated_args.get(3) {
                    None | Some(Value::Null) => {}
                    Some(Value::Number(n)) => {
                        max_distance = n.as_u64().ok_or_else(|| {
                            DbError::ExecutionError(
                                "FULLTEXT: distance must be a non-negative integer".to_string(),
                            )
                        })? as usize;
                    }
                    Some(Value::Object(opts)) => {
                        for (k, v) in opts {
                            match k.as_str() {
                                "distance" => {
                                    max_distance = v.as_u64().ok_or_else(|| {
                                        DbError::ExecutionError(
                                            "FULLTEXT: distance must be a non-negative integer"
                                                .to_string(),
                                        )
                                    })? as usize
                                }
                                "limit" => {
                                    limit = (v.as_u64().ok_or_else(|| {
                                        DbError::ExecutionError(
                                            "FULLTEXT: limit must be a non-negative integer"
                                                .to_string(),
                                        )
                                    })? as usize)
                                        .min(MAX_SEARCH_K)
                                }
                                other => {
                                    return Err(DbError::ExecutionError(format!(
                                        "FULLTEXT: unknown option '{}' (expected distance, limit)",
                                        other
                                    )))
                                }
                            }
                        }
                    }
                    Some(_) => {
                        return Err(DbError::ExecutionError(
                            "FULLTEXT: 4th argument must be a distance or an options object"
                                .to_string(),
                        ))
                    }
                }
                if limit == 0 {
                    return Ok(Value::Array(vec![]));
                }

                let collection = self.get_collection(collection_name)?;
                let gate = self.row_policy_gate(collection_name);
                // The index returns candidates that share a term with the
                // query; they are re-scored here with the requested distance.
                // Over-fetch so hits the policy drops do not under-fill.
                let fetch = if gate.is_some() {
                    limit.saturating_mul(4).min(MAX_SEARCH_K)
                } else {
                    limit
                };
                let query_terms = crate::storage::tokenize(query);
                let matches = collection
                    .fulltext_search(query, Some(vec![field.to_string()]), fetch)
                    .map_err(|e| {
                        DbError::ExecutionError(format!("Fulltext search failed: {}", e))
                    })?;
                let mut results: Vec<(f64, Value)> = Vec::with_capacity(matches.len());
                for m in matches {
                    let Ok(doc) = collection.get(&m.doc_key) else {
                        continue;
                    };
                    let doc = doc.to_value();
                    // Audit H2: hits hidden by the row policy are dropped.
                    if !gate
                        .as_ref()
                        .is_none_or(|g| self.row_policy_allows(g, &doc, ctx))
                    {
                        continue;
                    }
                    let score = match get_field_ref(&doc, field).and_then(Value::as_str) {
                        Some(text) => fulltext_score(&query_terms, text, max_distance),
                        None => m.score,
                    };
                    if score <= 0.0 {
                        continue;
                    }
                    let mut obj = serde_json::Map::new();
                    obj.insert("doc".to_string(), doc);
                    obj.insert("score".to_string(), json!(score));
                    obj.insert("matched".to_string(), json!(m.matched_terms));
                    results.push((score, Value::Object(obj)));
                }
                results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
                results.truncate(limit);
                Ok(Value::Array(results.into_iter().map(|(_, v)| v).collect()))
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

                if count == 0 {
                    return Ok(Value::Array(vec![]));
                }
                let collection = self.get_collection(collection_name)?;

                // Audit P3: the scan is bounded by the row ceiling (it used to
                // be `all()`, then a full copy and shuffle), rows hidden by the
                // row policy are excluded (H2), and the sample is drawn by
                // reservoir, holding at most `count` picks beyond the scan.
                let docs = self.scan_bounded(&collection)?;
                let docs = self.apply_row_policy(collection_name, docs, ctx);
                Ok(Value::Array(reservoir_sample(docs, count)))
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
                                        self.qualified_collection(collection_name)
                                    } else {
                                        // Relative path - use context
                                        self.get_collection(collection_name)
                                    }?;

                                    Ok(self
                                        .get_visible(&collection, collection_name, key, ctx)
                                        .unwrap_or(Value::Null))
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
                                                self.qualified_collection(collection_name)
                                            } else {
                                                self.get_collection(collection_name)
                                            };

                                            if let Ok(collection) = collection_result {
                                                if let Some(doc) = self.get_visible(
                                                    &collection,
                                                    collection_name,
                                                    key,
                                                    ctx,
                                                ) {
                                                    results.push(doc);
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
                            self.qualified_collection(collection_name)?
                        } else {
                            self.get_collection(collection_name)?
                        };

                        match &evaluated_args[1] {
                            // Single key
                            Value::String(key) => Ok(self
                                .get_visible(&collection, collection_name, key, ctx)
                                .unwrap_or(Value::Null)),
                            // Array of keys
                            Value::Array(keys) => {
                                let mut results = Vec::new();
                                for key_val in keys {
                                    if let Some(key) = key_val.as_str() {
                                        if let Some(doc) =
                                            self.get_visible(&collection, collection_name, key, ctx)
                                        {
                                            results.push(doc);
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

            // LEVENSHTEIN(string1, string2) - Levenshtein distance between two
            // strings. LEVENSHTEIN_DISTANCE is the AQL name.
            "LEVENSHTEIN" | "LEVENSHTEIN_DISTANCE" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(format!(
                        "{} requires 2 arguments: string1, string2",
                        name
                    )));
                }
                let s1 = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(format!("{}: first argument must be a string", name))
                })?;
                let s2 = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError(format!("{}: second argument must be a string", name))
                })?;
                Ok(Value::Number(serde_json::Number::from(
                    bounded_levenshtein(s1, s2, name)?,
                )))
            }

            // LEVENSHTEIN_MATCH(text, target, distance) - true when the edit
            // distance is at most `distance` (0-4). Plain Levenshtein: AQL's
            // optional transpositions / maxTerms / prefix arguments are not
            // supported.
            "LEVENSHTEIN_MATCH" => {
                if evaluated_args.len() != 3 {
                    return Err(DbError::ExecutionError(
                        "LEVENSHTEIN_MATCH requires 3 arguments: text, target, distance"
                            .to_string(),
                    ));
                }
                let (Some(text), Some(target)) =
                    (evaluated_args[0].as_str(), evaluated_args[1].as_str())
                else {
                    return Ok(Value::Bool(false));
                };
                let max = evaluated_args[2]
                    .as_u64()
                    .filter(|d| *d <= 4)
                    .ok_or_else(|| {
                        DbError::ExecutionError(
                            "LEVENSHTEIN_MATCH: distance must be an integer between 0 and 4"
                                .to_string(),
                        )
                    })? as usize;
                // Cheap reject before the O(n·m) distance.
                let (la, lb) = (text.chars().count(), target.chars().count());
                if la.abs_diff(lb) > max {
                    return Ok(Value::Bool(false));
                }
                let d = bounded_levenshtein(text, target, name)?;
                Ok(Value::Bool(d <= max))
            }

            // NGRAM_MATCH(text, target, threshold?, ngramSize?) - Jaccard
            // similarity of the n-gram sets >= threshold (default 0.7).
            // NGRAM_SIMILARITY is routed to the AQL score in builtins/string.rs;
            // this arm only answers for it if that route is ever bypassed.
            "NGRAM_SIMILARITY" | "NGRAM_MATCH" => {
                let is_match = name == "NGRAM_MATCH";
                let max_args = if is_match { 4 } else { 3 };
                if evaluated_args.len() < 2 || evaluated_args.len() > max_args {
                    return Err(DbError::ExecutionError(if is_match {
                        "NGRAM_MATCH requires 2-4 arguments: text, target, [threshold], [ngramSize]"
                            .to_string()
                    } else {
                        "NGRAM_SIMILARITY requires 2-3 arguments: text, target, [ngramSize]"
                            .to_string()
                    }));
                }
                let (Some(a), Some(b)) = (evaluated_args[0].as_str(), evaluated_args[1].as_str())
                else {
                    return Ok(if is_match {
                        Value::Bool(false)
                    } else {
                        Value::Null
                    });
                };
                let size_arg = evaluated_args.get(if is_match { 3 } else { 2 });
                let n = match size_arg {
                    None | Some(Value::Null) => crate::storage::NGRAM_SIZE as u64,
                    Some(v) => v
                        .as_u64()
                        .filter(|n| (1..=MAX_NGRAM_SIZE).contains(n))
                        .ok_or_else(|| {
                            DbError::ExecutionError(format!(
                                "{}: ngramSize must be an integer between 1 and {}",
                                name, MAX_NGRAM_SIZE
                            ))
                        })?,
                } as usize;
                use crate::storage::{generate_ngrams, ngram_similarity};
                let sim = ngram_similarity(&generate_ngrams(a, n), &generate_ngrams(b, n));
                if !is_match {
                    return Ok(Value::Number(number_from_f64(sim)));
                }
                let threshold = match evaluated_args.get(2) {
                    None | Some(Value::Null) => 0.7,
                    Some(v) => v
                        .as_f64()
                        .filter(|t| (0.0..=1.0).contains(t))
                        .ok_or_else(|| {
                            DbError::ExecutionError(
                                "NGRAM_MATCH: threshold must be a number between 0 and 1"
                                    .to_string(),
                            )
                        })?,
                };
                Ok(Value::Bool(sim >= threshold))
            }

            // IN_RANGE(value, low, high [, includeLow = true, includeHigh = true])
            // Uses the same ordering as the comparison operators.
            "IN_RANGE" => {
                if evaluated_args.len() < 3 || evaluated_args.len() > 5 {
                    return Err(DbError::ExecutionError(
                        "IN_RANGE requires 3-5 arguments: value, low, high, [includeLow], [includeHigh]"
                            .to_string(),
                    ));
                }
                let flag = |i: usize| -> DbResult<bool> {
                    match evaluated_args.get(i) {
                        None | Some(Value::Null) => Ok(true),
                        Some(Value::Bool(b)) => Ok(*b),
                        Some(_) => Err(DbError::ExecutionError(
                            "IN_RANGE: includeLow / includeHigh must be booleans".to_string(),
                        )),
                    }
                };
                let (include_low, include_high) = (flag(3)?, flag(4)?);
                use std::cmp::Ordering;
                let v = &evaluated_args[0];
                let lo = compare_values(v, &evaluated_args[1]);
                let hi = compare_values(v, &evaluated_args[2]);
                let above = lo == Ordering::Greater || (include_low && lo == Ordering::Equal);
                let below = hi == Ordering::Less || (include_high && hi == Ordering::Equal);
                Ok(Value::Bool(above && below))
            }

            // EXISTS(value [, "type", typeName]) on a computed value (the
            // attribute-path form is handled before argument evaluation).
            "EXISTS" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 3 {
                    return Err(DbError::ExecutionError(
                        "EXISTS requires 1-3 arguments: path, [\"type\", typeName]".to_string(),
                    ));
                }
                if evaluated_args[0].is_null() {
                    return Ok(Value::Bool(false));
                }
                Ok(Value::Bool(exists_type_matches(
                    &evaluated_args[0],
                    &evaluated_args[1..],
                )?))
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

            // BM25(field, query) - BM25-style relevance score for one field.
            // Usage: SORT BM25(doc.content, "search query") DESC
            //
            // Approximate: BM25 needs corpus statistics (document count,
            // average length, per-term document frequency) and this function
            // sees one string, so it scores against fixed estimates (1000
            // documents, average length 100, each term in 10% of documents).
            // Scores rank documents consistently within one query but are not
            // comparable to a real BM25 index. The fulltext index does not
            // expose those statistics yet.
            "BM25" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "BM25 requires 2 arguments: field, query".to_string(),
                    ));
                }
                // A missing field scores 0 rather than failing the query.
                let field_text = match &evaluated_args[0] {
                    Value::String(s) => s.as_str(),
                    Value::Null => return Ok(json!(0)),
                    _ => {
                        return Err(DbError::ExecutionError(
                            "BM25: field must be a string".to_string(),
                        ))
                    }
                };
                let query = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("BM25: query must be a string".to_string())
                })?;

                use crate::storage::{bm25_score, tokenize};
                // The query is the same on every row: tokenise it once.
                let query_terms = {
                    let mut cached = self.caches.bm25_query.lock();
                    match cached.as_ref() {
                        Some((q, terms)) if q == query => terms.clone(),
                        _ => {
                            let terms = std::sync::Arc::new(tokenize(query));
                            *cached = Some((query.to_string(), terms.clone()));
                            terms
                        }
                    }
                };
                let doc_terms = tokenize(field_text);
                let doc_length = doc_terms.len();

                const AVG_DOC_LENGTH: f64 = 100.0;
                const TOTAL_DOCS: usize = 1000;
                let term_doc_freq: std::collections::HashMap<String, usize> = query_terms
                    .iter()
                    .map(|t| (t.clone(), TOTAL_DOCS / 10))
                    .collect();

                let score = bm25_score(
                    &query_terms,
                    &doc_terms,
                    doc_length,
                    AVG_DOC_LENGTH,
                    TOTAL_DOCS,
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

                // Audit P7: take the first object by value and extend it,
                // instead of deep-copying every argument into a fresh map.
                let mut result: Option<serde_json::Map<String, Value>> = None;
                for arg in evaluated_args {
                    match arg {
                        Value::Object(obj) => match result.as_mut() {
                            None => result = Some(obj),
                            Some(acc) => acc.extend(obj),
                        },
                        Value::Null => continue,
                        other => {
                            return Err(DbError::ExecutionError(format!(
                                "MERGE: all arguments must be objects, got: {:?}",
                                other
                            )));
                        }
                    }
                }

                Ok(Value::Object(result.unwrap_or_default()))
            }

            // COLLECTION_COUNT(collection) - number of documents in a collection.
            //
            // For a principal bound by a row policy on the collection the
            // result is null: the stored count includes rows the policy
            // hides, and revealing it tells the caller how many rows exist
            // that it cannot see (audit S3). Count visible rows with
            // COUNT(FOR d IN coll RETURN 1) instead. Only local shards are
            // counted.
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
                if self.row_policy_applies(collection_name) {
                    return Ok(Value::Null);
                }
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
                            // Audit A2: sizes allocations downstream.
                            limit = (l as usize).min(MAX_SEARCH_K);
                        }
                        if let Some(f) = opts.get("fusion").and_then(|v| v.as_str()) {
                            fusion_method = f;
                        }
                    }
                }

                let collection = self.get_collection(collection_name)?;
                let gate = self.row_policy_gate(collection_name);

                // Delegate to the shared engine implementation (also used by
                // the HTTP and driver hybrid-search endpoints). Unknown fusion
                // values keep the historical lenient behavior: weighted.
                let opts = crate::storage::HybridSearchOptions {
                    vector_weight,
                    text_weight,
                    limit,
                    fusion: crate::storage::FusionMethod::parse(fusion_method).unwrap_or_default(),
                };

                let results: Vec<Value> = collection
                    .hybrid_search(
                        vector_index,
                        fulltext_field,
                        &query_vector,
                        text_query,
                        &opts,
                    )?
                    .into_iter()
                    .filter_map(|hit| {
                        let doc = hit.document?;
                        if !gate
                            .as_ref()
                            .is_none_or(|g| self.row_policy_allows(g, &doc, ctx))
                        {
                            return None;
                        }
                        let mut obj = serde_json::Map::new();
                        obj.insert("doc".to_string(), doc);
                        obj.insert("score".to_string(), json!(hit.score));
                        if let Some(vs) = hit.vector_score {
                            obj.insert("vector_score".to_string(), json!(vs));
                        }
                        if let Some(ts) = hit.text_score {
                            obj.insert("text_score".to_string(), json!(ts));
                        }
                        obj.insert("sources".to_string(), json!(hit.sources));
                        Some(Value::Object(obj))
                    })
                    .collect();

                Ok(Value::Array(results))
            }

            // VECTOR_SEARCH(collection, index, query_vector, k, options?)
            // k-NN search with an optional equality metadata filter. Options:
            //   { filter: { field: value, ... }, overfetch: N, ef: N }
            // Returns [{ doc, score }, ...] best-first, at most k after filtering.
            "VECTOR_SEARCH" => {
                if evaluated_args.len() < 4 || evaluated_args.len() > 5 {
                    return Err(DbError::ExecutionError(
                        "VECTOR_SEARCH requires 4-5 arguments: collection, index, query_vector, k, [options]"
                            .to_string(),
                    ));
                }
                let collection_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_SEARCH: collection must be a string".to_string(),
                    )
                })?;
                let index_name = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("VECTOR_SEARCH: index must be a string".to_string())
                })?;
                let query_vector =
                    Self::extract_vector_arg(&evaluated_args[2], "VECTOR_SEARCH: query_vector")?;
                let k = evaluated_args[3].as_u64().ok_or_else(|| {
                    DbError::ExecutionError(
                        "VECTOR_SEARCH: k must be a non-negative integer".to_string(),
                    )
                })?;
                // Audit A2: k, overfetch and ef size allocations in the vector
                // index; `VECTOR_SEARCH(..., 1e15)` aborted the process.
                let k = (k.min(MAX_SEARCH_K as u64)) as usize;

                let mut overfetch: usize = 1;
                let mut ef: Option<usize> = None;
                let mut filter = serde_json::Map::new();
                if evaluated_args.len() == 5 {
                    if let Some(opts) = evaluated_args[4].as_object() {
                        if let Some(o) = opts.get("overfetch").and_then(|v| v.as_u64()) {
                            overfetch = o.clamp(1, MAX_VECTOR_OVERFETCH as u64) as usize;
                        }
                        if let Some(e) = opts.get("ef").and_then(|v| v.as_u64()) {
                            ef = Some(e.min(MAX_SEARCH_K as u64) as usize);
                        }
                        if let Some(f) = opts.get("filter").and_then(|v| v.as_object()) {
                            filter = f.clone();
                        }
                    }
                }
                // With a filter but no explicit over-fetch, widen the candidate pool
                // so a selective filter still returns ~k rows.
                if !filter.is_empty() && overfetch <= 1 {
                    overfetch = 4;
                }

                let collection = self.get_collection(collection_name)?;
                let gate = self.row_policy_gate(collection_name);
                let results: Vec<Value> = collection
                    .vector_search_filtered(index_name, &query_vector, k, overfetch, ef, &filter)?
                    .into_iter()
                    .filter(|(doc, _)| {
                        gate.as_ref()
                            .is_none_or(|g| self.row_policy_allows(g, doc, ctx))
                    })
                    .map(|(doc, score)| {
                        let mut obj = serde_json::Map::new();
                        obj.insert("doc".to_string(), doc);
                        obj.insert("score".to_string(), json!(score));
                        Value::Object(obj)
                    })
                    .collect();
                Ok(Value::Array(results))
            }

            // NEIGHBORS(edge_collection, seeds, options?) - expand seeds N hops
            // over an edge collection, scored by hop distance (local Graph RAG).
            "NEIGHBORS" => self.eval_neighbors(&evaluated_args),

            // GRAPH_RAG(seed_collection, vector_index, edge_collection, query_vector, options?)
            // Retrieve seeds by vector/hybrid similarity, then expand the graph.
            "GRAPH_RAG" => self.eval_graph_rag(&evaluated_args),

            // COMMUNITY_SEARCH(query_text, options?) - global GraphRAG retrieval
            // of community summaries produced by a prior community build.
            "COMMUNITY_SEARCH" => self.eval_community_search(&evaluated_args),

            // PAGERANK(edge_collection [, options?])
            // Runs PageRank over the (undirected) graph defined by the edge collection.
            // Returns array of objects: [{ node: "...", score: 0.123 }, ...] sorted by score desc.
            "PAGERANK" => self.eval_pagerank(&evaluated_args),

            // DEGREE_CENTRALITY(edge_collection)
            "DEGREE_CENTRALITY" => self.eval_degree_centrality(&evaluated_args),

            // RERANK(query, docs, options?) - reorder retrieved docs by relevance.
            // options: { mode: "lexical"|"llm", field, limit, provider, model }.
            "RERANK" => self.eval_rerank(&evaluated_args),

            // RAG_PIPELINE(name, query_vector, options?) - run a stored retrieve→
            // expand→rerank pipeline by name (see _rag_pipelines).
            "RAG_PIPELINE" => self.eval_rag_pipeline(&evaluated_args),

            // DOC_AS_OF(collection, key, timestamp) - point-in-time read of a
            // versioned document. timestamp = epoch millis (number) or RFC3339 string.
            "DOC_AS_OF" => {
                if evaluated_args.len() != 3 {
                    return Err(DbError::ExecutionError(
                        "DOC_AS_OF requires 3 arguments: collection, key, timestamp".to_string(),
                    ));
                }
                let coll_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("DOC_AS_OF: collection must be a string".to_string())
                })?;
                let key = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("DOC_AS_OF: key must be a string".to_string())
                })?;
                let as_of = parse_as_of_micros(&evaluated_args[2])?;
                let collection = self.get_collection(coll_name)?;
                Ok(collection
                    .get_as_of(key, as_of)?
                    .filter(|doc| self.row_policy_permits(coll_name, doc, ctx))
                    .unwrap_or(Value::Null))
            }

            // DOC_HISTORY(collection, key) - version history, newest first.
            "DOC_HISTORY" => {
                if evaluated_args.len() != 2 {
                    return Err(DbError::ExecutionError(
                        "DOC_HISTORY requires 2 arguments: collection, key".to_string(),
                    ));
                }
                let coll_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("DOC_HISTORY: collection must be a string".to_string())
                })?;
                let key = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("DOC_HISTORY: key must be a string".to_string())
                })?;
                let collection = self.get_collection(coll_name)?;
                let history = collection.doc_history(key);
                Ok(Value::Array(match self.row_policy_gate(coll_name) {
                    // A version is shown only if the policy admits its value;
                    // tombstones only once some version of the key is visible.
                    Some(gate) => {
                        let visible: Vec<Option<bool>> = history
                            .iter()
                            .map(|v| {
                                v.get("value")
                                    .filter(|d| d.is_object())
                                    .map(|d| self.row_policy_allows(&gate, d, ctx))
                            })
                            .collect();
                        let any_visible = visible.contains(&Some(true));
                        history
                            .into_iter()
                            .zip(visible)
                            .filter(|(_, vis)| vis.unwrap_or(any_visible))
                            .map(|(v, _)| v)
                            .collect()
                    }
                    None => history,
                }))
            }

            "SNAPSHOT_DIFF" => {
                if evaluated_args.len() != 3 {
                    return Err(DbError::ExecutionError(
                        "SNAPSHOT_DIFF requires collection, t1, t2".to_string(),
                    ));
                }
                let coll_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError(
                        "SNAPSHOT_DIFF: collection must be a string".to_string(),
                    )
                })?;
                let t1 = parse_as_of_micros(&evaluated_args[1])?;
                let t2 = parse_as_of_micros(&evaluated_args[2])?;
                let collection = self.get_collection(coll_name)?;
                let a = self.apply_row_policy(coll_name, collection.scan_as_of(t1)?, ctx);
                let b = self.apply_row_policy(coll_name, collection.scan_as_of(t2)?, ctx);
                let key_of = |d: &Value| {
                    d.get("_key")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                };
                use std::collections::HashMap;
                let mut am: HashMap<String, Value> = HashMap::new();
                for d in a {
                    am.insert(key_of(&d), d);
                }
                let mut bm: HashMap<String, Value> = HashMap::new();
                for d in b {
                    bm.insert(key_of(&d), d);
                }
                let mut inserted = Vec::new();
                let mut updated = Vec::new();
                let mut deleted = Vec::new();
                for (k, vb) in &bm {
                    match am.get(k) {
                        None => inserted.push(vb.clone()),
                        Some(va) if !super::values_equal(va, vb) => updated.push(vb.clone()),
                        _ => {}
                    }
                }
                for (k, va) in &am {
                    if !bm.contains_key(k) {
                        deleted.push(va.clone());
                    }
                }
                Ok(json!({
                    "inserted": inserted,
                    "updated": updated,
                    "deleted": deleted
                }))
            }
            "CURRENT_USER" => Ok(self
                .principal
                .as_ref()
                .map(|p| Value::String(p.user.clone()))
                .unwrap_or(Value::Null)),
            // CURRENT_DATABASE() - the database this query runs in.
            "CURRENT_DATABASE" => {
                if !evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "CURRENT_DATABASE takes no arguments".to_string(),
                    ));
                }
                Ok(Value::String(
                    self.database
                        .clone()
                        .unwrap_or_else(|| "_system".to_string()),
                ))
            }
            "CURRENT_ROLES" => Ok(Value::Array(
                self.principal
                    .as_ref()
                    .map(|p| p.roles.iter().map(|r| Value::String(r.clone())).collect())
                    .unwrap_or_default(),
            )),
            "CAN" => self.eval_can(&evaluated_args),
            "CREATE_GRAPH" => self.eval_create_graph(&evaluated_args),
            "DROP_GRAPH" => self.eval_drop_graph(&evaluated_args),
            "GRAPH_INFO" => self.eval_graph_info(&evaluated_args),
            "CREATE_VIEW" => self.eval_create_view(&evaluated_args),
            "DROP_VIEW" => self.eval_drop_view(&evaluated_args),
            "SEARCH_INDEX" => self.eval_search_index(&evaluated_args),
            "ROW_POLICY" => {
                if evaluated_args.is_empty() || evaluated_args.len() > 2 {
                    return Err(DbError::ExecutionError(
                        "ROW_POLICY requires collection [, predicate]".to_string(),
                    ));
                }
                let coll_name = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("ROW_POLICY: collection must be a string".to_string())
                })?;
                let collection = self.get_collection(coll_name)?;
                if evaluated_args.len() == 1 {
                    return Ok(collection
                        .get_row_policy()
                        .map(Value::String)
                        .unwrap_or(Value::Null));
                }
                // Audit C4: the policy binds every non-admin principal, so only
                // an admin may lift or replace it. An executor with no
                // principal is refused too, as for the catalog functions.
                if !self.principal.as_ref().is_some_and(|p| p.can_admin) {
                    return Err(DbError::Forbidden(
                        "ROW_POLICY(collection, predicate) changes a row policy and requires admin"
                            .to_string(),
                    ));
                }
                // Compiled gates held by this executor are stale either way.
                self.invalidate_row_policy_gates();
                if evaluated_args[1].is_null() {
                    collection.set_row_policy(None)?;
                    // Cached results were computed under the old policy.
                    crate::storage::query_cache::invalidate_collection("", coll_name);
                    return Ok(Value::Null);
                }
                let pred = evaluated_args[1].as_str().ok_or_else(|| {
                    DbError::ExecutionError("ROW_POLICY: predicate must be a string".to_string())
                })?;
                // The predicate later runs under each reader's principal on
                // every scan: it must parse, and it must not write.
                let parsed = crate::sdbql::parser::Parser::new(pred)
                    .and_then(|mut p| p.parse_expression())
                    .map_err(|e| {
                        DbError::ExecutionError(format!("ROW_POLICY: invalid predicate: {e}"))
                    })?;
                if crate::sdbql::ast::expression_mutates(&parsed) {
                    return Err(DbError::ExecutionError(
                        "ROW_POLICY: predicate must not modify data".to_string(),
                    ));
                }
                collection.set_row_policy(Some(pred))?;
                crate::storage::query_cache::invalidate_collection("", coll_name);
                Ok(Value::String(pred.to_string()))
            }
            "EMBED" => self.eval_embed(&evaluated_args),
            "EXTRACT" => self.eval_extract(&evaluated_args),
            "CITE" => Ok(self.eval_cite(&evaluated_args)),
            "GROUNDED" => Ok(self.eval_grounded(&evaluated_args)),
            "SEARCH_SCORE" => Ok(ctx.get("__search_score").cloned().unwrap_or(json!(0.0))),
            "APPLY" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "APPLY requires function name [, args[]]".to_string(),
                    ));
                }
                let fname = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("APPLY: name must be a string".to_string())
                })?;
                let inner = evaluated_args
                    .get(1)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                self.apply_dynamic(fname, &inner, ctx)
            }
            "CALL" => {
                if evaluated_args.is_empty() {
                    return Err(DbError::ExecutionError(
                        "CALL requires function name, args...".to_string(),
                    ));
                }
                let fname = evaluated_args[0].as_str().ok_or_else(|| {
                    DbError::ExecutionError("CALL: name must be a string".to_string())
                })?;
                self.apply_dynamic(fname, &evaluated_args[1..], ctx)
            }

            // Unknown function
            _ => Err(DbError::ExecutionError(format!(
                "Unknown function: {}",
                name
            ))),
        }
    }

    fn apply_dynamic(&self, name: &str, args: &[Value], ctx: &Context) -> DbResult<Value> {
        thread_local! {
            static DEPTH: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
        }
        let too_deep = DEPTH.with(|d| {
            if d.get() >= 8 {
                true
            } else {
                d.set(d.get() + 1);
                false
            }
        });
        if too_deep {
            return Err(DbError::ExecutionError(
                "APPLY/CALL recursion limit (8)".to_string(),
            ));
        }
        // Audit A11: the mutation classifier cannot see through a dynamic
        // name, so state-changing builtins are only callable directly. Nested
        // APPLY/CALL are checked again at their own level of this function.
        let state_changing = crate::sdbql::ast::is_mutating_function(name)
            || (name.eq_ignore_ascii_case("ROW_POLICY") && args.len() >= 2);
        let res = if state_changing {
            Err(DbError::ExecutionError(format!(
                "APPLY/CALL cannot invoke {name}, which changes server state; call it directly"
            )))
        } else {
            let upper = super::builtins::upper_name(name);
            self.call_function(&upper, args.to_vec(), ctx)
        };
        DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        res
    }

    fn eval_embed(&self, args: &[Value]) -> DbResult<Value> {
        let text_or_arr = args
            .first()
            .ok_or_else(|| DbError::ExecutionError("EMBED requires text or [text]".to_string()))?;
        let opts = args.get(1);
        let provider = opts.and_then(|o| o.get("provider")).and_then(Value::as_str);
        let model = opts
            .and_then(|o| o.get("model"))
            .and_then(Value::as_str)
            .map(|s| s.to_string());
        let db = self.database.as_deref().unwrap_or("_system");
        let client =
            crate::server::llm_client::LLMClient::from_storage(self.storage, db, provider, model)?;
        if let Some(arr) = text_or_arr.as_array() {
            let texts: Vec<&str> = arr.iter().filter_map(Value::as_str).collect();
            let vecs = client.embed_batch_blocking(&texts)?;
            return Ok(Value::Array(
                vecs.into_iter()
                    .map(|v| Value::Array(v.into_iter().map(|f| json!(f)).collect()))
                    .collect(),
            ));
        }
        let text = text_or_arr.as_str().ok_or_else(|| {
            DbError::ExecutionError("EMBED: text must be a string or array of strings".to_string())
        })?;
        let v = client.embed_blocking(text)?;
        Ok(Value::Array(v.into_iter().map(|f| json!(f)).collect()))
    }

    fn eval_extract(&self, args: &[Value]) -> DbResult<Value> {
        if args.len() != 2 {
            return Err(DbError::ExecutionError(
                "EXTRACT requires text, schema".to_string(),
            ));
        }
        let text = args[0].as_str().unwrap_or("");
        let schema = &args[1];
        let db = self.database.as_deref().unwrap_or("_system");
        if let Ok(client) =
            crate::server::llm_client::LLMClient::from_storage(self.storage, db, None, None)
        {
            let prompt = format!(
                "Extract a JSON object matching this schema from the text. Return JSON only.\nSchema: {}\nText: {}",
                schema, text
            );
            let sys = crate::server::llm_client::Message::system(
                "You extract structured JSON. No markdown.",
            );
            let user = crate::server::llm_client::Message::user(&prompt);
            if let Ok(resp) = client.chat_blocking(vec![sys, user]) {
                let trimmed = resp
                    .trim()
                    .trim_start_matches("```json")
                    .trim_start_matches("```")
                    .trim_end_matches("```")
                    .trim();
                if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
                    return Ok(v);
                }
            }
        }
        Ok(Value::Null)
    }

    fn eval_cite(&self, args: &[Value]) -> Value {
        let answer = args.first().and_then(Value::as_str).unwrap_or("");
        let docs = args
            .get(1)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut citations = Vec::new();
        let tokens: Vec<&str> = answer.split_whitespace().filter(|t| t.len() > 3).collect();
        for doc in docs {
            let text = match &doc {
                Value::String(s) => s.clone(),
                Value::Object(o) => o
                    .get("content")
                    .or_else(|| o.get("text"))
                    .or_else(|| o.get("body"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                _ => String::new(),
            };
            let hits = tokens
                .iter()
                .filter(|t| text.to_lowercase().contains(&t.to_lowercase()))
                .count();
            if hits > 0 {
                citations.push(json!({
                    "doc": doc,
                    "score": hits as f64 / tokens.len().max(1) as f64
                }));
            }
        }
        json!({ "citations": citations })
    }

    fn eval_grounded(&self, args: &[Value]) -> Value {
        let cite = self.eval_cite(args);
        let n = cite
            .get("citations")
            .and_then(Value::as_array)
            .map(|a| a.len())
            .unwrap_or(0);
        let score = if n == 0 {
            0.0
        } else {
            (n as f64).min(5.0) / 5.0
        };
        json!({
            "score": score,
            "supported": cite.get("citations").cloned().unwrap_or(json!([])),
            "contradictions": []
        })
    }
}

/// Parse an `AS OF` timestamp argument into epoch microseconds (inclusive of the
/// whole millisecond). Accepts a number (epoch milliseconds) or an RFC3339 string.
pub(crate) fn as_of_micros(v: &Value) -> DbResult<u64> {
    parse_as_of_micros(v)
}

fn parse_as_of_micros(v: &Value) -> DbResult<u64> {
    let millis: u64 = if let Some(n) = v.as_u64() {
        n
    } else if let Some(f) = v.as_f64() {
        if f < 0.0 {
            return Err(DbError::ExecutionError(
                "DOC_AS_OF: timestamp must be non-negative".to_string(),
            ));
        }
        f as u64
    } else if let Some(s) = v.as_str() {
        match chrono::DateTime::parse_from_rfc3339(s) {
            Ok(dt) => dt.timestamp_millis().max(0) as u64,
            Err(_) => {
                return Err(DbError::ExecutionError(
                    "DOC_AS_OF: string timestamp must be RFC3339 (e.g. 2026-07-13T12:00:00Z)"
                        .to_string(),
                ))
            }
        }
    } else {
        return Err(DbError::ExecutionError(
            "DOC_AS_OF: timestamp must be epoch millis (number) or an RFC3339 string".to_string(),
        ));
    };
    Ok(millis.saturating_mul(1000).saturating_add(999))
}

fn check_same_dimension(a: &[f32], b: &[f32], fname: &str) -> DbResult<()> {
    if a.len() != b.len() {
        return Err(DbError::ExecutionError(format!(
            "{}: vectors must have the same dimension ({} vs {})",
            fname,
            a.len(),
            b.len()
        )));
    }
    Ok(())
}

#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[inline]
fn magnitude(v: &[f32]) -> f32 {
    v.iter().map(|x| x * x).sum::<f32>().sqrt()
}

/// Cosine similarity; 0 when either vector has zero length.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let (m1, m2) = (magnitude(a), magnitude(b));
    if m1 == 0.0 || m2 == 0.0 {
        0.0
    } else {
        dot(a, b) / (m1 * m2)
    }
}

/// `FULLTEXT` score of one field: +10 per exact term match, +5 per term
/// within `max_distance` edits (the index's own scoring, with the distance
/// the caller asked for instead of a fixed 2).
fn fulltext_score(query_terms: &[String], text: &str, max_distance: usize) -> f64 {
    let doc_terms = crate::storage::tokenize(text);
    let mut score = 0u64;
    for q in query_terms {
        let q_len = q.chars().count();
        for d in &doc_terms {
            if q == d {
                score += 10;
            } else if max_distance > 0
                && q_len.abs_diff(d.chars().count()) <= max_distance
                && crate::storage::levenshtein_distance(q, d) <= max_distance
            {
                score += 5;
            }
        }
    }
    score as f64
}

/// Levenshtein distance, or an error past the input-length cap (it used to
/// silently return the longer length).
fn bounded_levenshtein(a: &str, b: &str, fname: &str) -> DbResult<usize> {
    crate::storage::levenshtein_distance_bounded(a, b).ok_or_else(|| {
        DbError::ExecutionError(format!(
            "{}: inputs longer than {} characters are not supported",
            fname,
            crate::storage::LEVENSHTEIN_MAX_CHARS
        ))
    })
}

/// Whether `TRY` may replace this error with its fallback.
///
/// Only errors about the *value* being computed: a bad argument, an
/// unparseable date, a failed `ASSERT`. Permission, storage, timeout and
/// cluster errors are not the row's fault and must still stop the query, and
/// neither must the executor's own budget, which is reported as an
/// `ExecutionError` naming `SOLIDB_MAX_INTERMEDIATE_ROWS` (the deadline is
/// re-checked by the caller).
fn is_recoverable(e: &DbError) -> bool {
    match e {
        DbError::ExecutionError(msg) => !msg.contains("SOLIDB_MAX_INTERMEDIATE_ROWS"),
        DbError::BadRequest(_) | DbError::InvalidDocument(_) | DbError::JsonError(_) => true,
        _ => false,
    }
}

/// `EXISTS(path, "type", typeName)`: whether a present value has the given
/// type (`null`, `bool`/`boolean`, `numeric`/`number`, `string`, `array`,
/// `object`). With no extra arguments, presence alone is enough.
fn exists_type_matches(v: &Value, extra: &[Value]) -> DbResult<bool> {
    match extra {
        [] => Ok(true),
        [Value::String(kind), Value::String(t)] if kind.eq_ignore_ascii_case("type") => {
            Ok(match t.to_ascii_lowercase().as_str() {
                "null" => v.is_null(),
                "bool" | "boolean" => v.is_boolean(),
                "numeric" | "number" => v.is_number(),
                "string" => v.is_string(),
                "array" => v.is_array(),
                "object" => v.is_object(),
                other => {
                    return Err(DbError::ExecutionError(format!(
                "EXISTS: unknown type '{}' (expected null, bool, numeric, string, array, object)",
                other
            )))
                }
            })
        }
        _ => Err(DbError::ExecutionError(
            "EXISTS: expected EXISTS(path) or EXISTS(path, \"type\", typeName)".to_string(),
        )),
    }
}

/// Uniform sample of up to `k` items (Algorithm R), in no particular order.
fn reservoir_sample(items: Vec<Value>, k: usize) -> Vec<Value> {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut reservoir: Vec<Value> = Vec::with_capacity(k.min(items.len()));
    for (i, item) in items.into_iter().enumerate() {
        if i < k {
            reservoir.push(item);
        } else {
            let j = rng.gen_range(0..=i);
            if j < k {
                reservoir[j] = item;
            }
        }
    }
    // Algorithm R keeps the first `k` items in scan order when the input is
    // no larger than `k`; shuffle so the result order is random either way.
    use rand::seq::SliceRandom;
    reservoir.shuffle(&mut rng);
    reservoir
}
