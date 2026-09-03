//! Shared edge-expansion engine for graph traversal and Graph RAG.
//!
//! `EdgeExpander` encapsulates the index-probe / adjacency-fallback / neighbor
//! derivation logic. It is used both by the `FOR v,e IN .. OUTBOUND` traversal
//! clause (`clauses.rs`) and by the `NEIGHBORS` / `GRAPH_RAG` SDBQL functions
//! (`graph_rag.rs`), so the two share exactly one implementation of the subtle
//! bits:
//!
//! - `index_lookup_eq(field, ..)` returns `Some(vec![])` when an index exists
//!   but has no match, and `None` **only** when the field is unindexed. The
//!   in-memory adjacency map is therefore built (and consulted) exactly when a
//!   required field is unindexed — never when it merely has no matching edge.
//! - When `auto_index` is set **and the collection is an edge collection**, a
//!   missing `_from`/`_to` index is created lazily (persistent, non-unique).
//!   `create_index` backfills existing docs with one scan — the same cost as
//!   building the throwaway adjacency map it replaces — but it persists, so
//!   every later traversal / GRAPH_RAG call is indexed.
//!
//!   The edge-type gate matters: `NEIGHBORS`/`GRAPH_RAG` take a collection
//!   *name* from the query, and SDBQL function calls carry only `Read`
//!   permission. Without the gate, `NEIGHBORS("any_collection", …)` would
//!   persist `_from`/`_to` indexes on an unrelated document collection. Edge
//!   collections already get both indexes at creation time (`Database::
//!   create_collection`), so this path only ever fires for edge collections
//!   predating that behavior.

use crate::sdbql::ast::EdgeDirection;
use crate::storage::{Collection, Document};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

/// A vertex reached by BFS expansion, with its minimum hop distance from the
/// seed. Seeds themselves are not emitted.
pub(crate) struct Reached {
    pub id: String,
    pub depth: usize,
}

pub(crate) struct EdgeExpander<'e> {
    edge: &'e Collection,
    direction: EdgeDirection,
    adjacency: Option<HashMap<String, Vec<Document>>>,
}

impl<'e> EdgeExpander<'e> {
    /// Build an expander over `edge`. Probes for `_from`/`_to` indexes; when
    /// `auto_index` is set, creates any missing one. Falls back to a single-scan
    /// in-memory adjacency map only for directions whose required field stays
    /// unindexed.
    ///
    /// `max_edges` bounds that fallback: an unindexed edge collection larger
    /// than this is an error rather than a whole-collection load.
    pub(crate) fn new(
        edge: &'e Collection,
        direction: EdgeDirection,
        auto_index: bool,
        max_edges: usize,
    ) -> crate::error::DbResult<Self> {
        let probe = Value::String(String::new());
        let want_from = matches!(direction, EdgeDirection::Outbound | EdgeDirection::Any);
        let want_to = matches!(direction, EdgeDirection::Inbound | EdgeDirection::Any);

        let mut has_from_index = edge.index_lookup_eq("_from", &probe).is_some();
        let mut has_to_index = edge.index_lookup_eq("_to", &probe).is_some();

        if auto_index && edge.get_type() == "edge" {
            if want_from && !has_from_index {
                let _ = edge.create_index(
                    "_edge_from_idx".to_string(),
                    vec!["_from".to_string()],
                    crate::storage::IndexType::Persistent,
                    false,
                );
                has_from_index = edge.index_lookup_eq("_from", &probe).is_some();
            }
            if want_to && !has_to_index {
                let _ = edge.create_index(
                    "_edge_to_idx".to_string(),
                    vec!["_to".to_string()],
                    crate::storage::IndexType::Persistent,
                    false,
                );
                has_to_index = edge.index_lookup_eq("_to", &probe).is_some();
            }
        }

        let needs_adjacency = match direction {
            EdgeDirection::Outbound => !has_from_index,
            EdgeDirection::Inbound => !has_to_index,
            EdgeDirection::Any => !(has_from_index && has_to_index),
        };

        let adjacency = if needs_adjacency {
            let mut map: HashMap<String, Vec<Document>> = HashMap::new();
            let docs = edge.scan(Some(max_edges.saturating_add(1)));
            if docs.len() > max_edges {
                return Err(crate::error::DbError::ExecutionError(format!(
                    "Edge collection '{}' has more than {} edges and no _from/_to index; \
                     create one, or raise SOLIDB_MAX_INTERMEDIATE_ROWS",
                    edge.name, max_edges
                )));
            }
            for doc in docs {
                let from = match doc.get("_from") {
                    Some(Value::String(s)) => Some(s.clone()),
                    _ => None,
                };
                let to = match doc.get("_to") {
                    Some(Value::String(s)) => Some(s.clone()),
                    _ => None,
                };
                if want_from {
                    if let Some(ref f) = from {
                        map.entry(f.clone()).or_default().push(doc.clone());
                    }
                }
                if want_to {
                    if let Some(ref t) = to {
                        // Self-loop already inserted under _from
                        if !(want_from && from.as_deref() == Some(t.as_str())) {
                            map.entry(t.clone()).or_default().push(doc.clone());
                        }
                    }
                }
            }
            Some(map)
        } else {
            None
        };

        Ok(Self {
            edge,
            direction,
            adjacency,
        })
    }

