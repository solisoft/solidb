//! `NEIGHBORS` and `GRAPH_RAG` SDBQL functions — local Graph RAG.
//!
//! `NEIGHBORS(edge_collection, seeds, options?)` expands a set of seed vertices
//! N hops over an edge collection and returns the reached documents scored by
//! hop distance. `GRAPH_RAG(seed_collection, vector_index, edge_collection,
//! query_vector, options?)` first retrieves the seeds by vector (or hybrid)
//! similarity, then expands them the same way — the full retrieve→expand
//! pipeline in one query.
//!
//! Both share `EdgeExpander` (see `graph.rs`) for the actual BFS and a pure
//! `graph_aggregate` for scoring/dedup, which is unit-tested without storage.

use super::super::QueryExecutor;
use super::graph::{EdgeExpander, Reached};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::EdgeDirection;
use crate::server::llm_client::{LLMClient, Message};
use crate::storage::index::extract_field_value;
use crate::storage::{Collection, VectorMetric};
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::graph::pagerank_directed;

/// Allowed option keys for `RERANK` (strictly validated).
const RERANK_OPTIONS: &[&str] = &["mode", "field", "limit", "provider", "model"];
/// Allowed option keys for `RAG_PIPELINE` (strictly validated).
const RAG_PIPELINE_OPTIONS: &[&str] = &["text_query", "limit"];

#[derive(Clone, Copy)]
enum Combine {
    /// Keep the single strongest contribution to a vertex (default).
    Max,
    /// Sum every contribution (centrality-style recall).
    Sum,
}

struct ExpandOptions {
    hops: usize,
    direction: EdgeDirection,
    decay: f64,
    combine: Combine,
    include_seeds: bool,
    max_frontier: usize,
    limit: usize,
}

impl Default for ExpandOptions {
    fn default() -> Self {
        Self {
            hops: 2,
            direction: EdgeDirection::Outbound,
            decay: 0.6,
            combine: Combine::Max,
            include_seeds: true,
            max_frontier: 10_000,
            limit: 20,
        }
    }
}

struct ScoredHit {
    id: String,
    score: f64,
    best_contrib: f64,
    hops: usize,
    seed: bool,
    seed_score: Option<f64>,
    via: Option<String>,
}

impl<'a> QueryExecutor<'a> {
    /// Public API: `NEIGHBORS(edge_collection, seeds, options?)`
    pub fn neighbors(
        &self,
        edge_collection: &str,
        seeds: Value,
        options: Option<Value>,
    ) -> DbResult<Value> {
        let mut args = vec![json!(edge_collection), seeds];
        if let Some(o) = options {
            args.push(o);
        }
        self.eval_neighbors(&args)
    }

    /// Public API: `GRAPH_RAG(seed_collection, vector_index, edge_collection, query_vector, options?)`
    pub fn graph_rag(
        &self,
        seed_collection: &str,
        vector_index: &str,
        edge_collection: &str,
        query_vector: Value,
        options: Option<Value>,
    ) -> DbResult<Value> {
        let mut args = vec![
            json!(seed_collection),
            json!(vector_index),
            json!(edge_collection),
            query_vector,
        ];
        if let Some(o) = options {
            args.push(o);
        }
        self.eval_graph_rag(&args)
    }

    /// Public API: `COMMUNITY_SEARCH(query_text, options?)`
    pub fn community_search(&self, query_text: &str, options: Option<Value>) -> DbResult<Value> {
        let mut args = vec![json!(query_text)];
        if let Some(o) = options {
            args.push(o);
        }
        self.eval_community_search(&args)
    }

    /// `NEIGHBORS(edge_collection, seeds, options?)`
    pub(crate) fn eval_neighbors(&self, args: &[Value]) -> DbResult<Value> {
        if args.len() < 2 || args.len() > 3 {
            return Err(DbError::BadRequest(
                "NEIGHBORS requires 2-3 arguments: edge_collection, seeds, [options]".to_string(),
            ));
        }
        let edge_name = args[0].as_str().ok_or_else(|| {
            DbError::BadRequest("NEIGHBORS: edge_collection must be a string".to_string())
        })?;
        let opts_val = args.get(2);
        check_options(opts_val, &[EXPAND_OPTIONS, NEIGHBORS_OPTIONS], "NEIGHBORS")?;
        let opts = parse_expand_options(opts_val, "NEIGHBORS")?;
        let seeds = parse_seeds(&args[1], opts_val, "NEIGHBORS")?;
        self.graph_rag_run(edge_name, seeds, &opts)
    }

