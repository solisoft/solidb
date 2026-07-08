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
use serde_json::{json, Map, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

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
    /// `NEIGHBORS(edge_collection, seeds, options?)`
    pub(crate) fn eval_neighbors(&self, args: &[Value]) -> DbResult<Value> {
        if args.len() < 2 || args.len() > 3 {
            return Err(DbError::ExecutionError(
                "NEIGHBORS requires 2-3 arguments: edge_collection, seeds, [options]".to_string(),
            ));
        }
        let edge_name = args[0].as_str().ok_or_else(|| {
            DbError::ExecutionError("NEIGHBORS: edge_collection must be a string".to_string())
        })?;
        let opts_val = args.get(2);
        let opts = parse_expand_options(opts_val, "NEIGHBORS")?;
        let seeds = parse_seeds(&args[1], opts_val, "NEIGHBORS")?;
        self.graph_rag_run(edge_name, seeds, &opts)
    }

    /// `GRAPH_RAG(seed_collection, vector_index, edge_collection, query_vector, options?)`
    pub(crate) fn eval_graph_rag(&self, args: &[Value]) -> DbResult<Value> {
        if args.len() < 4 || args.len() > 5 {
            return Err(DbError::ExecutionError(
                "GRAPH_RAG requires 4-5 arguments: seed_collection, vector_index, edge_collection, query_vector, [options]".to_string(),
            ));
        }
        let seed_collection = args[0].as_str().ok_or_else(|| {
            DbError::ExecutionError("GRAPH_RAG: seed_collection must be a string".to_string())
        })?;
        let vector_index = args[1].as_str().ok_or_else(|| {
            DbError::ExecutionError("GRAPH_RAG: vector_index must be a string".to_string())
        })?;
        let edge_name = args[2].as_str().ok_or_else(|| {
            DbError::ExecutionError("GRAPH_RAG: edge_collection must be a string".to_string())
        })?;
        let query_vector = Self::extract_vector_arg(&args[3], "GRAPH_RAG: query_vector")?;
        let opts_val = args.get(4);
        let opts = parse_expand_options(opts_val, "GRAPH_RAG")?;

        let get_opt_str = |k: &str| opts_val.and_then(|o| o.get(k)).and_then(|v| v.as_str());
        let get_opt_u64 = |k: &str| opts_val.and_then(|o| o.get(k)).and_then(|v| v.as_u64());
        let seed_mode = get_opt_str("seed_mode").unwrap_or("vector");
        let seed_limit = get_opt_u64("seed_limit").unwrap_or(10) as usize;
        let ef = get_opt_u64("ef").map(|v| v as usize);

        let coll = self.get_collection(seed_collection)?;
        let mut seeds: Vec<(String, f64)> = Vec::new();
        match seed_mode {
            "hybrid" => {
                let fulltext_field = get_opt_str("fulltext_field").ok_or_else(|| {
                    DbError::ExecutionError(
                        "GRAPH_RAG: seed_mode 'hybrid' requires options.fulltext_field".to_string(),
                    )
                })?;
                let text_query = get_opt_str("text_query").ok_or_else(|| {
                    DbError::ExecutionError(
                        "GRAPH_RAG: seed_mode 'hybrid' requires options.text_query".to_string(),
                    )
                })?;
                let hopts = crate::storage::HybridSearchOptions {
                    limit: seed_limit,
                    ..Default::default()
                };
                for hit in coll.hybrid_search(
                    vector_index,
                    fulltext_field,
                    &query_vector,
                    text_query,
                    &hopts,
                )? {
                    let id = hit
                        .document
                        .as_ref()
                        .and_then(|d| d.get("_id"))
                        .and_then(|v| v.as_str())
                        .map(String::from)
                        .unwrap_or_else(|| format!("{}/{}", seed_collection, hit.doc_key));
                    seeds.push((id, hit.score as f64));
                }
            }
            "vector" => {
                for r in coll.vector_search(vector_index, &query_vector, seed_limit, ef)? {
                    seeds.push((format!("{}/{}", seed_collection, r.doc_key), r.score as f64));
                }
            }
            other => {
                return Err(DbError::ExecutionError(format!(
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
            return Err(DbError::ExecutionError(
                "COMMUNITY_SEARCH requires 1-2 arguments: query_text, [options]".to_string(),
            ));
        }
        let query_text = args[0].as_str().ok_or_else(|| {
            DbError::ExecutionError("COMMUNITY_SEARCH: query_text must be a string".to_string())
        })?;
        let opts = args.get(1);
        let get_str = |k: &str| opts.and_then(|o| o.get(k)).and_then(|v| v.as_str());
        let limit = opts
            .and_then(|o| o.get("limit"))
            .and_then(|v| v.as_u64())
            .unwrap_or(5) as usize;

        // Resolve the run to filter on: explicit run_id, else the latest run
        // recorded for the edge collection in _graph_runs.
        let run_id: Option<String> = match get_str("run_id") {
            Some(r) => Some(r.to_string()),
            None => get_str("edge_collection").and_then(|ec| {
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

        // Score community summaries by term overlap with the query.
        // `_community_summaries` is a small derived collection (bounded by
        // max_communities), so a scan is cheap and avoids coupling to
        // fulltext-index maintenance.
        let query_terms = tokenize_lower(query_text);
        let mut scored: Vec<(f64, Value)> = Vec::new();
        for doc in summaries.scan(None) {
            let v = doc.to_value();
            if let Some(ref rid) = run_id {
                if v.get("run_id").and_then(|x| x.as_str()) != Some(rid.as_str()) {
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

        let mut out = Vec::with_capacity(scored.len());
        for hit in scored {
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
}

/// Pure scoring/dedup: fold seed similarities and per-seed BFS reaches into a
/// ranked hit list. `contrib = seed_sim * decay^hop`, combined per vertex by
/// `max` (default) or `sum`, keeping the minimum hop and the seed responsible
/// for the strongest single contribution (`via`).
fn graph_aggregate(
    seeds: &[(String, f64)],
    reached: &[Vec<Reached>],
    opts: &ExpandOptions,
) -> Vec<ScoredHit> {
    let mut map: HashMap<String, ScoredHit> = HashMap::new();

    let add = |map: &mut HashMap<String, ScoredHit>,
               id: &str,
               contrib: f64,
               hops: usize,
               via: Option<String>| {
        let e = map.entry(id.to_string()).or_insert_with(|| ScoredHit {
            id: id.to_string(),
            score: 0.0,
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
    hits.truncate(opts.limit);
    hits
}

/// Split text into lowercase alphanumeric tokens of length >= 3.
fn tokenize_lower(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_lowercase())
        .collect()
}

fn parse_direction(s: &str, fname: &str) -> DbResult<EdgeDirection> {
    match s.to_lowercase().as_str() {
        "outbound" | "out" => Ok(EdgeDirection::Outbound),
        "inbound" | "in" => Ok(EdgeDirection::Inbound),
        "any" | "both" => Ok(EdgeDirection::Any),
        other => Err(DbError::ExecutionError(format!(
            "{}: unknown direction '{}' (expected outbound|inbound|any)",
            fname, other
        ))),
    }
}

fn parse_expand_options(opts_val: Option<&Value>, fname: &str) -> DbResult<ExpandOptions> {
    let mut o = ExpandOptions::default();
    if let Some(Value::Object(m)) = opts_val {
        if let Some(v) = m.get("hops").and_then(|v| v.as_u64()) {
            o.hops = v as usize;
        }
        if let Some(v) = m.get("direction").and_then(|v| v.as_str()) {
            o.direction = parse_direction(v, fname)?;
        }
        if let Some(v) = m.get("decay").and_then(|v| v.as_f64()) {
            o.decay = v.clamp(f64::EPSILON, 1.0);
        }
        if let Some(v) = m.get("combine").and_then(|v| v.as_str()) {
            o.combine = match v.to_lowercase().as_str() {
                "max" => Combine::Max,
                "sum" => Combine::Sum,
                other => {
                    return Err(DbError::ExecutionError(format!(
                        "{}: unknown combine '{}' (expected max|sum)",
                        fname, other
                    )))
                }
            };
        }
        if let Some(v) = m.get("include_seeds").and_then(|v| v.as_bool()) {
            o.include_seeds = v;
        }
        if let Some(v) = m.get("max_frontier").and_then(|v| v.as_u64()) {
            o.max_frontier = v as usize;
        }
        if let Some(v) = m.get("limit").and_then(|v| v.as_u64()) {
            o.limit = v as usize;
        }
    }
    Ok(o)
}

/// Parse the `seeds` argument of `NEIGHBORS`: an array of `"coll/key"` strings
/// or `{id|_id, score?}` objects. Bare keys are qualified with
/// `options.seed_collection` when provided.
fn parse_seeds(v: &Value, opts_val: Option<&Value>, fname: &str) -> DbResult<Vec<(String, f64)>> {
    let seed_coll = opts_val
        .and_then(|o| o.get("seed_collection"))
        .and_then(|v| v.as_str());
    let arr = v
        .as_array()
        .ok_or_else(|| DbError::ExecutionError(format!("{}: seeds must be an array", fname)))?;
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
                        DbError::ExecutionError(format!(
                            "{}: seed object requires an 'id' or '_id' string",
                            fname
                        ))
                    })?
                    .to_string();
                let score = m.get("score").and_then(|v| v.as_f64()).unwrap_or(1.0);
                (id, score)
            }
            _ => {
                return Err(DbError::ExecutionError(format!(
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