    fn adjacency_edges(&self, key: &str) -> Vec<Document> {
        self.adjacency
            .as_ref()
            .and_then(|m| m.get(key).cloned())
            .unwrap_or_default()
    }

    /// Edges incident to `current_id` in the configured direction.
    pub(crate) fn edges_for(&self, current_id: &str) -> Vec<Document> {
        let current_value = Value::String(current_id.to_string());
        match self.direction {
            EdgeDirection::Outbound => self
                .edge
                .index_lookup_eq("_from", &current_value)
                .unwrap_or_else(|| self.adjacency_edges(current_id)),
            EdgeDirection::Inbound => self
                .edge
                .index_lookup_eq("_to", &current_value)
                .unwrap_or_else(|| self.adjacency_edges(current_id)),
            EdgeDirection::Any => match (
                self.edge.index_lookup_eq("_from", &current_value),
                self.edge.index_lookup_eq("_to", &current_value),
            ) {
                (Some(from_edges), Some(to_edges)) => {
                    let mut seen: HashSet<String> = HashSet::new();
                    from_edges
                        .into_iter()
                        .chain(to_edges)
                        .filter(|e| seen.insert(e.key.clone()))
                        .collect()
                }
                _ => self.adjacency_edges(current_id),
            },
        }
    }

    /// The vertex on the far side of an edge with endpoints `from`/`to`,
    /// relative to `current_id`.
    fn resolve_next(
        &self,
        from: Option<&str>,
        to: Option<&str>,
        current_id: &str,
    ) -> Option<String> {
        match self.direction {
            EdgeDirection::Outbound => to.map(|s| s.to_string()),
            EdgeDirection::Inbound => from.map(|s| s.to_string()),
            EdgeDirection::Any => {
                if from == Some(current_id) {
                    to.map(|s| s.to_string())
                } else if to == Some(current_id) {
                    from.map(|s| s.to_string())
                } else {
                    None
                }
            }
        }
    }

    /// The vertex on the far side of `edge_val` relative to `current_id`.
    pub(crate) fn next_id(&self, edge_val: &Value, current_id: &str) -> Option<String> {
        self.resolve_next(
            edge_val.get("_from").and_then(|v| v.as_str()),
            edge_val.get("_to").and_then(|v| v.as_str()),
            current_id,
        )
    }

    /// Same, straight off the stored document. `Document::get` clones a single
    /// field, where `to_value()` would rebuild the whole JSON object plus five
    /// metadata fields — wasteful in the BFS inner loop, which only ever reads
    /// `_from` and `_to`.
    fn next_id_from_doc(&self, edge_doc: &Document, current_id: &str) -> Option<String> {
        let from = edge_doc.get("_from");
        let to = edge_doc.get("_to");
        self.resolve_next(
            from.as_ref().and_then(|v| v.as_str()),
            to.as_ref().and_then(|v| v.as_str()),
            current_id,
        )
    }

    /// Single-source BFS from `seed`, emitting each reached vertex once at its
    /// minimum hop distance (the seed is excluded). `max_frontier` bounds the
    /// number of distinct vertices visited to keep high-fan-out graphs safe.
    pub(crate) fn bfs_from(
        &self,
        seed: &str,
        max_hops: usize,
        max_frontier: usize,
    ) -> Vec<Reached> {
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        let mut out: Vec<Reached> = Vec::new();

        visited.insert(seed.to_string());
        queue.push_back((seed.to_string(), 0));

        while let Some((current_id, depth)) = queue.pop_front() {
            if depth > 0 {
                out.push(Reached {
                    id: current_id.clone(),
                    depth,
                });
            }
            if depth >= max_hops {
                continue;
            }
            for edge_doc in self.edges_for(&current_id) {
                if let Some(next) = self.next_id_from_doc(&edge_doc, &current_id) {
                    if visited.len() >= max_frontier {
                        break;
                    }
                    if visited.insert(next.clone()) {
                        queue.push_back((next, depth + 1));
                    }
                }
            }
        }
        out
    }
}