    /// `GRAPH_RAG(seed_collection, vector_index, edge_collection, query_vector, options?)`
    pub(crate) fn eval_graph_rag(&self, args: &[Value]) -> DbResult<Value> {
        if args.len() < 4 || args.len() > 5 {
            return Err(DbError::BadRequest(
                "GRAPH_RAG requires 4-5 arguments: seed_collection, vector_index, edge_collection, query_vector, [options]".to_string(),
            ));
        }
        let seed_collection = args[0].as_str().ok_or_else(|| {
            DbError::BadRequest("GRAPH_RAG: seed_collection must be a string".to_string())
        })?;
        let vector_index = args[1].as_str().ok_or_else(|| {
            DbError::BadRequest("GRAPH_RAG: vector_index must be a string".to_string())
        })?;
        let edge_name = args[2].as_str().ok_or_else(|| {
            DbError::BadRequest("GRAPH_RAG: edge_collection must be a string".to_string())
        })?;
        let query_vector = Self::extract_vector_arg(&args[3], "GRAPH_RAG: query_vector")?;
        let opts_val = args.get(4);
        check_options(opts_val, &[EXPAND_OPTIONS, GRAPH_RAG_OPTIONS], "GRAPH_RAG")?;
        let opts = parse_expand_options(opts_val, "GRAPH_RAG")?;

        let seed_mode = opt_str(opts_val, "seed_mode", "GRAPH_RAG")?.unwrap_or("vector");
        let seed_limit = opt_u64(opts_val, "seed_limit", "GRAPH_RAG")?.unwrap_or(10) as usize;
        let ef = opt_u64(opts_val, "ef", "GRAPH_RAG")?.map(|v| v as usize);

        let coll = self.get_collection(seed_collection)?;
        let mut seeds: Vec<(String, f64)> = Vec::new();
        match seed_mode {
            "hybrid" => {
                let fulltext_field =
                    opt_str(opts_val, "fulltext_field", "GRAPH_RAG")?.ok_or_else(|| {
                        DbError::BadRequest(
                            "GRAPH_RAG: seed_mode 'hybrid' requires options.fulltext_field"
                                .to_string(),
                        )
                    })?;
                let text_query =
                    opt_str(opts_val, "text_query", "GRAPH_RAG")?.ok_or_else(|| {
                        DbError::BadRequest(
                            "GRAPH_RAG: seed_mode 'hybrid' requires options.text_query".to_string(),
                        )
                    })?;
                let hopts = parse_hybrid_options(opts_val, seed_limit, "GRAPH_RAG")?;
                for hit in coll.hybrid_search(
                    vector_index,
                    fulltext_field,
                    &query_vector,
                    text_query,
                    &hopts,
                )? {
                    // Derive the vertex id from `doc_key`, never from the hit's
                    // `_id`: a document's `_id` is built from the column-family
                    // name, so under a database context it reads
                    // "<db>:<collection>/<key>" while edges reference vertices
                    // as "<collection>/<key>".
                    seeds.push((
                        format!("{}/{}", seed_collection, hit.doc_key),
                        hit.score as f64,
                    ));
                }
            }
            "vector" => {
                let metric = vector_index_metric(&coll, vector_index, "GRAPH_RAG")?;
                for r in coll.vector_search(vector_index, &query_vector, seed_limit, ef)? {
                    seeds.push((
                        format!("{}/{}", seed_collection, r.doc_key),
                        similarity_weight(metric, r.score as f64),
                    ));
                }
            }
            other => {
                return Err(DbError::BadRequest(format!(
                    "GRAPH_RAG: unknown seed_mode '{}' (expected 'vector' or 'hybrid')",
                    other
                )));
            }
        }
        self.graph_rag_run(edge_name, seeds, &opts)
    }

    /// `COMMUNITY_SEARCH(query_text, options?)` — global GraphRAG retrieval:
    /// find community summaries (from a prior build) by fulltext relevance.
    /// `options`: `{ run_id?, edge_collection?, limit }`. When `run_id` is
    /// omitted it defaults to the latest run recorded for `edge_collection`.
    pub(crate) fn eval_community_search(&self, args: &[Value]) -> DbResult<Value> {
        if args.is_empty() || args.len() > 2 {
            return Err(DbError::BadRequest(
                "COMMUNITY_SEARCH requires 1-2 arguments: query_text, [options]".to_string(),
            ));
        }
        let query_text = args[0].as_str().ok_or_else(|| {
            DbError::BadRequest("COMMUNITY_SEARCH: query_text must be a string".to_string())
        })?;
        let opts = args.get(1);
        check_options(opts, &[COMMUNITY_SEARCH_OPTIONS], "COMMUNITY_SEARCH")?;
        let limit = opt_u64(opts, "limit", "COMMUNITY_SEARCH")?.unwrap_or(5) as usize;

        // Resolve the run to filter on: explicit run_id, else the latest run
        // recorded for the edge collection in _graph_runs.
        let run_id: Option<String> = match opt_str(opts, "run_id", "COMMUNITY_SEARCH")? {
            Some(r) => Some(r.to_string()),
            None => opt_str(opts, "edge_collection", "COMMUNITY_SEARCH")?.and_then(|ec| {
                self.get_collection("_graph_runs").ok().and_then(|runs| {
                    runs.get(ec).ok().and_then(|d| {
                        d.to_value()
                            .get("latest_run_id")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    })
                })
            }),
        };

        let summaries = match self.get_collection("_community_summaries") {
            Ok(c) => c,
            Err(_) => return Ok(Value::Array(vec![])),
        };

        // Prefer the fulltext index created during community build; fall back to
        // a term-overlap scan when no index exists or fulltext yields no hits.
        if !summaries.list_fulltext_indexes().is_empty() {
            let ft =
                self.community_search_fulltext(&summaries, query_text, run_id.as_deref(), limit)?;
            if ft.as_array().is_some_and(|a| !a.is_empty()) {
                return Ok(ft);
            }
        }
        Ok(Value::Array(community_search_token_overlap(
            summaries.scan(None).into_iter().map(|d| d.to_value()),
            query_text,
            run_id.as_deref(),
            limit,
        )))
    }

    fn community_search_fulltext(
        &self,
        summaries: &Collection,
        query_text: &str,
        run_id: Option<&str>,
        limit: usize,
    ) -> DbResult<Value> {
        let matches = summaries.fulltext_search(
            query_text,
            Some(vec!["summary".to_string(), "title".to_string()]),
            limit.saturating_mul(4),
        )?;
        let mut scored: Vec<(f64, Value)> = Vec::new();
        for m in matches {
            // A summary deleted between the index hit and the fetch (a
            // concurrent community build purging the previous run) must not
            // fail the whole search.
            let Ok(doc) = summaries.get(&m.doc_key) else {
                continue;
            };
            let v = doc.to_value();
            if let Some(rid) = run_id {
                if v.get("run_id").and_then(|x| x.as_str()) != Some(rid) {
                    continue;
                }
            }
            let mut obj = Map::new();
            for f in [
                "community_id",
                "title",
                "summary",
                "keywords",
                "size",
                "run_id",
            ] {
                obj.insert(f.to_string(), v.get(f).cloned().unwrap_or(Value::Null));
            }
            obj.insert("score".to_string(), json!(m.score));
            scored.push((m.score, Value::Object(obj)));
        }
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        let out: Vec<Value> = scored.into_iter().take(limit).map(|(_, v)| v).collect();
        Ok(Value::Array(out))
    }

