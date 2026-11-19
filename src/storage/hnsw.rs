//! The HNSW graph of a vector index (card #983).
//!
//! Nodes are numbered: a node is a `u32`, its vector a slice of one
//! contiguous arena, its links lists of numbers. The document keys are kept
//! beside, and only searched results are turned back into keys. Through
//! 2.14.0 the graph held `HashMap<String, …>` everywhere: every distance
//! hashed a key and every link was a cloned string, and building an index of
//! 300,000 vectors took 652 s where pgvector took 24.
//!
//! Links are chosen with the HNSW paper's heuristic (Malkov & Yashunin,
//! algorithm 4): a candidate is kept only if it is nearer to the node than to
//! the neighbours already kept, the others filling what is left. Taking the
//! M nearest, as before 2.14.0's fix, cut clustered data into islands
//! (card #979).
//!
//! A whole index is built on several threads (`Graph::build`): each node's
//! links behind a lock of their own, as hnswlib does.
//!
//! A quantized graph (card #996) keeps one byte a dimension instead of the
//! f32 arena ([`Sq8`]): the walk computes its distances on the codes, and
//! the caller re-scores what it keeps against the exact vectors.

use crate::storage::index::VectorMetric;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

/// A node and its distance to the point searched for, nearest first in a
/// `BinaryHeap` (a min-heap by distance).
#[derive(Clone, Copy, PartialEq)]
struct Near {
    d: f32,
    id: u32,
}

impl Eq for Near {}

impl Ord for Near {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .d
            .partial_cmp(&self.d)
            .unwrap_or(Ordering::Equal)
            .then(other.id.cmp(&self.id))
    }
}

impl PartialOrd for Near {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The same, furthest first (a max-heap by distance).
#[derive(Clone, Copy, PartialEq)]
struct Far {
    d: f32,
    id: u32,
}

impl Eq for Far {}

impl Ord for Far {
    fn cmp(&self, other: &Self) -> Ordering {
        self.d
            .partial_cmp(&other.d)
            .unwrap_or(Ordering::Equal)
            .then(self.id.cmp(&other.id))
    }
}

impl PartialOrd for Far {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

thread_local! {
    /// Nodes visited by the current search on this thread: a node was
    /// visited when its entry equals the epoch. No set is allocated per
    /// search.
    static VISITED: RefCell<(Vec<u32>, u32)> = const { RefCell::new((Vec::new(), 0)) };
}

/// Run `f` with a "visited" test-and-set for a graph of `n` nodes.
fn with_visited<T>(n: usize, f: impl FnOnce(&mut dyn FnMut(u32) -> bool) -> T) -> T {
    VISITED.with(|cell| {
        let mut guard = cell.borrow_mut();
        let (marks, epoch) = &mut *guard;
        if marks.len() < n {
            marks.resize(n, 0);
        }
        *epoch = epoch.wrapping_add(1);
        if *epoch == 0 {
            marks.iter_mut().for_each(|m| *m = 0);
            *epoch = 1;
        }
        let e = *epoch;
        let mut first_visit = |id: u32| {
            let slot = &mut marks[id as usize];
            if *slot == e {
                false
            } else {
                *slot = e;
                true
            }
        };
        f(&mut first_visit)
    })
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    // Four accumulators, so the compiler can keep several lanes busy.
    let (a4, ar) = a.as_chunks::<4>();
    let (b4, br) = b.as_chunks::<4>();
    let mut acc = [0.0f32; 4];
    for (x, y) in a4.iter().zip(b4) {
        for i in 0..4 {
            acc[i] += x[i] * y[i];
        }
    }
    let mut sum = acc[0] + acc[1] + acc[2] + acc[3];
    for (x, y) in ar.iter().zip(br) {
        sum += x * y;
    }
    sum
}

fn squared_l2(a: &[f32], b: &[f32]) -> f32 {
    let (a4, ar) = a.as_chunks::<4>();
    let (b4, br) = b.as_chunks::<4>();
    let mut acc = [0.0f32; 4];
    for (x, y) in a4.iter().zip(b4) {
        for i in 0..4 {
            let d = x[i] - y[i];
            acc[i] += d * d;
        }
    }
    let mut sum = acc[0] + acc[1] + acc[2] + acc[3];
    for (x, y) in ar.iter().zip(br) {
        let d = x - y;
        sum += d * d;
    }
    sum
}

/// The vector as the graph keeps it: normalised for cosine, so that the
/// distance is `1 - dot`; as given otherwise.
fn prepared(v: &[f32], metric: VectorMetric) -> Vec<f32> {
    match metric {
        VectorMetric::Cosine => {
            let n = dot(v, v).sqrt();
            if n > 1e-10 {
                v.iter().map(|x| x / n).collect()
            } else {
                vec![0.0; v.len()]
            }
        }
        _ => v.to_vec(),
    }
}

/// Distance between two prepared vectors; lower is nearer. The same values
/// as `1 - cosine_similarity`, `euclidean_distance` and `-dot_product`.
fn distance(a: &[f32], b: &[f32], metric: VectorMetric) -> f32 {
    match metric {
        VectorMetric::Cosine => 1.0 - dot(a, b),
        VectorMetric::Euclidean => squared_l2(a, b).sqrt(),
        VectorMetric::DotProduct => -dot(a, b),
    }
}

/// A graph's vectors at one byte a dimension: `value = min + code * step`,
/// per dimension, over the prepared vectors (normalised for cosine). The
/// bounds are those of the vectors when the graph was quantized; a later
/// vector outside them is clamped to them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sq8 {
    pub min: Vec<f32>,
    pub step: Vec<f32>,
    /// `dim` codes a node, in node order (removed nodes included).
    pub codes: Vec<u8>,
}

impl Sq8 {
    /// Bounds over `rows` (`dim` floats each), with no codes yet.
    fn bounds_of(rows: &[f32], dim: usize) -> Self {
        let mut min = vec![f32::MAX; dim];
        let mut max = vec![f32::MIN; dim];
        for row in rows.chunks_exact(dim) {
            for (i, &v) in row.iter().enumerate() {
                min[i] = min[i].min(v);
                max[i] = max[i].max(v);
            }
        }
        if rows.is_empty() {
            min = vec![0.0; dim];
            max = vec![0.0; dim];
        }
        let step = min
            .iter()
            .zip(&max)
            .map(|(lo, hi)| ((hi - lo) / 255.0).max(0.0))
            .collect();
        Sq8 {
            min,
            step,
            codes: Vec::new(),
        }
    }

