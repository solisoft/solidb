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
use std::sync::Arc;

/// A vertex reached by BFS expansion, with its minimum hop distance from the
/// seed. Seeds themselves are not emitted.
pub(crate) struct Reached {
    pub id: String,
    pub depth: usize,
}

pub(crate) use crate::storage::kept as neighbourhoods;

pub(crate) struct EdgeExpander<'e> {
    edge: &'e Collection,
    direction: EdgeDirection,
    adjacency: Option<HashMap<String, Vec<Document>>>,
    /// Whether the collection has an index on `[_from, _to]` /
    /// `[_to, _from]`: a vertex's neighbours are then read from the index
    /// entries, without the edge documents.
    out_pairs: bool,
    in_pairs: bool,
    /// Whether some edge names its `_from` / `_to` with the database
    /// (`"db:coll/key"`): only then is a vertex looked up under both forms
    /// in the two-field indexes.
    from_with_db: bool,
    to_with_db: bool,
    /// The neighbourhoods kept between queries, when edges are read from an
    /// index.
    cached: Option<Arc<neighbourhoods::Neighbourhoods>>,
    /// `"<database>:"`, what a document's `_id` starts with and an edge's
    /// `_from` / `_to` usually do not (see [`local_id`]).
    db_prefix: String,
}

/// A vertex id without its database: `"shop:customers/ann"` and
/// `"customers/ann"` name one document of database `shop`, the first as its
/// `_id` reads, the second as edges are usually written. A traversal works on
/// the second form and looks edges up under both, so that a start vertex
/// given as `c._id`, or edges inserted with `_from: a._id`, are found.
pub(crate) fn local_id<'i>(id: &'i str, db_prefix: &str) -> &'i str {
    if db_prefix.is_empty() {
        return id;
    }
    id.strip_prefix(db_prefix).unwrap_or(id)
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
        let db_prefix = edge
            .name
            .split_once(':')
            .map(|(db, _)| format!("{db}:"))
            .unwrap_or_default();
        let want_from = matches!(direction, EdgeDirection::Outbound | EdgeDirection::Any);
        let want_to = matches!(direction, EdgeDirection::Inbound | EdgeDirection::Any);

        let mut out_pairs = edge.has_pair_index("_from", "_to");
        let mut in_pairs = edge.has_pair_index("_to", "_from");
        let mut has_from_index = out_pairs || edge.index_lookup_eq("_from", &probe).is_some();
        let mut has_to_index = in_pairs || edge.index_lookup_eq("_to", &probe).is_some();

        // An edge collection the walk needs an index on gets it here when it
        // has none (one created before edge collections were indexed at
        // creation): on each end, and on both ends in each order, whose
        // entries say where an edge leads without its document.
        if auto_index && edge.get_type() == "edge" {
            let create = |name: &str, fields: &[&str]| {
                let _ = edge.create_index(
                    name.to_string(),
                    fields.iter().map(|f| f.to_string()).collect(),
                    crate::storage::IndexType::Persistent,
                    false,
                );
            };
            if want_from && !has_from_index {
                create("_edge_from_idx", &["_from"]);
                has_from_index = edge.index_lookup_eq("_from", &probe).is_some();
            }
            if want_to && !has_to_index {
                create("_edge_to_idx", &["_to"]);
                has_to_index = edge.index_lookup_eq("_to", &probe).is_some();
            }
            if want_from && !out_pairs {
                create("_edge_out_idx", &["_from", "_to"]);
                out_pairs = edge.has_pair_index("_from", "_to");
            }
            if want_to && !in_pairs {
                create("_edge_in_idx", &["_to", "_from"]);
                in_pairs = edge.has_pair_index("_to", "_from");
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
                    Some(Value::String(s)) => Some(local_id(&s, &db_prefix).to_string()),
                    _ => None,
                };
                let to = match doc.get("_to") {
                    Some(Value::String(s)) => Some(local_id(&s, &db_prefix).to_string()),
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

        // Answers that come from the cache are not reads of stored keys: a
        // transaction that checks its reads holds the collection to the
        // range rule.
        let cached = match (&adjacency, &direction) {
            (Some(_), _) => None,
            (None, EdgeDirection::Outbound) => neighbourhoods::of(edge.ks.state(), 0),
            (None, EdgeDirection::Inbound) => neighbourhoods::of(edge.ks.state(), 1),
            (None, EdgeDirection::Any) => neighbourhoods::of(edge.ks.state(), 2),
        };
        if cached.is_some() {
            edge.note_range_read();
        }

        Ok(Self {
            cached,
            edge,
            direction,
            adjacency,
            from_with_db: out_pairs
                && !db_prefix.is_empty()
                && neighbourhoods::fact(edge.ks.state(), 0, || {
                    edge.index_pairs_start_with("_from", "_to", &db_prefix)
                }),
            to_with_db: in_pairs
                && !db_prefix.is_empty()
                && neighbourhoods::fact(edge.ks.state(), 1, || {
                    edge.index_pairs_start_with("_to", "_from", &db_prefix)
                }),
            out_pairs,
            in_pairs,
            db_prefix,
        })
    }

    fn adjacency_edges(&self, key: &str) -> Vec<Document> {
        self.adjacency
            .as_ref()
            .and_then(|m| m.get(key).cloned())
            .unwrap_or_default()
    }

    /// The vertex id as this traversal names it: without its database.
    pub(crate) fn local<'i>(&self, id: &'i str) -> &'i str {
        local_id(id, &self.db_prefix)
    }

    /// Edges whose `field` is `current_id`, written with or without the
    /// database. `None` when the field has no index.
    fn indexed_edges(&self, field: &str, current_id: &str) -> Option<Vec<Document>> {
        if (field == "_from" && self.out_pairs) || (field == "_to" && self.in_pairs) {
            // The documents the two-field index names, in key order as a
            // single-field index would give them.
            let mut keys: Vec<String> = self
                .pairs(field, current_id)?
                .into_iter()
                .map(|(_, key)| key)
                .collect();
            keys.sort_unstable();
            return Some(self.edge.get_many(&keys));
        }
        let mut edges = self
            .edge
            .index_lookup_eq(field, &Value::String(current_id.to_string()))?;
        if !self.db_prefix.is_empty() {
            let with_db = Value::String(format!("{}{}", self.db_prefix, current_id));
            edges.extend(
                self.edge
                    .index_lookup_eq(field, &with_db)
                    .unwrap_or_default(),
            );
        }
        Some(edges)
    }

    /// The vertices on the far side of `current_id`'s edges, in the order
    /// [`Self::edges_for`] gives the edges, reading only each edge's ends.
    /// `None` when the edges come from the adjacency map rather than an
    /// index: the caller then goes through [`Self::edges_for`].
    pub(crate) fn far_ends(&self, current_id: &str) -> Option<neighbourhoods::Ends> {
        if self.adjacency.is_some() {
            return None;
        }
        let current = self.local(current_id);
        if let Some(found) = self.cached.as_ref().and_then(|cache| cache.get(current)) {
            return Some(found);
        }
        // Noted before the index is read: see `storage::kept`.
        let seen = self.cached.as_ref().map(|cache| cache.epoch());
        let ends: neighbourhoods::Ends = self
            .read_far_ends(current)?
            .into_iter()
            .map(Arc::<str>::from)
            .collect();
        if let Some(cache) = &self.cached {
            let size = ends.iter().map(|end| end.len() + 16).sum();
            cache.put(current, &ends, size, seen.unwrap_or_default());
        }
        Some(ends)
    }

    /// The far end and the key of each of `current_id`'s edges, in key
    /// order, from the two-field indexes (and kept between queries as
    /// [`Self::far_ends`] is). `None` when the collection has none for
    /// this direction.
    pub(crate) fn far_steps(&self, current_id: &str) -> Option<neighbourhoods::Steps> {
        let (out, inn) = match self.direction {
            EdgeDirection::Outbound => (self.out_pairs, false),
            EdgeDirection::Inbound => (false, self.in_pairs),
            EdgeDirection::Any => (self.out_pairs, self.in_pairs),
        };
        if self.adjacency.is_some()
            || !(out || inn)
            || (matches!(self.direction, EdgeDirection::Any) && !(out && inn))
        {
            return None;
        }
        let current = self.local(current_id);
        let kind = match self.direction {
            EdgeDirection::Outbound => 0,
            EdgeDirection::Inbound => 1,
            EdgeDirection::Any => 2,
        };
        let kept = neighbourhoods::steps(self.edge.ks.state(), kind);
        if let Some(found) = kept.as_ref().and_then(|kept| kept.get(current)) {
            return Some(found);
        }
        let seen = kept.as_ref().map(|kept| kept.epoch());
        let sorted = |field: &str| -> Option<Vec<(String, String)>> {
            let mut pairs = self.pairs(field, current)?;
            pairs.sort_unstable_by(|a, b| a.1.cmp(&b.1));
            Some(pairs)
        };
        let mut pairs = Vec::new();
        if out {
            pairs.extend(sorted("_from")?);
        }
        if inn {
            let mut seen: HashSet<String> = pairs.iter().map(|(_, key)| key.clone()).collect();
            // A self-loop is in both lists.
            pairs.extend(
                sorted("_to")?
                    .into_iter()
                    .filter(|(_, key)| seen.insert(key.clone())),
            );
        }
        let steps: neighbourhoods::Steps = pairs
            .into_iter()
            .map(|(end, key)| (Arc::<str>::from(self.local(&end)), key.into_boxed_str()))
            .collect();
        if let Some(kept) = &kept {
            let size = steps
                .iter()
                .map(|(end, key)| end.len() + key.len() + 32)
                .sum();
            kept.put(current, &steps, size, seen.unwrap_or_default());
        }
        Some(steps)
    }

    /// The direction, as the kept maps number it.
    pub(crate) fn direction_code(&self) -> u8 {
        match self.direction {
            EdgeDirection::Outbound => 0,
            EdgeDirection::Inbound => 1,
            EdgeDirection::Any => 2,
        }
    }

    /// The collection the edges are in.
    pub(crate) fn edges(&self) -> &Collection {
        self.edge
    }

    /// [`Self::far_ends`], read from the index.
    fn read_far_ends(&self, current_id: &str) -> Option<Vec<String>> {
        let current = self.local(current_id);
        let from_pairs = |field: &str| -> Option<Vec<(String, String)>> {
            let mut pairs = self.pairs(field, current)?;
            // Key order: what a single-field index gives.
            pairs.sort_unstable_by(|a, b| a.1.cmp(&b.1));
            Some(pairs)
        };
        let local = |end: String| self.local(&end).to_string();
        match self.direction {
            EdgeDirection::Outbound if self.out_pairs => {
                return Some(
                    from_pairs("_from")?
                        .into_iter()
                        .map(|(to, _)| local(to))
                        .collect(),
                );
            }
            EdgeDirection::Inbound if self.in_pairs => {
                return Some(
                    from_pairs("_to")?
                        .into_iter()
                        .map(|(from, _)| local(from))
                        .collect(),
                );
            }
            EdgeDirection::Any if self.out_pairs && self.in_pairs => {
                let mut seen: HashSet<String> = HashSet::new();
                let mut ends = Vec::new();
                for (end, key) in from_pairs("_from")?.into_iter().chain(from_pairs("_to")?) {
                    // A self-loop is in both lists.
                    if seen.insert(key) {
                        ends.push(local(end));
                    }
                }
                return Some(ends);
            }
            _ => {}
        }
        thread_local! {
            static ENDS: [String; 3] =
                ["_key".to_string(), "_from".to_string(), "_to".to_string()];
        }
        let lookup = |field: &str| -> Option<Vec<Value>> {
            ENDS.with(|ends| {
                let mut edges = self.edge.index_lookup_eq_projected(
                    field,
                    &Value::String(current.to_string()),
                    ends,
                )?;
                if !self.db_prefix.is_empty() {
                    let with_db = Value::String(format!("{}{}", self.db_prefix, current));
                    edges.extend(
                        self.edge
                            .index_lookup_eq_projected(field, &with_db, ends)
                            .unwrap_or_default(),
                    );
                }
                Some(edges)
            })
        };
        let edges = match self.direction {
            EdgeDirection::Outbound => lookup("_from")?,
            EdgeDirection::Inbound => lookup("_to")?,
            EdgeDirection::Any => {
                let mut seen: HashSet<String> = HashSet::new();
                let mut edges = lookup("_from")?;
                edges.extend(lookup("_to")?);
                edges.retain(|e| match e.get("_key").and_then(Value::as_str) {
                    Some(key) => seen.insert(key.to_string()),
                    None => true,
                });
                edges
            }
        };
        Some(
            edges
                .iter()
                .filter_map(|e| {
                    self.resolve_next(
                        e.get("_from").and_then(Value::as_str),
                        e.get("_to").and_then(Value::as_str),
                        current,
                    )
                })
                .collect(),
        )
    }

    /// `(far end, edge key)` of the edges whose `field` is `current_id`,
    /// written with or without the database, from the two-field index.
    fn pairs(&self, field: &str, current_id: &str) -> Option<Vec<(String, String)>> {
        let other = if field == "_from" { "_to" } else { "_from" };
        let mut pairs = self.edge.index_pairs(field, other, current_id)?;
        let with_db = if field == "_from" {
            self.from_with_db
        } else {
            self.to_with_db
        };
        if with_db {
            let with_db = format!("{}{}", self.db_prefix, current_id);
            pairs.extend(
                self.edge
                    .index_pairs(field, other, &with_db)
                    .unwrap_or_default(),
            );
        }
        Some(pairs)
    }

    /// Edges incident to `current_id` in the configured direction.
    pub(crate) fn edges_for(&self, current_id: &str) -> Vec<Document> {
        let current_id = self.local(current_id);
        match self.direction {
            EdgeDirection::Outbound => self
                .indexed_edges("_from", current_id)
                .unwrap_or_else(|| self.adjacency_edges(current_id)),
            EdgeDirection::Inbound => self
                .indexed_edges("_to", current_id)
                .unwrap_or_else(|| self.adjacency_edges(current_id)),
            EdgeDirection::Any => match (
                self.indexed_edges("_from", current_id),
                self.indexed_edges("_to", current_id),
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
        let (from, to) = (from.map(|s| self.local(s)), to.map(|s| self.local(s)));
        let current_id = self.local(current_id);
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

    /// The vertex on the far side of an edge, relative to `current_id`,
    /// straight off the stored document. `Document::get` clones a single
    /// field, where `to_value()` would rebuild the whole JSON object plus five
    /// metadata fields — wasteful in the BFS inner loop, which only ever reads
    /// `_from` and `_to`.
    pub(crate) fn next_id_from_doc(&self, edge_doc: &Document, current_id: &str) -> Option<String> {
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

impl EdgeExpander<'_> {
    /// One shortest path from `start` to `end` by number of edges, searched
    /// from both ends at once: `forward` follows the edges from `start`,
    /// `backward` (the opposite direction over the same collection) from
    /// `end`, the smaller frontier first, until they meet. Only the ends of
    /// the edges are read while searching; the edges of the path found are
    /// fetched at the end.
    ///
    /// `None` when either side cannot answer from an index (the caller then
    /// searches the ordinary way), `Some(None)` when there is no path.
    pub(crate) fn shortest_unweighted(
        forward: &Self,
        backward: &Self,
        start: &str,
        end: &str,
    ) -> Option<Option<super::paths::FoundPath>> {
        use super::paths::FoundPath;
        let (start, end) = (forward.local(start), forward.local(end));
        if start == end {
            return Some(Some(FoundPath {
                vertices: vec![start.to_string()],
                edges: Vec::new(),
                weight: 0.0,
            }));
        }
        // Each side: the vertex each reached vertex was reached from, and
        // at how many edges from that side's end.
        type Reached = HashMap<String, (Option<String>, usize)>;
        let mut from_start: Reached = HashMap::from([(start.to_string(), (None, 0))]);
        let mut from_end: Reached = HashMap::from([(end.to_string(), (None, 0))]);
        let mut front_start = vec![start.to_string()];
        let mut front_end = vec![end.to_string()];
        let (mut depth_start, mut depth_end) = (0usize, 0usize);
        // The meeting vertex and the length of the path through it.
        let mut meet: Option<(String, usize)> = None;
        while meet.is_none() && !front_start.is_empty() && !front_end.is_empty() {
            let grow_start = front_start.len() <= front_end.len();
            let (side, front, seen, other, depth) = if grow_start {
                (
                    forward,
                    &mut front_start,
                    &mut from_start,
                    &from_end,
                    &mut depth_start,
                )
            } else {
                (
                    backward,
                    &mut front_end,
                    &mut from_end,
                    &from_start,
                    &mut depth_end,
                )
            };
            *depth += 1;
            let mut next = Vec::new();
            // The whole level is expanded before a meeting point is kept:
            // of the vertices of this level the other side has reached, the
            // one nearest to the other side's end (the first, among equals).
            for at in front.iter() {
                for to in side.far_ends(at)?.iter() {
                    if seen.contains_key(&**to) {
                        continue;
                    }
                    let to = to.to_string();
                    seen.insert(to.clone(), (Some(at.clone()), *depth));
                    if let Some((_, other_depth)) = other.get(&to) {
                        let length = *depth + other_depth;
                        if meet.as_ref().is_none_or(|(_, best)| length < *best) {
                            meet = Some((to.clone(), length));
                        }
                    }
                    next.push(to);
                }
            }
            *front = next;
        }
        let Some((meet, _)) = meet else {
            return Some(None);
        };
        // start … meet, then meet … end.
        let mut vertices = vec![meet.clone()];
        let mut at = &meet;
        while let Some((Some(previous), _)) = from_start.get(at) {
            vertices.push(previous.clone());
            at = previous;
        }
        vertices.reverse();
        let mut at = &meet;
        while let Some((Some(following), _)) = from_end.get(at) {
            vertices.push(following.clone());
            at = following;
        }
        // The edge of each step: the first, in key order, that leads there.
        let mut edges = Vec::with_capacity(vertices.len() - 1);
        for step in vertices.windows(2) {
            // From the index when it names the edges' keys: one document.
            if let Some(steps) = forward.far_steps(&step[0]) {
                let (_, key) = steps.iter().find(|(to, _)| **to == *step[1])?;
                edges.push(forward.edge.get(key).ok()?.to_value());
                continue;
            }
            let mut candidates = forward.edges_for(&step[0]);
            candidates.sort_by(|a, b| a.key.cmp(&b.key));
            let edge = candidates.into_iter().find(|edge| {
                forward.next_id_from_doc(edge, &step[0]).as_deref() == Some(step[1].as_str())
            })?;
            edges.push(edge.to_value());
        }
        Some(Some(FoundPath {
            weight: edges.len() as f64,
            vertices,
            edges,
        }))
    }
}

/// A path search reads a vertex's edges where a traversal does: from the
/// `_from` / `_to` index, the part of the graph it visits and no more.
impl super::paths::Neighbours for EdgeExpander<'_> {
    fn nexts(&self, current: &str) -> Vec<(String, Value)> {
        let mut edges = self.edges_for(current);
        // The order a scan of the collection gives.
        edges.sort_by(|a, b| a.key.cmp(&b.key));
        edges
            .into_iter()
            .filter_map(|doc| {
                let next = self.next_id_from_doc(&doc, current)?;
                Some((next, doc.to_value()))
            })
            .collect()
    }
}