    /// Shared: expand `seeds` over `edge_name`, score, dedup, hydrate.
    fn graph_rag_run(
        &self,
        edge_name: &str,
        seeds: Vec<(String, f64)>,
        opts: &ExpandOptions,
    ) -> DbResult<Value> {
        if seeds.is_empty() {
            return Ok(Value::Array(vec![]));
        }
        let edge = self.get_collection(edge_name)?;
        let expander = EdgeExpander::new(&edge, opts.direction.clone(), true);

        let reached_per_seed: Vec<Vec<Reached>> = seeds
            .iter()
            .map(|(id, _)| expander.bfs_from(id, opts.hops, opts.max_frontier))
            .collect();

        let scored = graph_aggregate(&seeds, &reached_per_seed, opts);

        // `limit` counts *returned* hits, so truncate after hydration: a
        // dangling target must not consume a slot a valid hit could fill.
        let mut out = Vec::with_capacity(opts.limit.min(scored.len()));
        for hit in scored {
            if out.len() >= opts.limit {
                break;
            }
            // Resolve "coll/key" -> document; skip dangling / heterogeneous
            // targets (mirrors the traversal clause's tolerant hydration).
            let doc = match hit.id.split_once('/') {
                Some((coll, key)) => match self.get_collection(coll).and_then(|c| c.get(key)) {
                    Ok(d) => d.to_value(),
                    Err(_) => continue,
                },
                None => continue,
            };
            let mut obj = Map::new();
            obj.insert("doc".to_string(), doc);
            obj.insert("id".to_string(), Value::String(hit.id));
            obj.insert("score".to_string(), json!(hit.score));
            obj.insert("hops".to_string(), json!(hit.hops));
            obj.insert("seed".to_string(), Value::Bool(hit.seed));
            obj.insert(
                "seed_score".to_string(),
                hit.seed_score.map(|s| json!(s)).unwrap_or(Value::Null),
            );
            obj.insert(
                "via".to_string(),
                hit.via.map(Value::String).unwrap_or(Value::Null),
            );
            out.push(Value::Object(obj));
        }
        Ok(Value::Array(out))
    }

    // PAGERANK(edge_collection [, options?])
    // Runs PageRank over the directed graph defined by the edge collection.
    // Returns array of objects: [{ node: "...", score: 0.123 }, ...] sorted by score desc.
    pub(crate) fn eval_pagerank(&self, args: &[Value]) -> DbResult<Value> {
        if args.is_empty() || args.len() > 2 {
            return Err(DbError::BadRequest(
                "PAGERANK requires 1-2 args: edge_collection, [options]".to_string(),
            ));
        }
        let edge_name = args[0].as_str().ok_or_else(|| {
            DbError::BadRequest("PAGERANK: edge_collection must be string".into())
        })?;

        let opts = args.get(1);
        let damping = opt_f64(opts, "damping", "PAGERANK")?.unwrap_or(0.85);
        let iters = opt_u64(opts, "iterations", "PAGERANK")?.unwrap_or(20) as usize;
        let limit = opt_u64(opts, "limit", "PAGERANK")?.unwrap_or(100) as usize;

        // Validate options strictly rather than silently producing garbage scores.
        if !(damping > 0.0 && damping <= 1.0) {
            return Err(DbError::BadRequest(
                "PAGERANK: damping must be in (0, 1]".to_string(),
            ));
        }
        if iters == 0 || iters > 1000 {
            return Err(DbError::BadRequest(
                "PAGERANK: iterations must be between 1 and 1000".to_string(),
            ));
        }

        let edge_coll = self.get_collection(edge_name)?;

        // Build directed out-neighbors for PAGERANK (with weights if present).
        let mut out_neighbors: std::collections::HashMap<String, Vec<(String, f64)>> =
            std::collections::HashMap::new();
        for doc in edge_coll.scan(None) {
            let v = doc.to_value();
            if let (Some(from), Some(to)) = (
                v.get("_from").and_then(|x| x.as_str()),
                v.get("_to").and_then(|x| x.as_str()),
            ) {
                let w = v.get("weight").and_then(|x| x.as_f64()).unwrap_or(1.0);
                out_neighbors
                    .entry(from.to_string())
                    .or_default()
                    .push((to.to_string(), w));
            }
        }

        let mut scored = pagerank_directed(&out_neighbors, damping, iters, 1e-6);
        if scored.len() > limit {
            scored.truncate(limit);
        }

        let out: Vec<Value> = scored
            .into_iter()
            .map(|(node, score)| json!({ "node": node, "score": score }))
            .collect();

        Ok(Value::Array(out))
    }