    fn push(&mut self, v: &[f32]) {
        for (i, &x) in v.iter().enumerate() {
            let code = if self.step[i] > 0.0 {
                ((x - self.min[i]) / self.step[i]).round().clamp(0.0, 255.0) as u8
            } else {
                0
            };
            self.codes.push(code);
        }
    }

    fn codes_of(&self, id: u32, dim: usize) -> &[u8] {
        let at = id as usize * dim;
        &self.codes[at..at + dim]
    }

    fn decode(&self, id: u32, dim: usize) -> Vec<f32> {
        self.codes_of(id, dim)
            .iter()
            .zip(self.min.iter().zip(&self.step))
            .map(|(&c, (lo, st))| lo + c as f32 * st)
            .collect()
    }

    /// `distance(q, decode(id))` without decoding into a buffer.
    fn distance(&self, q: &[f32], id: u32, dim: usize, metric: VectorMetric) -> f32 {
        let codes = self.codes_of(id, dim);
        let values = codes
            .iter()
            .zip(self.min.iter().zip(&self.step))
            .map(|(&c, (lo, st))| lo + c as f32 * st);
        match metric {
            VectorMetric::Cosine => 1.0 - q.iter().zip(values).map(|(a, b)| a * b).sum::<f32>(),
            VectorMetric::DotProduct => -q.iter().zip(values).map(|(a, b)| a * b).sum::<f32>(),
            VectorMetric::Euclidean => q
                .iter()
                .zip(values)
                .map(|(a, b)| (a - b) * (a - b))
                .sum::<f32>()
                .sqrt(),
        }
    }
}

/// The graph as 2.14.0 and earlier stored it, kept to read those indexes:
/// the field order is the old `HnswGraph`'s, which bincode depends on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyGraph {
    nodes: HashMap<String, LegacyNode>,
    entry_point: Option<String>,
    max_level: usize,
    m: usize,
    m0: usize,
    ef_construction: usize,
    level_mult: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyNode {
    neighbors: Vec<Vec<String>>,
    level: usize,
}

/// An HNSW graph over numbered nodes.
#[derive(Clone, Serialize, Deserialize)]
pub struct Graph {
    dim: usize,
    metric: VectorMetric,
    m: usize,
    m0: usize,
    ef_construction: usize,
    level_mult: f64,
    /// Node -> document key; empty for a removed node.
    keys: Vec<String>,
    /// Node -> level -> neighbours.
    links: Vec<Vec<Vec<u32>>>,
    removed: Vec<bool>,
    entry: Option<u32>,
    max_level: usize,
    /// Rebuilt by `restore` after a load.
    #[serde(skip)]
    ids: HashMap<String, u32>,
    /// Node -> prepared vector, `dim` floats each. Rebuilt by `restore`.
    /// Empty once the graph is quantized: `sq` holds the vectors then.
    #[serde(skip)]
    arena: Vec<f32>,
    /// The vectors at one byte a dimension, when the graph is quantized.
    #[serde(skip)]
    sq: Option<Sq8>,
    #[serde(skip)]
    live: usize,
}

impl Graph {
    pub fn new(dim: usize, metric: VectorMetric, m: usize, ef_construction: usize) -> Self {
        let m = m.max(2);
        Graph {
            dim,
            metric,
            m,
            m0: m * 2,
            ef_construction,
            level_mult: 1.0 / (m as f64).ln(),
            keys: Vec::new(),
            links: Vec::new(),
            removed: Vec::new(),
            entry: None,
            max_level: 0,
            ids: HashMap::new(),
            arena: Vec::new(),
            sq: None,
            live: 0,
        }
    }

