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

/// A graph's vectors as codes: `value = min + code * step`, per dimension,
/// over the prepared vectors (normalised for cosine).
///
/// - `bits == 8` (scalar, card #996): a byte a dimension, `min`/`step`
///   spanning the dimension's range; a later vector outside it is clamped.
/// - `bits == 1` (binary, card #999): a bit a dimension, packed eight to a
///   byte; `min = mean - s`, `step = 2s` with `s` the dimension's mean
///   absolute deviation, so a code reads as `mean ± s`.
///
/// The bounds are taken when the graph is quantized and then kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sq8 {
    pub min: Vec<f32>,
    pub step: Vec<f32>,
    /// [`Self::bytes_per_node`] bytes a node, in node order (removed nodes
    /// included).
    pub codes: Vec<u8>,
    /// Bits a dimension: 8 or 1.
    pub bits: u8,
}

impl Sq8 {
    /// Bytes of codes a node takes.
    pub fn bytes_per_node(&self, dim: usize) -> usize {
        if self.bits == 1 {
            dim.div_ceil(8)
        } else {
            dim
        }
    }

    /// Bounds over `rows` (`dim` floats each) for `bits` a dimension, with
    /// no codes yet.
    fn bounds_of<'a, I>(rows: I, dim: usize, bits: u8) -> Self
    where
        I: Iterator<Item = &'a [f32]> + Clone,
    {
        if bits == 1 {
            let n = rows.clone().count().max(1) as f32;
            let mut mean = vec![0.0f32; dim];
            for row in rows.clone() {
                for (i, &v) in row.iter().enumerate() {
                    mean[i] += v / n;
                }
            }
            let mut dev = vec![0.0f32; dim];
            for row in rows {
                for (i, &v) in row.iter().enumerate() {
                    dev[i] += (v - mean[i]).abs() / n;
                }
            }
            return Sq8 {
                min: mean.iter().zip(&dev).map(|(m, s)| m - s).collect(),
                step: dev.iter().map(|s| 2.0 * s).collect(),
                codes: Vec::new(),
                bits: 1,
            };
        }
        let mut min = vec![f32::MAX; dim];
        let mut max = vec![f32::MIN; dim];
        let mut empty = true;
        for row in rows {
            empty = false;
            for (i, &v) in row.iter().enumerate() {
                min[i] = min[i].min(v);
                max[i] = max[i].max(v);
            }
        }
        if empty {
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
            bits: 8,
        }
    }

    fn push(&mut self, v: &[f32]) {
        if self.bits == 1 {
            // Above the dimension's mean (`min + step / 2`) reads as 1.
            let mut byte = 0u8;
            for (i, &x) in v.iter().enumerate() {
                if x >= self.min[i] + self.step[i] / 2.0 {
                    byte |= 1 << (i % 8);
                }
                if i % 8 == 7 {
                    self.codes.push(byte);
                    byte = 0;
                }
            }
            if !v.len().is_multiple_of(8) {
                self.codes.push(byte);
            }
            return;
        }
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
        let per = self.bytes_per_node(dim);
        let at = id as usize * per;
        &self.codes[at..at + per]
    }

    /// Node `id`'s values, dimension by dimension.
    fn values(&self, id: u32, dim: usize) -> impl Iterator<Item = f32> + '_ {
        let codes = self.codes_of(id, dim);
        let bits = self.bits;
        (0..dim).map(move |i| {
            let code = if bits == 1 {
                (codes[i / 8] >> (i % 8)) & 1
            } else {
                codes[i]
            };
            self.min[i] + code as f32 * self.step[i]
        })
    }

    fn decode(&self, id: u32, dim: usize) -> Vec<f32> {
        self.values(id, dim).collect()
    }

    /// `distance(q, base + decode(id))` without decoding into a buffer;
    /// `base` is the node's coarse centroid when the codes are of residuals.
    fn distance(
        &self,
        q: &[f32],
        id: u32,
        dim: usize,
        metric: VectorMetric,
        base: Option<&[f32]>,
    ) -> f32 {
        coded_distance(q, self.values(id, dim), base, metric)
    }
}

/// The distance from `q` to a coded vector given dimension by dimension,
/// plus `base` when the codes are of the residual from a coarse centroid.
fn coded_distance(
    q: &[f32],
    values: impl Iterator<Item = f32>,
    base: Option<&[f32]>,
    metric: VectorMetric,
) -> f32 {
    match base {
        Some(b) => metric_of(q, values.zip(b).map(|(v, c)| v + c), metric),
        None => metric_of(q, values, metric),
    }
}