    // DEGREE_CENTRALITY(edge_collection)
    pub(crate) fn eval_degree_centrality(&self, args: &[Value]) -> DbResult<Value> {
        if args.is_empty() {
            return Err(DbError::BadRequest(
                "DEGREE_CENTRALITY(edge_collection) required".to_string(),
            ));
        }
        let edge_name = args[0].as_str().ok_or_else(|| {
            DbError::BadRequest("DEGREE_CENTRALITY: edge_collection must be string".into())
        })?;

        let edge_coll = self.get_collection(edge_name)?;
        let mut degree: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

        for doc in edge_coll.scan(None) {
            let v = doc.to_value();
            if let Some(from) = v.get("_from").and_then(|x| x.as_str()) {
                *degree.entry(from.to_string()).or_insert(0.0) += 1.0;
            }
            if let Some(to) = v.get("_to").and_then(|x| x.as_str()) {
                *degree.entry(to.to_string()).or_insert(0.0) += 1.0;
            }
        }

        let mut pairs: Vec<_> = degree.into_iter().collect();
        pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let out: Vec<Value> = pairs
            .into_iter()
            .map(|(node, deg)| json!({ "node": node, "degree": deg }))
            .collect();

        Ok(Value::Array(out))
    }

    /// RERANK(query, docs, options?) — reorder retrieved documents by relevance to
    /// `query`. `options` (all optional): `mode` ("lexical" default, or "llm"),
    /// `field` (dotted path to the text; auto-detected otherwise), `limit`,
    /// `provider`, `model`. LLM mode falls back to lexical on any failure, so the
    /// function is always safe to call.
    pub(crate) fn eval_rerank(&self, args: &[Value]) -> DbResult<Value> {
        if args.len() < 2 || args.len() > 3 {
            return Err(DbError::BadRequest(
                "RERANK requires 2-3 args: query, docs, [options]".to_string(),
            ));
        }
        let query = args[0]
            .as_str()
            .ok_or_else(|| DbError::BadRequest("RERANK: query must be a string".to_string()))?;
        let docs = args[1]
            .as_array()
            .ok_or_else(|| DbError::BadRequest("RERANK: docs must be an array".to_string()))?;

        let opts = args.get(2);
        check_options(opts, &[RERANK_OPTIONS], "RERANK")?;
        let mode = opt_str(opts, "mode", "RERANK")?.unwrap_or("lexical");
        let field = opt_str(opts, "field", "RERANK")?;
        let limit = opt_u64(opts, "limit", "RERANK")?.map(|n| n as usize);
        let provider = opt_str(opts, "provider", "RERANK")?;
        let model = opt_str(opts, "model", "RERANK")?;

        if docs.is_empty() {
            return Ok(Value::Array(vec![]));
        }

        let texts: Vec<String> = docs.iter().map(|d| rerank_extract_text(d, field)).collect();

        let order: Vec<usize> = if mode.eq_ignore_ascii_case("llm") {
            match self.rerank_llm(query, &texts, provider, model) {
                Ok(ord) => ord,
                Err(e) => {
                    tracing::warn!("RERANK llm mode failed ({}); falling back to lexical", e);
                    rerank_lexical_order(query, &texts)
                }
            }
        } else if mode.eq_ignore_ascii_case("lexical") {
            rerank_lexical_order(query, &texts)
        } else {
            return Err(DbError::BadRequest(format!(
                "RERANK: unknown mode '{}' (expected 'lexical' or 'llm')",
                mode
            )));
        };

        let mut out: Vec<Value> = order.into_iter().map(|i| docs[i].clone()).collect();
        if let Some(l) = limit {
            out.truncate(l);
        }
        Ok(Value::Array(out))
    }

    /// LLM-backed rerank: ask the chat model to return the document indices in
    /// relevance order. Returns a full permutation (indices the model omits are
    /// appended in their original order).
    fn rerank_llm(
        &self,
        query: &str,
        texts: &[String],
        provider: Option<&str>,
        model: Option<&str>,
    ) -> DbResult<Vec<usize>> {
        let db = self.database.as_deref().unwrap_or("_system");
        let client =
            LLMClient::from_storage(self.storage, db, provider, model.map(|s| s.to_string()))?;

        let mut listing = String::new();
        for (i, t) in texts.iter().enumerate() {
            let snippet: String = t.chars().take(500).collect();
            listing.push_str(&format!("[{}] {}\n", i, snippet.replace('\n', " ")));
        }
        let sys = Message::system(
            "You are a search result re-ranker. Given a query and numbered documents, \
             respond with ONLY a JSON array of the document indices ordered from most to \
             least relevant, e.g. [2,0,1]. No prose, no code fences.",
        );
        let user = Message::user(&format!(
            "Query: {}\n\nDocuments:\n{}\nReturn the JSON array of indices, most relevant first.",
            query, listing
        ));
        let resp = client.chat_blocking(vec![sys, user])?;
        parse_index_list(&resp, texts.len())
    }