    /// Nodes in the graph, removed ones apart.
    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    /// The vector kept for `key` (normalised for cosine; approximate once
    /// the graph is quantized), if it is in the graph.
    pub fn vector_of(&self, key: &str) -> Option<Cow<'_, [f32]>> {
        self.ids.get(key).map(|&id| self.vector(id))
    }

    pub fn contains(&self, key: &str) -> bool {
        self.ids.contains_key(key)
    }

    /// Every node's key and vector, removed ones apart.
    pub fn live_vectors(&self) -> impl Iterator<Item = (&str, Cow<'_, [f32]>)> + '_ {
        self.ids
            .iter()
            .map(|(key, &id)| (key.as_str(), self.vector(id)))
    }

    /// The vectors, `dim` floats a node in node order (removed ones
    /// included), as an index saves them. Empty when the graph is
    /// quantized: [`Self::sq`] holds them.
    pub fn arena(&self) -> &[f32] {
        &self.arena
    }

    /// The vectors at one byte a dimension, when the graph is quantized.
    pub fn sq(&self) -> Option<&Sq8> {
        self.sq.as_ref()
    }

    pub fn is_quantized(&self) -> bool {
        self.sq.is_some()
    }

    /// Keep the vectors at one byte a dimension and drop the f32 arena: a
    /// quarter of the memory. Bounds are taken over the live nodes.
    pub fn quantize(&mut self) {
        if self.sq.is_some() {
            return;
        }
        let dim = self.dim;
        let live: Vec<f32> = (0..self.keys.len())
            .filter(|&id| !self.removed[id])
            .flat_map(|id| self.arena[id * dim..(id + 1) * dim].iter().copied())
            .collect();
        let mut sq = Sq8::bounds_of(&live, dim);
        drop(live);
        sq.codes.reserve_exact(self.arena.len());
        for row in self.arena.chunks_exact(dim) {
            sq.push(row);
        }
        self.sq = Some(sq);
        self.arena = Vec::new();
    }

    /// Back to f32 vectors, decoded from the codes: they keep the
    /// quantization's error until the index is rebuilt from its documents.
    pub fn dequantize(&mut self) {
        let Some(sq) = self.sq.take() else {
            return;
        };
        let mut arena = Vec::with_capacity(self.keys.len() * self.dim);
        for id in 0..self.keys.len() as u32 {
            arena.extend(sq.decode(id, self.dim));
        }
        self.arena = arena;
    }
    /// After a load: the vectors as [`Self::arena`] gave them. `false`
    /// when the arena does not match the nodes (nothing is restored).
    pub fn restore_arena(&mut self, arena: Vec<f32>) -> bool {
        if arena.len() != self.keys.len() * self.dim || self.removed.len() != self.keys.len() {
            return false;
        }
        self.ids.clear();
        self.live = 0;
        self.arena = arena;
        self.sq = None;
        self.renumber();
        true
    }

    /// After a load: the vectors as [`Self::sq`] gave them. `false` when
    /// they do not match the nodes (nothing is restored).
    pub fn restore_sq(&mut self, sq: Sq8) -> bool {
        if sq.codes.len() != self.keys.len() * self.dim
            || sq.min.len() != self.dim
            || sq.step.len() != self.dim
            || self.removed.len() != self.keys.len()
        {
            return false;
        }
        self.arena = Vec::new();
        self.sq = Some(sq);
        self.renumber();
        true
    }

    /// The keys' numbers and the live count, from `keys` and `removed`.
    fn renumber(&mut self) {
        self.ids.clear();
        self.live = 0;
        for (id, key) in self.keys.iter().enumerate() {
            if !self.removed[id] {
                self.ids.insert(key.clone(), id as u32);
                self.live += 1;
            }
        }
    }

    /// Node `id`'s vector: borrowed from the arena, or decoded from its
    /// codes when the graph is quantized.
    fn vector(&self, id: u32) -> Cow<'_, [f32]> {
        match &self.sq {
            Some(sq) => Cow::Owned(sq.decode(id, self.dim)),
            None => {
                let at = id as usize * self.dim;
                Cow::Borrowed(&self.arena[at..at + self.dim])
            }
        }
    }