fn metric_of(q: &[f32], values: impl Iterator<Item = f32>, metric: VectorMetric) -> f32 {
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

/// Coarse centroids for residual codes (card #1008): `k <= 256` centroids
/// of the whole vectors, learned by k-means, and each node's nearest. Binary
/// and product codes encode `vector - centroid`. On clustered vectors (what
/// embeddings are) the codes then describe where a vector sits inside its
/// cluster, where codes of the whole vector mostly spent their bits on the
/// cluster and could not tell its members apart: at 1M vectors recall@10
/// fell to 0.385 (binary) and 0.598 (PQ).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Coarse {
    pub k: usize,
    /// `k` centroids of `dim` floats.
    pub centroids: Vec<f32>,
    /// Node -> its centroid, in node order (removed nodes included).
    pub ids: Vec<u16>,
}

/// Coarse centroids at most, and the rows their k-means trains on.
const COARSE_K: usize = 256;
const COARSE_SAMPLE: usize = 25_000;
const COARSE_ITERATIONS: usize = 10;

impl Coarse {
    fn centroid(&self, c: u16, dim: usize) -> &[f32] {
        let at = c as usize * dim;
        &self.centroids[at..at + dim]
    }

    fn of_node(&self, id: u32, dim: usize) -> &[f32] {
        self.centroid(self.ids[id as usize], dim)
    }

    fn nearest(&self, v: &[f32], dim: usize) -> u16 {
        let mut best = (f32::MAX, 0usize);
        for c in 0..self.k {
            let d = squared_l2_of(v, &self.centroids[c * dim..(c + 1) * dim]);
            if d < best.0 {
                best = (d, c);
            }
        }
        best.1 as u16
    }

    /// Centroids trained on `rows` (live vectors in key order), no ids yet.
    fn train(rows: &[f32], dim: usize, threads: usize) -> Self {
        let n = rows.len() / dim.max(1);
        let k = n.clamp(1, COARSE_K);
        let stride = n.div_ceil(COARSE_SAMPLE).max(1);
        let sample: Vec<&[f32]> = (0..n)
            .step_by(stride)
            .map(|r| &rows[r * dim..(r + 1) * dim])
            .collect();
        Coarse {
            k,
            centroids: kmeans(&sample, dim, k, COARSE_ITERATIONS, 0x00C0_A25E, threads),
            ids: Vec::new(),
        }
    }