    /// RAG_PIPELINE(name, query_vector, options?) — run a stored retrieve→expand→
    /// rerank pipeline. The definition lives in the `_rag_pipelines` collection
    /// keyed by `name`:
    /// ```json
    /// { "_key": "faq", "seed_collection": "docs", "vector_index": "emb",
    ///   "edge_collection": "links", "retrieve_options": { "hops": 1, "seed_limit": 20 },
    ///   "rerank": { "mode": "lexical", "field": "doc.content", "limit": 5 } }
    /// ```
    /// `options` may carry `text_query` (for lexical/LLM rerank) and override `limit`.
    pub(crate) fn eval_rag_pipeline(&self, args: &[Value]) -> DbResult<Value> {
        if args.len() < 2 || args.len() > 3 {
            return Err(DbError::BadRequest(
                "RAG_PIPELINE requires 2-3 args: name, query_vector, [options]".to_string(),
            ));
        }
        let name = args[0].as_str().ok_or_else(|| {
            DbError::BadRequest("RAG_PIPELINE: name must be a string".to_string())
        })?;
        let query_vector = args[1].clone();
        let call_opts = args.get(2);
        check_options(call_opts, &[RAG_PIPELINE_OPTIONS], "RAG_PIPELINE")?;

        // Load the pipeline definition.
        let pipelines = self.get_collection("_rag_pipelines")?;
        let def = pipelines.get(name).map_err(|_| {
            DbError::BadRequest(format!("RAG_PIPELINE: pipeline '{}' not found", name))
        })?;
        let def = def.to_value();

        let seed_collection = def
            .get("seed_collection")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                DbError::BadRequest("RAG_PIPELINE: definition missing seed_collection".to_string())
            })?;
        let vector_index = def
            .get("vector_index")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                DbError::BadRequest("RAG_PIPELINE: definition missing vector_index".to_string())
            })?;
        let edge_collection = def
            .get("edge_collection")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                DbError::BadRequest("RAG_PIPELINE: definition missing edge_collection".to_string())
            })?;
        let retrieve_options = def
            .get("retrieve_options")
            .cloned()
            .unwrap_or_else(|| json!({}));

        // Retrieve via the existing, fully-validated GRAPH_RAG path.
        let gr_args = vec![
            json!(seed_collection),
            json!(vector_index),
            json!(edge_collection),
            query_vector,
            retrieve_options,
        ];
        let retrieved = self.eval_graph_rag(&gr_args)?;
        let hits = match retrieved {
            Value::Array(a) => a,
            other => return Ok(other),
        };

        // Optional rerank stage.
        let rerank_cfg = def.get("rerank");
        let text_query = call_opts
            .and_then(|o| o.get("text_query"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                rerank_cfg
                    .and_then(|r| r.get("text_query"))
                    .and_then(|v| v.as_str())
            });

        let mut result = hits;
        if let (Some(cfg), Some(q)) = (rerank_cfg, text_query) {
            let mode = cfg
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("lexical");
            let field = cfg.get("field").and_then(|v| v.as_str());
            let texts: Vec<String> = result
                .iter()
                .map(|d| rerank_extract_text(d, field))
                .collect();
            let order = if mode.eq_ignore_ascii_case("llm") {
                let provider = cfg.get("provider").and_then(|v| v.as_str());
                let model = cfg.get("model").and_then(|v| v.as_str());
                self.rerank_llm(q, &texts, provider, model)
                    .unwrap_or_else(|_| rerank_lexical_order(q, &texts))
            } else {
                rerank_lexical_order(q, &texts)
            };
            result = order.into_iter().map(|i| result[i].clone()).collect();
        }

        // Apply the final limit (call option wins over the stored one).
        let limit = call_opts
            .and_then(|o| o.get("limit"))
            .and_then(|v| v.as_u64())
            .or_else(|| {
                rerank_cfg
                    .and_then(|r| r.get("limit"))
                    .and_then(|v| v.as_u64())
            })
            .map(|n| n as usize);
        if let Some(l) = limit {
            result.truncate(l);
        }

        Ok(Value::Array(result))
    }
}

/// Pure scoring/dedup: fold seed similarities and per-seed BFS reaches into a
/// ranked hit list. `contrib = seed_sim * decay^hop`, combined per vertex by
/// `max` (default) or `sum`, keeping the minimum hop and the seed responsible
/// for the strongest single contribution (`via`). The caller applies
/// `opts.limit` after hydration.
fn graph_aggregate(
    seeds: &[(String, f64)],
    reached: &[Vec<Reached>],
    opts: &ExpandOptions,
) -> Vec<ScoredHit> {
    let mut map: HashMap<String, ScoredHit> = HashMap::new();

    // `Max` must not floor scores at zero: a seed weight can legitimately be
    // negative (raw cosine similarity, dot product), and `0.0.max(-0.3)` would
    // silently flatten every such hit to a tie.
    let init_score = match opts.combine {
        Combine::Max => f64::NEG_INFINITY,
        Combine::Sum => 0.0,
    };

    let add = |map: &mut HashMap<String, ScoredHit>,
               id: &str,
               contrib: f64,
               hops: usize,
               via: Option<String>| {
        let e = map.entry(id.to_string()).or_insert_with(|| ScoredHit {
            id: id.to_string(),
            score: init_score,
            best_contrib: f64::NEG_INFINITY,
            hops,
            seed: false,
            seed_score: None,
            via: None,
        });
        match opts.combine {
            Combine::Max => e.score = e.score.max(contrib),
            Combine::Sum => e.score += contrib,
        }
        if contrib > e.best_contrib {
            e.best_contrib = contrib;
            e.via = via;
        }
        if hops < e.hops {
            e.hops = hops;
        }
    };

    let seed_ids: HashSet<&str> = seeds.iter().map(|(id, _)| id.as_str()).collect();

    if opts.include_seeds {
        for (id, sim) in seeds {
            add(&mut map, id, *sim, 0, None);
            if let Some(e) = map.get_mut(id) {
                e.seed = true;
                e.seed_score = Some(e.seed_score.map_or(*sim, |p| p.max(*sim)));
            }
        }
    }

    for (i, reaches) in reached.iter().enumerate() {
        let (seed_id, sim) = &seeds[i];
        for r in reaches {
            // `include_seeds: false` excludes seeds from the output entirely,
            // including a seed reached from a *different* seed — otherwise the
            // option would only half-apply.
            if !opts.include_seeds && seed_ids.contains(r.id.as_str()) {
                continue;
            }
            let contrib = sim * opts.decay.powi(r.depth as i32);
            add(&mut map, &r.id, contrib, r.depth, Some(seed_id.clone()));
        }
    }

    let mut hits: Vec<ScoredHit> = map.into_values().collect();
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.id.cmp(&b.id))
    });
    hits
}