    fn dist_to(&self, q: &[f32], id: u32) -> f32 {
        match &self.sq {
            Some(sq) => sq.distance(q, id, self.dim, self.metric),
            None => {
                let at = id as usize * self.dim;
                distance(q, &self.arena[at..at + self.dim], self.metric)
            }
        }
    }

    fn random_level(&self) -> usize {
        let r: f64 = rand::thread_rng().gen();
        ((-r.max(f64::MIN_POSITIVE).ln() * self.level_mult).floor() as usize).min(16)
    }

    /// The `ef` nodes nearest to `q` found from `entry` at `level`, nearest
    /// first. `links_of` reads a node's links at that level.
    fn search_layer(
        &self,
        q: &[f32],
        entry: &[u32],
        ef: usize,
        level: usize,
        links_of: &dyn Fn(u32, usize, &mut Vec<u32>),
    ) -> Vec<Near> {
        with_visited(self.keys.len(), |first_visit| {
            let mut candidates: BinaryHeap<Near> = BinaryHeap::new();
            let mut result: BinaryHeap<Far> = BinaryHeap::new();
            for &e in entry {
                if first_visit(e) && !self.removed[e as usize] {
                    let d = self.dist_to(q, e);
                    candidates.push(Near { d, id: e });
                    result.push(Far { d, id: e });
                }
            }
            let mut neighbours = Vec::new();
            while let Some(current) = candidates.pop() {
                let furthest = result.peek().map_or(f32::MAX, |f| f.d);
                if current.d > furthest && result.len() >= ef {
                    break;
                }
                links_of(current.id, level, &mut neighbours);
                for &n in &neighbours {
                    if !first_visit(n) || self.removed[n as usize] {
                        continue;
                    }
                    let d = self.dist_to(q, n);
                    let furthest = result.peek().map_or(f32::MAX, |f| f.d);
                    if result.len() < ef || d < furthest {
                        candidates.push(Near { d, id: n });
                        result.push(Far { d, id: n });
                        if result.len() > ef {
                            result.pop();
                        }
                    }
                }
            }
            let mut out: Vec<Near> = result
                .into_iter()
                .map(|f| Near { d: f.d, id: f.id })
                .collect();
            out.sort_by(|a, b| {
                a.d.partial_cmp(&b.d)
                    .unwrap_or(Ordering::Equal)
                    .then(a.id.cmp(&b.id))
            });
            out
        })
    }

    /// Up to `m` of `candidates` (nearest first, distances to `base`): the
    /// paper's heuristic, then the passed-over ones to fill.
    fn select(&self, candidates: &[Near], m: usize) -> Vec<u32> {
        let mut selected: Vec<Near> = Vec::with_capacity(m);
        let mut passed_over: Vec<u32> = Vec::new();
        for c in candidates {
            if selected.len() >= m {
                break;
            }
            let v = self.vector(c.id);
            if selected.iter().all(|s| self.dist_to(&v, s.id) > c.d) {
                selected.push(*c);
            } else {
                passed_over.push(c.id);
            }
        }
        let mut out: Vec<u32> = selected.into_iter().map(|s| s.id).collect();
        for id in passed_over {
            if out.len() >= m {
                break;
            }
            out.push(id);
        }
        out
    }

    /// Give `key` a node with `vector`, without linking it.
    fn add_node(&mut self, key: &str, vector: &[f32]) -> (u32, usize) {
        let id = self.keys.len() as u32;
        let level = self.random_level();
        self.keys.push(key.to_string());
        self.links.push(vec![Vec::new(); level + 1]);
        self.removed.push(false);
        let v = prepared(vector, self.metric);
        match self.sq.as_mut() {
            Some(sq) => sq.push(&v),
            None => self.arena.extend_from_slice(&v),
        }
        self.ids.insert(key.to_string(), id);
        self.live += 1;
        (id, level)
    }