    /// Each row's nearest centroid, on `threads` threads, in row order.
    fn assign_all(&mut self, rows: &[f32], dim: usize, threads: usize) {
        let n = rows.len() / dim.max(1);
        let chunk = n.div_ceil(threads.max(1)).max(1);
        let me = &*self;
        let parts: Vec<Vec<u16>> = std::thread::scope(|scope| {
            let handles: Vec<_> = rows
                .chunks(chunk * dim)
                .map(|block| {
                    scope.spawn(move || {
                        block
                            .chunks_exact(dim)
                            .map(|row| me.nearest(row, dim))
                            .collect::<Vec<u16>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_default())
                .collect()
        });
        self.ids = parts.concat();
    }

    /// Each of `rows` minus its centroid (by `ids`), in place.
    fn subtract(&self, rows: &mut [f32], dim: usize) {
        for (id, row) in rows.chunks_exact_mut(dim).enumerate() {
            let c = self.of_node(id as u32, dim);
            row.iter_mut().zip(c).for_each(|(x, m)| *x -= m);
        }
    }
}

/// `k` centroids of `points` (`dim` floats each) by Lloyd's k-means, the
/// assignment step on `threads` threads; deterministic for a seed.
fn kmeans(
    points: &[&[f32]],
    dim: usize,
    k: usize,
    iterations: usize,
    seed: u64,
    threads: usize,
) -> Vec<f32> {
    use rand::{seq::SliceRandom, SeedableRng};
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    let mut order: Vec<usize> = (0..points.len()).collect();
    order.shuffle(&mut rng);
    let mut centroids: Vec<f32> = (0..k)
        .flat_map(|c| points[order[c % order.len()]].iter().copied())
        .collect();
    let chunk = points.len().div_ceil(threads.max(1)).max(1);
    for _ in 0..iterations {
        let assign: Vec<usize> = {
            let centroids = &centroids;
            std::thread::scope(|scope| {
                let handles: Vec<_> = points
                    .chunks(chunk)
                    .map(|block| {
                        scope.spawn(move || {
                            block
                                .iter()
                                .map(|p| {
                                    let mut best = (f32::MAX, 0usize);
                                    for c in 0..k {
                                        let d =
                                            squared_l2_of(p, &centroids[c * dim..(c + 1) * dim]);
                                        if d < best.0 {
                                            best = (d, c);
                                        }
                                    }
                                    best.1
                                })
                                .collect::<Vec<usize>>()
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|h| h.join().unwrap_or_default())
                    .collect()
            })
        };
        let mut sums = vec![0.0f32; k * dim];
        let mut counts = vec![0usize; k];
        for (p, &c) in points.iter().zip(&assign) {
            counts[c] += 1;
            for (s, v) in sums[c * dim..(c + 1) * dim].iter_mut().zip(p.iter()) {
                *s += v;
            }
        }
        for c in 0..k {
            if counts[c] == 0 {
                let p = order[(c * 7919) % order.len()];
                centroids[c * dim..(c + 1) * dim].copy_from_slice(points[p]);
                continue;
            }
            for d in 0..dim {
                centroids[c * dim + d] = sums[c * dim + d] / counts[c] as f32;
            }
        }
    }
    centroids
}

/// A graph's vectors by product quantization (card #1000): each vector is
/// cut into `dim / sub` sub-vectors of `sub` dimensions, and each of those
/// is kept as the number (one byte) of its nearest of `k <= 256` centroids,
/// learned by k-means over the sub-vectors of a sample of the vectors when
/// the graph is quantized. A node takes `dim / sub` bytes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pq {
    /// Dimensions a sub-vector.
    pub sub: usize,
    /// Centroids a sub-space.
    pub k: usize,
    /// Sub-space by sub-space, centroid by centroid: `sub` floats each.
    pub centroids: Vec<f32>,
    /// `dim / sub` codes a node, in node order (removed nodes included).
    pub codes: Vec<u8>,
}

/// The sub-vector length product quantization uses for `dim` dimensions:
/// the largest of 4, 2, 1 that divides it.
pub fn pq_sub_for(dim: usize) -> usize {
    [4usize, 2, 1]
        .into_iter()
        .find(|s| dim.is_multiple_of(*s))
        .unwrap_or(1)
}

/// The rows of a sample k-means trains on, at most.
const PQ_SAMPLE: usize = 10_000;
/// Lloyd iterations of the training.
const PQ_ITERATIONS: usize = 12;

fn squared_l2_of(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum()
}

impl Pq {
    fn m(&self, dim: usize) -> usize {
        dim / self.sub
    }

    /// Codebooks trained over `rows` (`dim` floats each), no codes yet.
    /// Deterministic: the same rows give the same codebooks.
    fn train(rows: &[f32], dim: usize, sub: usize, threads: usize) -> Self {
        use rand::{seq::SliceRandom, SeedableRng};
        let n = rows.len() / dim.max(1);
        let m = dim / sub;
        let k = n.clamp(1, 256);
        // An even sample, in row order.
        let stride = n.div_ceil(PQ_SAMPLE).max(1);
        let sample: Vec<usize> = (0..n).step_by(stride).collect();
        let train_one = |j: usize| -> Vec<f32> {
            let points: Vec<&[f32]> = sample
                .iter()
                .map(|&r| &rows[r * dim + j * sub..r * dim + (j + 1) * sub])
                .collect();
            let mut rng = rand::rngs::StdRng::seed_from_u64(0x5eed_0000 + j as u64);
            let mut order: Vec<usize> = (0..points.len()).collect();
            order.shuffle(&mut rng);
            let mut centroids: Vec<f32> = (0..k)
                .flat_map(|c| points[order[c % order.len()]].iter().copied())
                .collect();
            let mut assign = vec![0usize; points.len()];
            for _ in 0..PQ_ITERATIONS {
                for (p, point) in points.iter().enumerate() {
                    let mut best = (f32::MAX, 0);
                    for c in 0..k {
                        let d = squared_l2_of(point, &centroids[c * sub..(c + 1) * sub]);
                        if d < best.0 {
                            best = (d, c);
                        }
                    }
                    assign[p] = best.1;
                }
                let mut sums = vec![0.0f32; k * sub];
                let mut counts = vec![0usize; k];
                for (p, point) in points.iter().enumerate() {
                    let c = assign[p];
                    counts[c] += 1;
                    for (s, v) in sums[c * sub..(c + 1) * sub].iter_mut().zip(point.iter()) {
                        *s += v;
                    }
                }
                for c in 0..k {
                    if counts[c] == 0 {
                        // An empty cluster takes a point again.
                        let p = order[(c * 7919) % order.len()];
                        centroids[c * sub..(c + 1) * sub].copy_from_slice(points[p]);
                        continue;
                    }
                    for d in 0..sub {
                        centroids[c * sub + d] = sums[c * sub + d] / counts[c] as f32;
                    }
                }
            }
            centroids
        };
        let mut books: Vec<Vec<f32>> = vec![Vec::new(); m];
        let next = AtomicUsize::new(0);
        let done = parking_lot::Mutex::new(&mut books);
        std::thread::scope(|scope| {
            for _ in 0..threads.clamp(1, m.max(1)) {
                scope.spawn(|| loop {
                    let j = next.fetch_add(1, AtomicOrdering::Relaxed);
                    if j >= m {
                        break;
                    }
                    let book = train_one(j);
                    done.lock()[j] = book;
                });
            }
        });
        Pq {
            sub,
            k,
            centroids: books.concat(),
            codes: Vec::new(),
        }
    }

    /// The codes of `v`: for each sub-vector, its nearest centroid.
    fn encode(&self, v: &[f32], out: &mut Vec<u8>) {
        let (sub, k) = (self.sub, self.k);
        for (j, part) in v.chunks_exact(sub).enumerate() {
            let book = &self.centroids[j * k * sub..(j + 1) * k * sub];
            let mut best = (f32::MAX, 0usize);
            for c in 0..k {
                let d = squared_l2_of(part, &book[c * sub..(c + 1) * sub]);
                if d < best.0 {
                    best = (d, c);
                }
            }
            out.push(best.1 as u8);
        }
    }

    fn push(&mut self, v: &[f32]) {
        let mut codes = std::mem::take(&mut self.codes);
        self.encode(v, &mut codes);
        self.codes = codes;
    }

    /// Codes for every row of `rows`, on `threads` threads, in row order.
    fn encode_all(&mut self, rows: &[f32], dim: usize, threads: usize) {
        let n = rows.len() / dim.max(1);
        let chunk = n.div_ceil(threads.max(1)).max(1);
        let parts: Vec<Vec<u8>> = std::thread::scope(|scope| {
            let handles: Vec<_> = rows
                .chunks(chunk * dim)
                .map(|block| {
                    let me = &*self;
                    scope.spawn(move || {
                        let mut out = Vec::with_capacity(block.len() / me.sub);
                        for row in block.chunks_exact(dim) {
                            me.encode(row, &mut out);
                        }
                        out
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().unwrap_or_default())
                .collect()
        });
        self.codes = parts.concat();
    }

    /// Node `id`'s values: its centroids, sub-vector after sub-vector.
    fn values(&self, id: u32, dim: usize) -> impl Iterator<Item = f32> + '_ {
        let m = self.m(dim);
        let (sub, k) = (self.sub, self.k);
        let codes = &self.codes[id as usize * m..(id as usize + 1) * m];
        codes.iter().enumerate().flat_map(move |(j, &c)| {
            let at = (j * k + c as usize) * sub;
            self.centroids[at..at + sub].iter().copied()
        })
    }

    fn decode(&self, id: u32, dim: usize) -> Vec<f32> {
        self.values(id, dim).collect()
    }

    fn distance(
        &self,
        q: &[f32],
        id: u32,
        dim: usize,
        metric: VectorMetric,
        base: Option<&[f32]>,
    ) -> f32 {
        coded_distance(q, self.values(id, dim), base, metric)
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
    /// The vectors by product quantization, when the graph is quantized so.
    #[serde(skip)]
    pq: Option<Pq>,
    /// Coarse centroids of residual codes (binary and PQ, card #1008).
    #[serde(skip)]
    coarse: Option<Coarse>,
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
            pq: None,
            coarse: None,
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
        self.sq.is_some() || self.pq.is_some()
    }

    /// The vectors by product quantization, when the graph is quantized so.
    pub fn pq(&self) -> Option<&Pq> {
        self.pq.as_ref()
    }

    /// The coarse centroids binary and product codes are residuals of.
    pub fn coarse(&self) -> Option<&Coarse> {
        self.coarse.as_ref()
    }

    /// The live nodes' rows of `rows` (`dim` floats a node, node order), in
    /// key order: two nodes holding the same documents train the same
    /// centroids and codebooks only from their keys, since node numbers
    /// follow the order the vectors came in.
    ///
    /// Only an even sample of at most `max` rows is copied, the one the
    /// trainers take: at 768 dimensions a copy of every row is 3 GB a
    /// million vectors, and a 1M index ran out of memory quantizing.
    fn live_sample_by_key(&self, rows: &[f32], max: usize) -> Vec<f32> {
        let dim = self.dim;
        let ids = self.live_ids_by_key();
        let stride = ids.len().div_ceil(max).max(1);
        ids.iter()
            .step_by(stride)
            .flat_map(|&id| rows[id * dim..(id + 1) * dim].iter().copied())
            .collect()
    }

    /// The live nodes, in key order.
    fn live_ids_by_key(&self) -> Vec<usize> {
        let mut ids: Vec<usize> = (0..self.keys.len())
            .filter(|&id| !self.removed[id])
            .collect();
        ids.sort_by(|a, b| self.keys[*a].cmp(&self.keys[*b]));
        ids
    }

    /// Coarse centroids over the arena, which becomes the residuals: taken
    /// out of the graph and changed in place, so no second copy is held.
    fn residual_arena(&mut self, threads: usize) -> (Coarse, Vec<f32>) {
        let dim = self.dim;
        let sample = self.live_sample_by_key(&self.arena, COARSE_SAMPLE);
        let mut coarse = Coarse::train(&sample, dim, threads);
        drop(sample);
        let mut rows = std::mem::take(&mut self.arena);
        coarse.assign_all(&rows, dim, threads);
        coarse.subtract(&mut rows, dim);
        (coarse, rows)
    }

    /// Keep the vectors by product quantization (card #1000) and drop the
    /// f32 arena: `dim / sub` bytes a node. The codebooks are trained on
    /// the live nodes, on `threads` threads. A graph quantized otherwise
    /// is decoded first.
    pub fn quantize_pq(&mut self, threads: usize) {
        if self.pq.is_some() {
            return;
        }
        self.dequantize();
        let dim = self.dim;
        let (coarse, residuals) = self.residual_arena(threads);
        let mut pq = Pq::train(
            &self.live_sample_by_key(&residuals, PQ_SAMPLE),
            dim,
            pq_sub_for(dim),
            threads,
        );
        pq.encode_all(&residuals, dim, threads);
        self.pq = Some(pq);
        self.coarse = Some(coarse);
        self.arena = Vec::new();
    }

    /// Keep the vectors as codes of `bits` a dimension (8 or 1) and drop
    /// the f32 arena: a quarter of the memory at 8, a thirty-second at 1.
    /// Bounds are taken over the live nodes. A graph quantized with other
    /// bits is decoded first.
    pub fn quantize(&mut self, bits: u8) {
        let bits = if bits == 1 { 1 } else { 8 };
        if self.pq.is_some() {
            self.dequantize();
        }
        match &self.sq {
            Some(sq) if sq.bits == bits => return,
            Some(_) => self.dequantize(),
            None => {}
        }
        let dim = self.dim;
        // One bit a dimension encodes the residual from a coarse centroid;
        // a byte is precise enough over the whole vector.
        let (coarse, rows) = if bits == 1 {
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
            let (coarse, residuals) = self.residual_arena(threads);
            (Some(coarse), residuals)
        } else {
            (None, std::mem::take(&mut self.arena))
        };
        // Bounds over the live rows, in key order (the mean of binary
        // codes sums in that order), with no copy of them.
        let live = self.live_ids_by_key();
        let mut sq = Sq8::bounds_of(
            live.iter().map(|&id| &rows[id * dim..(id + 1) * dim]),
            dim,
            bits,
        );
        drop(live);
        sq.codes
            .reserve_exact(self.keys.len() * sq.bytes_per_node(dim));
        for row in rows.chunks_exact(dim) {
            sq.push(row);
        }
        self.sq = Some(sq);
        self.coarse = coarse;
        self.arena = Vec::new();
    }

    /// Back to f32 vectors, decoded from the codes: they keep the
    /// quantization's error until the index is rebuilt from its documents.
    pub fn dequantize(&mut self) {
        if !self.is_quantized() {
            return;
        }
        let mut arena = Vec::with_capacity(self.keys.len() * self.dim);
        for id in 0..self.keys.len() as u32 {
            arena.extend(self.decoded(id));
        }
        self.sq = None;
        self.pq = None;
        self.coarse = None;
        self.arena = arena;
    }

    /// After a load: the coarse centroids of residual codes. `false` when
    /// they do not match the nodes (nothing is restored).
    pub fn restore_coarse(&mut self, coarse: Coarse) -> bool {
        if coarse.k == 0
            || coarse.k > COARSE_K
            || coarse.centroids.len() != coarse.k * self.dim
            || coarse.ids.len() != self.keys.len()
            || coarse.ids.iter().any(|&c| c as usize >= coarse.k)
        {
            return false;
        }
        self.coarse = Some(coarse);
        true
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
        self.pq = None;
        self.coarse = None;
        self.renumber();
        true
    }

    /// After a load: the vectors as [`Self::pq`] gave them. `false` when
    /// they do not match the nodes (nothing is restored).
    pub fn restore_pq(&mut self, pq: Pq) -> bool {
        let dim = self.dim;
        if pq.sub == 0
            || !dim.is_multiple_of(pq.sub)
            || pq.k == 0
            || pq.k > 256
            || pq.centroids.len() != (dim / pq.sub) * pq.k * pq.sub
            || pq.codes.len() != self.keys.len() * (dim / pq.sub)
            || self.removed.len() != self.keys.len()
        {
            return false;
        }
        self.arena = Vec::new();
        self.sq = None;
        self.coarse = None;
        self.pq = Some(pq);
        self.renumber();
        true
    }

    /// After a load: the vectors as [`Self::sq`] gave them. `false` when
    /// they do not match the nodes (nothing is restored).
    pub fn restore_sq(&mut self, sq: Sq8) -> bool {
        if !matches!(sq.bits, 1 | 8)
            || sq.codes.len() != self.keys.len() * sq.bytes_per_node(self.dim)
            || sq.min.len() != self.dim
            || sq.step.len() != self.dim
            || self.removed.len() != self.keys.len()
        {
            return false;
        }
        self.arena = Vec::new();
        self.pq = None;
        self.coarse = None;
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
    /// Node `id`'s vector: borrowed from the arena, or decoded from its
    /// codes when the graph is quantized (its arena is then empty).
    #[inline]
    fn vector(&self, id: u32) -> Cow<'_, [f32]> {
        if !self.arena.is_empty() {
            let at = id as usize * self.dim;
            return Cow::Borrowed(&self.arena[at..at + self.dim]);
        }
        Cow::Owned(self.decoded(id))
    }

    #[inline(never)]
    #[cold]
    fn decoded(&self, id: u32) -> Vec<f32> {
        let mut v = match (&self.pq, &self.sq) {
            (Some(pq), _) => pq.decode(id, self.dim),
            (_, Some(sq)) => sq.decode(id, self.dim),
            _ => vec![0.0; self.dim],
        };
        if let Some(coarse) = &self.coarse {
            for (x, c) in v.iter_mut().zip(coarse.of_node(id, self.dim)) {
                *x += c;
            }
        }
        v
    }

    /// The distance from `q` to node `id`. An f32 graph is the one with an
    /// arena: one test, then the arena slice, so this stays small enough to
    /// be inlined into the walk as it was before quantization (card #1006,
    /// a test of `pq` then `sq` on every distance cost 6-8% of a search).
    #[inline(always)]
    fn dist_to(&self, q: &[f32], id: u32) -> f32 {
        if !self.arena.is_empty() {
            let at = id as usize * self.dim;
            return distance(q, &self.arena[at..at + self.dim], self.metric);
        }
        self.dist_to_codes(q, id)
    }

    #[inline(never)]
    fn dist_to_codes(&self, q: &[f32], id: u32) -> f32 {
        let base = self.coarse.as_ref().map(|c| c.of_node(id, self.dim));
        match (&self.pq, &self.sq) {
            (Some(pq), _) => pq.distance(q, id, self.dim, self.metric, base),
            (_, Some(sq)) => sq.distance(q, id, self.dim, self.metric, base),
            _ => f32::MAX,
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
        let mut v = prepared(vector, self.metric);
        // Residual codes: the node's coarse centroid, and what is left.
        if let Some(coarse) = self.coarse.as_mut() {
            let c = coarse.nearest(&v, self.dim);
            coarse.ids.push(c);
            for (x, m) in v.iter_mut().zip(coarse.centroid(c, self.dim)) {
                *x -= m;
            }
        }
        if let Some(pq) = self.pq.as_mut() {
            pq.push(&v);
        } else {
            match self.sq.as_mut() {
                Some(sq) => sq.push(&v),
                None => self.arena.extend_from_slice(&v),
            }
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
        self.pq = None;
        self.coarse = None;
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