fn parse_hybrid_options(
    opts_val: Option<&Value>,
    seed_limit: usize,
    fname: &str,
) -> DbResult<crate::storage::HybridSearchOptions> {
    use crate::storage::{FusionMethod, HybridSearchOptions};

    let defaults = HybridSearchOptions::default();
    let fusion = match opt_str(opts_val, "fusion", fname)? {
        Some(s) => FusionMethod::parse(s).ok_or_else(|| {
            DbError::BadRequest(format!(
                "{}: unknown fusion '{}' (expected weighted|rrf)",
                fname, s
            ))
        })?,
        None => defaults.fusion,
    };
    Ok(HybridSearchOptions {
        vector_weight: opt_f64(opts_val, "vector_weight", fname)?
            .map(|v| v as f32)
            .unwrap_or(defaults.vector_weight),
        text_weight: opt_f64(opts_val, "text_weight", fname)?
            .map(|v| v as f32)
            .unwrap_or(defaults.text_weight),
        limit: seed_limit,
        fusion,
    })
}

/// The metric of `index` on `coll`. Read from the stored config rather than
/// `get_vector_index`, which would page the whole HNSW graph in from disk.
fn vector_index_metric(coll: &Collection, index: &str, fname: &str) -> DbResult<VectorMetric> {
    coll.get_all_vector_index_configs()
        .into_iter()
        .find(|c| c.name == index)
        .map(|c| c.metric)
        .ok_or_else(|| DbError::BadRequest(format!("{}: unknown vector index '{}'", fname, index)))
}

/// Map a raw `vector_search` score onto a strictly positive weight that grows
/// with similarity.
///
/// `VectorSearchResult::score` means a different thing per metric: a cosine
/// similarity in `[-1, 1]`, an L2 **distance** (smaller is closer), or an
/// unbounded dot product. Hop-decay scoring multiplies this weight by
/// `decay^hop` and ranks descending, so it needs "bigger is closer" and a sign
/// that survives the multiplication. Each mapping below is strictly monotonic
/// in similarity, so the seed ordering the index produced is preserved.
fn similarity_weight(metric: VectorMetric, score: f64) -> f64 {
    match metric {
        // [-1, 1] -> [0, 1]
        VectorMetric::Cosine => (score + 1.0) / 2.0,
        // distance in [0, inf) -> (0, 1], decreasing
        VectorMetric::Euclidean => 1.0 / (1.0 + score.max(0.0)),
        // (-inf, inf) -> (0, 1), increasing
        VectorMetric::DotProduct => 1.0 / (1.0 + (-score).exp()),
    }
}

/// Expansion options, shared by `NEIGHBORS` and `GRAPH_RAG`.
const EXPAND_OPTIONS: &[&str] = &[
    "hops",
    "direction",
    "decay",
    "combine",
    "include_seeds",
    "max_frontier",
    "limit",
];

/// `NEIGHBORS`-only: qualifies bare seed keys. `GRAPH_RAG` derives seed ids
/// from the collection it searched, so accepting it there would be misleading.
const NEIGHBORS_OPTIONS: &[&str] = &["seed_collection"];

/// Options `GRAPH_RAG` accepts on top of [`EXPAND_OPTIONS`], for seed retrieval.
const GRAPH_RAG_OPTIONS: &[&str] = &[
    "seed_mode",
    "seed_limit",
    "ef",
    "fulltext_field",
    "text_query",
    "vector_weight",
    "text_weight",
    "fusion",
];

const COMMUNITY_SEARCH_OPTIONS: &[&str] = &["run_id", "edge_collection", "limit"];

/// Reject a non-object `options` argument and any key outside `allowed`, so a
/// typo (`seedLimit`) fails loudly instead of silently selecting a default.
fn check_options(opts_val: Option<&Value>, allowed: &[&[&str]], fname: &str) -> DbResult<()> {
    let Some(v) = opts_val else { return Ok(()) };
    if v.is_null() {
        return Ok(());
    }
    let Some(m) = v.as_object() else {
        return Err(DbError::BadRequest(format!(
            "{}: options must be an object",
            fname
        )));
    };
    for key in m.keys() {
        if !allowed.iter().any(|set| set.contains(&key.as_str())) {
            return Err(DbError::BadRequest(format!(
                "{}: unknown option '{}'",
                fname, key
            )));
        }
    }
    Ok(())
}

/// Typed option readers. A key that is present but of the wrong type is an
/// error, not a silent fallback to the default.
fn opt_u64(opts_val: Option<&Value>, key: &str, fname: &str) -> DbResult<Option<u64>> {
    match opts_val.and_then(|o| o.get(key)) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().map(Some).ok_or_else(|| {
            DbError::BadRequest(format!(
                "{}: option '{}' must be a non-negative integer",
                fname, key
            ))
        }),
    }
}

fn opt_f64(opts_val: Option<&Value>, key: &str, fname: &str) -> DbResult<Option<f64>> {
    match opts_val.and_then(|o| o.get(key)) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().map(Some).ok_or_else(|| {
            DbError::BadRequest(format!("{}: option '{}' must be a number", fname, key))
        }),
    }
}

fn opt_bool(opts_val: Option<&Value>, key: &str, fname: &str) -> DbResult<Option<bool>> {
    match opts_val.and_then(|o| o.get(key)) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| {
            DbError::BadRequest(format!("{}: option '{}' must be a boolean", fname, key))
        }),
    }
}

fn opt_str<'v>(opts_val: Option<&'v Value>, key: &str, fname: &str) -> DbResult<Option<&'v str>> {
    match opts_val.and_then(|o| o.get(key)) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| {
            DbError::BadRequest(format!("{}: option '{}' must be a string", fname, key))
        }),
    }
}