    /// Insert `key` (replacing the node it had).
    pub fn insert(&mut self, key: &str, vector: &[f32]) {
        if vector.len() != self.dim {
            return;
        }
        if self.ids.contains_key(key) {
            self.remove(key);
        }
        let (id, level) = self.add_node(key, vector);
        let Some(entry) = self.entry else {
            self.entry = Some(id);
            self.max_level = level;
            return;
        };
        // The exact vector, not its codes: the links are chosen from it.
        let q = prepared(vector, self.metric);
        let links = std::mem::take(&mut self.links);
        let read = |n: u32, l: usize, out: &mut Vec<u32>| {
            out.clear();
            if let Some(ls) = links[n as usize].get(l) {
                out.extend_from_slice(ls);
            }
        };
        let mut ep = vec![entry];
        for l in (level + 1..=self.max_level).rev() {
            if let Some(best) = self.search_layer(&q, &ep, 1, l, &read).first() {
                ep = vec![best.id];
            }
        }
        let mut chosen: Vec<(usize, Vec<u32>)> = Vec::new();
        for l in (0..=level.min(self.max_level)).rev() {
            let found = self.search_layer(&q, &ep, self.ef_construction, l, &read);
            let m = if l == 0 { self.m0 } else { self.m };
            chosen.push((l, self.select(&found, m)));
            if !found.is_empty() {
                ep = found.iter().map(|n| n.id).collect();
            }
        }
        self.links = links;
        for (l, selected) in chosen {
            self.links[id as usize][l] = selected.clone();
            for n in selected {
                self.link_back(n, id, l);
            }
        }
        if level > self.max_level {
            self.max_level = level;
            self.entry = Some(id);
        }
    }

    /// Add `id` to `n`'s links at `l`, keeping the best `m` by the heuristic.
    fn link_back(&mut self, n: u32, id: u32, l: usize) {
        let m = if l == 0 { self.m0 } else { self.m };
        let Some(list) = self.links[n as usize].get(l) else {
            return;
        };
        if list.contains(&id) {
            return;
        }
        let mut list = list.clone();
        list.push(id);
        if list.len() > m {
            let base = self.vector(n);
            let mut cands: Vec<Near> = list
                .iter()
                .filter(|&&c| !self.removed[c as usize])
                .map(|&c| Near {
                    d: self.dist_to(&base, c),
                    id: c,
                })
                .collect();
            cands.sort_by(|a, b| {
                a.d.partial_cmp(&b.d)
                    .unwrap_or(Ordering::Equal)
                    .then(a.id.cmp(&b.id))
            });
            list = self.select(&cands, m);
        }
        self.links[n as usize][l] = list;
    }

    /// Remove `key`'s node: it is marked removed, skipped by every search,
    /// and taken out of its neighbours' links.
    pub fn remove(&mut self, key: &str) {
        let Some(id) = self.ids.remove(key) else {
            return;
        };
        self.removed[id as usize] = true;
        self.keys[id as usize].clear();
        self.live -= 1;
        let own = std::mem::take(&mut self.links[id as usize]);
        for (l, ns) in own.iter().enumerate() {
            for &n in ns {
                if let Some(list) = self.links[n as usize].get_mut(l) {
                    list.retain(|&x| x != id);
                }
            }
        }
        if self.entry == Some(id) {
            // The live node of highest level.
            let best = (0..self.keys.len() as u32)
                .filter(|&n| !self.removed[n as usize])
                .max_by_key(|&n| self.links[n as usize].len());
            self.entry = best;
            self.max_level = best.map_or(0, |n| self.links[n as usize].len().saturating_sub(1));
        }
    }

    /// The `k` nearest to `query` (keys and distances, nearest first),
    /// searching `ef` candidates.
    pub fn search(&self, query: &[f32], k: usize, ef: usize) -> Vec<(String, f32)> {
        let Some(entry) = self.entry else {
            return Vec::new();
        };
        if query.len() != self.dim {
            return Vec::new();
        }
        let q = prepared(query, self.metric);
        let read = |n: u32, l: usize, out: &mut Vec<u32>| {
            out.clear();
            if let Some(ls) = self.links[n as usize].get(l) {
                out.extend_from_slice(ls);
            }
        };
        let ep = self.descend(&q, entry, &read);
        let mut found = self.search_layer(&q, &ep, ef.max(k), 0, &read);
        found.truncate(k);
        found
            .into_iter()
            .map(|n| (self.keys[n.id as usize].clone(), n.d))
            .collect()
    }

    /// The entry point at level 0: the nearest node found going down the
    /// upper levels from `entry`.
    fn descend(&self, q: &[f32], entry: u32, read: &dyn Fn(u32, usize, &mut Vec<u32>)) -> Vec<u32> {
        let mut ep = vec![entry];
        for l in (1..=self.max_level).rev() {
            if let Some(best) = self.search_layer(q, &ep, 1, l, read).first() {
                ep = vec![best.id];
            }
        }
        ep
    }