/// Token-overlap fallback for COMMUNITY_SEARCH when no fulltext index exists.
fn community_search_token_overlap(
    docs: impl Iterator<Item = Value>,
    query_text: &str,
    run_id: Option<&str>,
    limit: usize,
) -> Vec<Value> {
    let query_terms = tokenize_lower(query_text);
    let mut scored: Vec<(f64, Value)> = Vec::new();
    for v in docs {
        if let Some(rid) = run_id {
            if v.get("run_id").and_then(|x| x.as_str()) != Some(rid) {
                continue;
            }
        }
        let mut hay = String::new();
        for field in ["summary", "title"] {
            if let Some(s) = v.get(field).and_then(|x| x.as_str()) {
                hay.push_str(s);
                hay.push(' ');
            }
        }
        if let Some(kw) = v.get("keywords").and_then(|x| x.as_array()) {
            for k in kw {
                if let Some(s) = k.as_str() {
                    hay.push_str(s);
                    hay.push(' ');
                }
            }
        }
        let hay_terms: HashSet<String> = tokenize_lower(&hay).into_iter().collect();
        let hits = query_terms
            .iter()
            .filter(|t| hay_terms.contains(*t))
            .count();
        let score = if query_terms.is_empty() {
            1.0
        } else {
            hits as f64 / query_terms.len() as f64
        };
        if score <= 0.0 {
            continue;
        }
        let mut obj = Map::new();
        for f in [
            "community_id",
            "title",
            "summary",
            "keywords",
            "size",
            "run_id",
        ] {
            obj.insert(f.to_string(), v.get(f).cloned().unwrap_or(Value::Null));
        }
        obj.insert("score".to_string(), json!(score));
        scored.push((score, Value::Object(obj)));
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
    scored.into_iter().take(limit).map(|(_, v)| v).collect()
}

/// Split text into lowercase alphanumeric tokens of length >= 3.
fn tokenize_lower(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect()
}

// ==================== RERANK helpers ====================

/// Extract the text to rank a document by. Uses `field` (a dotted path) when
/// given; otherwise probes common text fields, including under a `doc` wrapper
/// (as produced by VECTOR_SEARCH / HYBRID_SEARCH / GRAPH_RAG).
fn rerank_extract_text(doc: &Value, field: Option<&str>) -> String {
    if let Some(f) = field {
        return value_to_text(&extract_field_value(doc, f));
    }
    const CANDIDATES: &[&str] = &[
        "content",
        "text",
        "summary",
        "body",
        "title",
        "doc.content",
        "doc.text",
        "doc.summary",
        "doc.body",
        "doc.title",
    ];
    for cand in CANDIDATES {
        if let Some(s) = extract_field_value(doc, cand).as_str() {
            if !s.trim().is_empty() {
                return s.to_string();
            }
        }
    }
    value_to_text(doc)
}

fn value_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Order document indices best→worst by query-token overlap. Ties keep the input
/// order (stable), so retrieval order is preserved when there's no signal.
fn rerank_lexical_order(query: &str, texts: &[String]) -> Vec<usize> {
    let q_tokens: HashSet<String> = tokenize_lower(query).into_iter().collect();
    let mut scored: Vec<(usize, usize)> = texts
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let matches = if q_tokens.is_empty() {
                0
            } else {
                tokenize_lower(t)
                    .into_iter()
                    .filter(|tok| q_tokens.contains(tok))
                    .count()
            };
            (i, matches)
        })
        .collect();
    // Higher score first; original index breaks ties (stable order).
    scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().map(|(i, _)| i).collect()
}

/// Parse a JSON array of indices out of an LLM response (tolerating surrounding
/// text/code fences) into a full permutation of `0..n`. Out-of-range and
/// duplicate indices are dropped; omitted indices are appended in original order.
fn parse_index_list(resp: &str, n: usize) -> DbResult<Vec<usize>> {
    let start = resp.find('[').ok_or_else(|| {
        DbError::ExecutionError("RERANK: no JSON array in LLM response".to_string())
    })?;
    let end = resp[start..]
        .find(']')
        .map(|e| start + e + 1)
        .ok_or_else(|| {
            DbError::ExecutionError("RERANK: unterminated JSON array in LLM response".to_string())
        })?;
    let arr: Vec<i64> = serde_json::from_str(&resp[start..end])
        .map_err(|e| DbError::ExecutionError(format!("RERANK: bad index list: {}", e)))?;

    let mut seen = vec![false; n];
    let mut order = Vec::with_capacity(n);
    for idx in arr {
        if idx >= 0 && (idx as usize) < n && !seen[idx as usize] {
            seen[idx as usize] = true;
            order.push(idx as usize);
        }
    }
    for (i, s) in seen.iter().enumerate() {
        if !*s {
            order.push(i);
        }
    }
    Ok(order)
}

fn parse_direction(s: &str, fname: &str) -> DbResult<EdgeDirection> {
    match s.to_lowercase().as_str() {
        "outbound" | "out" => Ok(EdgeDirection::Outbound),
        "inbound" | "in" => Ok(EdgeDirection::Inbound),
        "any" | "both" => Ok(EdgeDirection::Any),
        other => Err(DbError::BadRequest(format!(
            "{}: unknown direction '{}' (expected outbound|inbound|any)",
            fname, other
        ))),
    }
}

fn parse_expand_options(opts_val: Option<&Value>, fname: &str) -> DbResult<ExpandOptions> {
    let mut o = ExpandOptions::default();
    if let Some(v) = opt_u64(opts_val, "hops", fname)? {
        o.hops = v as usize;
    }
    if let Some(v) = opt_str(opts_val, "direction", fname)? {
        o.direction = parse_direction(v, fname)?;
    }
    if let Some(v) = opt_f64(opts_val, "decay", fname)? {
        if !(v > 0.0 && v <= 1.0) {
            return Err(DbError::BadRequest(format!(
                "{}: decay must be in (0, 1], got {}",
                fname, v
            )));
        }
        o.decay = v;
    }
    if let Some(v) = opt_str(opts_val, "combine", fname)? {
        o.combine = match v.to_lowercase().as_str() {
            "max" => Combine::Max,
            "sum" => Combine::Sum,
            other => {
                return Err(DbError::BadRequest(format!(
                    "{}: unknown combine '{}' (expected max|sum)",
                    fname, other
                )))
            }
        };
    }
    if let Some(v) = opt_bool(opts_val, "include_seeds", fname)? {
        o.include_seeds = v;
    }
    if let Some(v) = opt_u64(opts_val, "max_frontier", fname)? {
        o.max_frontier = v as usize;
    }
    if let Some(v) = opt_u64(opts_val, "limit", fname)? {
        o.limit = v as usize;
    }
    Ok(o)
}

/// Parse the `seeds` argument of `NEIGHBORS`: an array of `"coll/key"` strings
/// or `{id|_id, score?}` objects. Bare keys are qualified with
/// `options.seed_collection` when provided.
fn parse_seeds(v: &Value, opts_val: Option<&Value>, fname: &str) -> DbResult<Vec<(String, f64)>> {
    let seed_coll = opt_str(opts_val, "seed_collection", fname)?;
    let arr = v
        .as_array()
        .ok_or_else(|| DbError::BadRequest(format!("{}: seeds must be an array", fname)))?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let (raw_id, score) = match item {
            Value::String(s) => (s.clone(), 1.0),
            Value::Object(m) => {
                let id = m
                    .get("id")
                    .or_else(|| m.get("_id"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        DbError::BadRequest(format!(
                            "{}: seed object requires an 'id' or '_id' string",
                            fname
                        ))
                    })?
                    .to_string();
                let score = m.get("score").and_then(|v| v.as_f64()).unwrap_or(1.0);
                (id, score)
            }
            _ => {
                return Err(DbError::BadRequest(format!(
                    "{}: each seed must be a string or an object",
                    fname
                )))
            }
        };
        let id = if raw_id.contains('/') {
            raw_id
        } else if let Some(sc) = seed_coll {
            format!("{}/{}", sc, raw_id)
        } else {
            raw_id
        };
        out.push((id, score));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(combine: Combine, include_seeds: bool) -> ExpandOptions {
        ExpandOptions {
            hops: 3,
            direction: EdgeDirection::Outbound,
            decay: 0.5,
            combine,
            include_seeds,
            max_frontier: 1000,
            limit: 20,
        }
    }

    fn reach(id: &str, depth: usize) -> Reached {
        Reached {
            id: id.to_string(),
            depth,
        }
    }

    #[test]
    fn scores_decay_by_hop_and_marks_seeds() {
        let seeds = vec![("c/a".to_string(), 1.0)];
        let reached = vec![vec![reach("c/b", 1), reach("c/c", 2)]];
        let hits = graph_aggregate(&seeds, &reached, &opts(Combine::Max, true));
        // a (seed, 1.0), b (0.5), c (0.25)
        assert_eq!(hits[0].id, "c/a");
        assert!(hits[0].seed);
        assert_eq!(hits[0].hops, 0);
        assert_eq!(hits[1].id, "c/b");
        assert!((hits[1].score - 0.5).abs() < 1e-9);
        assert_eq!(hits[1].hops, 1);
        assert!((hits[2].score - 0.25).abs() < 1e-9);
    }

    #[test]
    fn dedup_keeps_min_hop_and_max_by_default() {
        // b reachable from a at hop 2 and from d at hop 1 -> min hop 1, max score
        let seeds = vec![("c/a".to_string(), 1.0), ("c/d".to_string(), 1.0)];
        let reached = vec![vec![reach("c/b", 2)], vec![reach("c/b", 1)]];
        let hits = graph_aggregate(&seeds, &reached, &opts(Combine::Max, false));
        let b = hits.iter().find(|h| h.id == "c/b").unwrap();
        assert_eq!(b.hops, 1);
        assert!((b.score - 0.5).abs() < 1e-9); // max(0.25, 0.5)
        assert_eq!(b.via.as_deref(), Some("c/d"));
    }

    #[test]
    fn combine_sum_adds_contributions() {
        let seeds = vec![("c/a".to_string(), 1.0), ("c/d".to_string(), 1.0)];
        let reached = vec![vec![reach("c/b", 1)], vec![reach("c/b", 1)]];
        let hits = graph_aggregate(&seeds, &reached, &opts(Combine::Sum, false));
        let b = hits.iter().find(|h| h.id == "c/b").unwrap();
        assert!((b.score - 1.0).abs() < 1e-9); // 0.5 + 0.5
    }

    #[test]
    fn exclude_seeds_when_disabled() {
        let seeds = vec![("c/a".to_string(), 1.0)];
        let reached = vec![vec![reach("c/b", 1)]];
        let hits = graph_aggregate(&seeds, &reached, &opts(Combine::Max, false));
        assert!(hits.iter().all(|h| h.id != "c/a"));
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn parse_seeds_strings_and_objects() {
        let v = json!(["users/alice", {"id": "users/bob", "score": 0.9}, {"_id": "users/carl"}]);
        let seeds = parse_seeds(&v, None, "NEIGHBORS").unwrap();
        assert_eq!(seeds[0], ("users/alice".to_string(), 1.0));
        assert_eq!(seeds[1], ("users/bob".to_string(), 0.9));
        assert_eq!(seeds[2], ("users/carl".to_string(), 1.0));
    }

    #[test]
    fn parse_seeds_qualifies_bare_keys() {
        let v = json!(["alice", "beta/x"]);
        let o = json!({ "seed_collection": "users" });
        let seeds = parse_seeds(&v, Some(&o), "NEIGHBORS").unwrap();
        assert_eq!(seeds[0].0, "users/alice"); // bare key qualified
        assert_eq!(seeds[1].0, "beta/x"); // already has a collection
    }
}