    /// The `k` nodes nearest to `query` among those whose key `passes`,
    /// nearest first, with their distances.
    ///
    /// The filter is followed in the walk at level 0: every node is
    /// walked through, but only one that passes is kept, and the walk goes
    /// on until the `ef` nearest that pass are found and no node left to
    /// walk is nearer than the furthest of them. A selective filter makes
    /// the walk wider, not the search start again. `passes` is asked at
    /// most once per node, and only of a node that would be kept; the walk
    /// stops after asking it `max_checks` times, and answers with what it
    /// found (fewer than `k` when the filter is that rare).
    pub fn search_filtered(
        &self,
        query: &[f32],
        k: usize,
        ef: usize,
        max_checks: usize,
        passes: &mut dyn FnMut(&str) -> bool,
    ) -> Vec<(String, f32)> {
        let Some(entry) = self.entry else {
            return Vec::new();
        };
        if query.len() != self.dim || k == 0 {
            return Vec::new();
        }
        let q = prepared(query, self.metric);
        let read = |n: u32, l: usize, out: &mut Vec<u32>| {
            out.clear();
            if let Some(ls) = self.links[n as usize].get(l) {
                out.extend_from_slice(ls);
            }
        };
        let ep = self.descend(&q, entry, &read);
        let ef = ef.max(k);
        let mut found = with_visited(self.keys.len(), |first_visit| {
            let checks = std::cell::Cell::new(0usize);
            let mut check = |id: u32| {
                checks.set(checks.get() + 1);
                passes(&self.keys[id as usize])
            };
            // Every node walked through, nearest first.
            let mut candidates: BinaryHeap<Near> = BinaryHeap::new();
            // Only those that pass, furthest first.
            let mut result: BinaryHeap<Far> = BinaryHeap::new();
            for &e in &ep {
                if first_visit(e) && !self.removed[e as usize] {
                    let d = self.dist_to(&q, e);
                    candidates.push(Near { d, id: e });
                    if check(e) {
                        result.push(Far { d, id: e });
                    }
                }
            }
            let mut neighbours = Vec::new();
            while let Some(current) = candidates.pop() {
                let furthest = result.peek().map_or(f32::MAX, |f| f.d);
                if result.len() >= ef && current.d > furthest {
                    break;
                }
                read(current.id, 0, &mut neighbours);
                for &n in &neighbours {
                    if !first_visit(n) || self.removed[n as usize] {
                        continue;
                    }
                    let d = self.dist_to(&q, n);
                    let furthest = result.peek().map_or(f32::MAX, |f| f.d);
                    if result.len() < ef || d < furthest {
                        candidates.push(Near { d, id: n });
                        if check(n) {
                            result.push(Far { d, id: n });
                            if result.len() > ef {
                                result.pop();
                            }
                        }
                    }
                }
                if checks.get() >= max_checks {
                    break;
                }
            }
            result
                .into_iter()
                .map(|f| Near { d: f.d, id: f.id })
                .collect::<Vec<Near>>()
        });
        found.sort_by(|a, b| {
            a.d.partial_cmp(&b.d)
                .unwrap_or(Ordering::Equal)
                .then(a.id.cmp(&b.id))
        });
        found.truncate(k);
        found
            .into_iter()
            .map(|n| (self.keys[n.id as usize].clone(), n.d))
            .collect()
    }

    /// Build a graph of every `(key, vector)` on `threads` threads.
    pub fn build(
        dim: usize,
        metric: VectorMetric,
        m: usize,
        ef_construction: usize,
        vectors: &HashMap<String, Vec<f32>>,
        threads: usize,
    ) -> Self {
        let mut g = Graph::new(dim, metric, m, ef_construction);
        let mut levels = Vec::with_capacity(vectors.len());
        for (key, v) in vectors {
            if v.len() == dim {
                levels.push(g.add_node(key, v));
            }
        }
        let n = levels.len();
        if n == 0 {
            return g;
        }
        // The first node of highest level is the entry; it is linked as
        // the others are, against what was inserted before it.
        let (first, first_level) = levels[0];
        g.entry = Some(first);
        g.max_level = first_level;
        let locks: Vec<parking_lot::Mutex<Vec<Vec<u32>>>> = std::mem::take(&mut g.links)
            .into_iter()
            .map(parking_lot::Mutex::new)
            .collect();
        let entry = parking_lot::Mutex::new((first, first_level));
        let next = AtomicUsize::new(1);
        let shared = &g;
        let read = |id: u32, l: usize, out: &mut Vec<u32>| {
            out.clear();
            if let Some(ls) = locks[id as usize].lock().get(l) {
                out.extend_from_slice(ls);
            }
        };
        std::thread::scope(|s| {
            for _ in 0..threads.max(1) {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, AtomicOrdering::Relaxed);
                    if i >= n {
                        break;
                    }
                    let (id, level) = levels[i];
                    let (ep0, top) = *entry.lock();
                    let q = shared.vector(id);
                    let q = &q[..];
                    let mut ep = vec![ep0];
                    for l in (level + 1..=top).rev() {
                        if let Some(best) = shared.search_layer(q, &ep, 1, l, &read).first() {
                            ep = vec![best.id];
                        }
                    }
                    for l in (0..=level.min(top)).rev() {
                        let found = shared.search_layer(q, &ep, shared.ef_construction, l, &read);
                        let m = if l == 0 { shared.m0 } else { shared.m };
                        let selected = shared.select(&found, m);
                        locks[id as usize].lock()[l] = selected.clone();
                        for nb in selected {
                            let mut list = locks[nb as usize].lock();
                            let Some(at) = list.get_mut(l) else {
                                continue;
                            };
                            if at.contains(&id) {
                                continue;
                            }
                            at.push(id);
                            if at.len() > m {
                                let base = shared.vector(nb);
                                let mut cands: Vec<Near> = at
                                    .iter()
                                    .map(|&c| Near {
                                        d: shared.dist_to(&base, c),
                                        id: c,
                                    })
                                    .collect();
                                cands.sort_by(|a, b| {
                                    a.d.partial_cmp(&b.d)
                                        .unwrap_or(Ordering::Equal)
                                        .then(a.id.cmp(&b.id))
                                });
                                *at = shared.select(&cands, m);
                            }
                        }
                        if !found.is_empty() {
                            ep = found.iter().map(|x| x.id).collect();
                        }
                    }
                    if level > top {
                        let mut e = entry.lock();
                        if level > e.1 {
                            *e = (id, level);
                        }
                    }
                });
            }
        });
        let (e, top) = entry.into_inner();
        g.links = locks.into_iter().map(|l| l.into_inner()).collect();
        g.entry = Some(e);
        g.max_level = top;
        g
    }

    /// After a load: the keys' numbers and the vectors, from the index's
    /// vectors (a removed node gets zeros).
    pub fn restore(&mut self, vectors: &HashMap<String, Vec<f32>>) {
        self.ids.clear();
        self.sq = None;
        self.arena = Vec::with_capacity(self.keys.len() * self.dim);
        self.live = 0;
        for (id, key) in self.keys.iter().enumerate() {
            match vectors
                .get(key)
                .filter(|v| v.len() == self.dim && !self.removed[id])
            {
                Some(v) => {
                    self.arena.extend_from_slice(&prepared(v, self.metric));
                    self.ids.insert(key.clone(), id as u32);
                    self.live += 1;
                }
                None => {
                    self.arena.extend(std::iter::repeat_n(0.0, self.dim));
                    self.removed[id] = true;
                }
            }
        }
    }

    /// A graph of 2.14.0 or earlier, renumbered; its links are kept as
    /// they were built.
    pub fn from_legacy(
        legacy: LegacyGraph,
        dim: usize,
        metric: VectorMetric,
        vectors: &HashMap<String, Vec<f32>>,
    ) -> Self {
        let mut g = Graph::new(dim, metric, legacy.m, legacy.ef_construction);
        let mut keys: Vec<&String> = legacy.nodes.keys().collect();
        keys.sort();
        let number: HashMap<&String, u32> = keys
            .iter()
            .enumerate()
            .map(|(i, k)| (*k, i as u32))
            .collect();
        g.keys = keys.iter().map(|k| (*k).clone()).collect();
        g.removed = vec![false; g.keys.len()];
        g.links = keys
            .iter()
            .map(|k| {
                legacy.nodes[*k]
                    .neighbors
                    .iter()
                    .map(|level| {
                        level
                            .iter()
                            .filter_map(|n| number.get(n).copied())
                            .collect()
                    })
                    .collect()
            })
            .collect();
        g.entry = legacy
            .entry_point
            .as_ref()
            .and_then(|k| number.get(k).copied());
        g.max_level = legacy.max_level;
        g.restore(vectors);
        g
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(n: usize, dim: usize) -> HashMap<String, Vec<f32>> {
        let mut rng = rand::thread_rng();
        (0..n)
            .map(|i| {
                (
                    format!("d{i}"),
                    (0..dim).map(|_| rng.gen_range(-1.0..1.0)).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn a_parallel_build_finds_what_an_exhaustive_search_finds() {
        let vectors = data(3_000, 16);
        let g = Graph::build(16, VectorMetric::Cosine, 16, 200, &vectors, 4);
        assert_eq!(g.len(), 3_000);
        let q = &vectors["d42"];
        let got = g.search(q, 5, 100);
        assert_eq!(got[0].0, "d42");
        assert!(got[0].1.abs() < 1e-5);
    }

    /// A graph of 2.14.0 (keyed by strings) is renumbered with its links.
    #[test]
    fn a_legacy_graph_is_renumbered() {
        let vectors: HashMap<String, Vec<f32>> = [
            ("a", vec![1.0, 0.0]),
            ("b", vec![0.9, 0.1]),
            ("c", vec![0.0, 1.0]),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let node = |ns: &[&str]| LegacyNode {
            neighbors: vec![ns.iter().map(|s| s.to_string()).collect()],
            level: 0,
        };
        let legacy = LegacyGraph {
            nodes: [
                ("a", node(&["b", "c"])),
                ("b", node(&["a", "c"])),
                ("c", node(&["a", "b"])),
            ]
            .into_iter()
            .map(|(k, n)| (k.to_string(), n))
            .collect(),
            entry_point: Some("a".to_string()),
            max_level: 0,
            m: 16,
            m0: 32,
            ef_construction: 200,
            level_mult: 0.36,
        };
        let g = Graph::from_legacy(legacy, 2, VectorMetric::Cosine, &vectors);
        assert_eq!(g.len(), 3);
        assert_eq!(g.search(&[0.0, 1.0], 1, 10)[0].0, "c");
        assert_eq!(g.search(&[1.0, 0.05], 2, 10)[0].0, "a");
    }

    #[test]
    fn a_filtered_search_finds_the_nearest_that_pass() {
        let vectors = data(3_000, 16);
        let g = Graph::build(16, VectorMetric::Cosine, 16, 200, &vectors, 4);
        // A tenth of the keys pass.
        let passes = |key: &str| key.trim_start_matches('d').parse::<usize>().unwrap() % 10 == 3;
        let mut hits = 0;
        let mut asked = 0;
        for probe in (0..3_000).step_by(97).map(|i| format!("d{i}")) {
            let q = &vectors[&probe];
            let mut exact: Vec<(String, f32)> = vectors
                .iter()
                .filter(|(key, _)| passes(key))
                .map(|(key, v)| {
                    (
                        key.clone(),
                        distance(
                            &prepared(q, VectorMetric::Cosine),
                            &prepared(v, VectorMetric::Cosine),
                            VectorMetric::Cosine,
                        ),
                    )
                })
                .collect();
            exact.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            let mut checked = std::collections::HashSet::new();
            let got = g.search_filtered(q, 10, 40, usize::MAX, &mut |key| {
                assert!(checked.insert(key.to_string()), "{key} checked twice");
                passes(key)
            });
            assert_eq!(got.len(), 10);
            assert!(got.iter().all(|(key, _)| passes(key)));
            let want: std::collections::HashSet<&str> =
                exact[..10].iter().map(|(k, _)| k.as_str()).collect();
            hits += got
                .iter()
                .filter(|(key, _)| want.contains(key.as_str()))
                .count();
            asked += 10;
        }
        assert!(hits * 100 >= asked * 95, "recall {hits}/{asked}");
        // A filter nothing passes stops at the cap.
        let mut calls = 0;
        let none = g.search_filtered(&vectors["d0"], 10, 40, 500, &mut |_| {
            calls += 1;
            false
        });
        assert!(none.is_empty());
        assert!(calls < 600, "{calls} checks");
    }

    #[test]
    fn removal_and_restore_keep_the_graph_usable() {
        let vectors = data(500, 8);
        let mut g = Graph::new(8, VectorMetric::Euclidean, 8, 50);
        for (k, v) in &vectors {
            g.insert(k, v);
        }
        g.remove("d7");
        assert_eq!(g.len(), 499);
        assert!(g
            .search(&vectors["d7"], 10, 50)
            .iter()
            .all(|(k, _)| k != "d7"));
        let bytes = bincode::serialize(&g).unwrap();
        let mut back: Graph = bincode::deserialize(&bytes).unwrap();
        back.restore(&vectors);
        assert_eq!(back.len(), 499);
        assert_eq!(back.search(&vectors["d9"], 1, 50)[0].0, "d9");
    }
}
