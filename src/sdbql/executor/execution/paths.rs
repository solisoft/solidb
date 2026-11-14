//! Hop-count BFS, Dijkstra, all-shortest, k-shortest, and k-paths.

use crate::sdbql::ast::{EdgeDirection, PathFindMode, ShortestPathClause};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

#[derive(Clone, Debug)]
pub struct FoundPath {
    pub vertices: Vec<String>,
    pub edges: Vec<Value>,
    pub weight: f64,
}

#[derive(Clone)]
struct State {
    cost: f64,
    id: String,
}

impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost && self.id == other.id
    }
}
impl Eq for State {}
impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
            .then_with(|| self.id.cmp(&other.id))
    }
}

/// Where a search finds the edges that leave a vertex.
pub trait Neighbours {
    /// The edges incident to `current` in the search's direction, each with
    /// the vertex on its far side, in the order of the edges' keys.
    fn nexts(&self, current: &str) -> Vec<(String, Value)>;
}

/// Edges already read into memory, indexed by their ends once: looking for
/// a vertex's edges in the whole list at every step made a search cost
/// (vertices visited) x (edges in the collection).
pub struct Scanned<'a> {
    edges: &'a [Value],
    by_from: HashMap<&'a str, Vec<usize>>,
    by_to: HashMap<&'a str, Vec<usize>>,
    dir: EdgeDirection,
}

impl<'a> Scanned<'a> {
    pub fn new(edges: &'a [Value], dir: &EdgeDirection) -> Self {
        let mut by_from: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut by_to: HashMap<&str, Vec<usize>> = HashMap::new();
        for (at, e) in edges.iter().enumerate() {
            if let Some(from) = e.get("_from").and_then(Value::as_str) {
                by_from.entry(from).or_default().push(at);
            }
            if let Some(to) = e.get("_to").and_then(Value::as_str) {
                by_to.entry(to).or_default().push(at);
            }
        }
        Self {
            edges,
            by_from,
            by_to,
            dir: dir.clone(),
        }
    }
}

impl Neighbours for Scanned<'_> {
    fn nexts(&self, current: &str) -> Vec<(String, Value)> {
        let none = Vec::new();
        let from = self.by_from.get(current).unwrap_or(&none);
        let to = self.by_to.get(current).unwrap_or(&none);
        let at: Vec<usize> = match self.dir {
            EdgeDirection::Outbound => from.clone(),
            EdgeDirection::Inbound => to.clone(),
            EdgeDirection::Any => {
                let mut both: Vec<usize> = from.iter().chain(to).copied().collect();
                both.sort_unstable();
                both.dedup();
                both
            }
        };
        at.into_iter()
            .filter_map(|i| {
                let e = &self.edges[i];
                let from = e.get("_from").and_then(Value::as_str);
                let to = e.get("_to").and_then(Value::as_str);
                let next = match self.dir {
                    EdgeDirection::Outbound => to,
                    EdgeDirection::Inbound => from,
                    EdgeDirection::Any if from == Some(current) => to,
                    EdgeDirection::Any => from,
                };
                next.map(|n| (n.to_string(), e.clone()))
            })
            .collect()
    }
}

/// A graph without the edges and vertices Yen's algorithm sets aside for
/// one spur search.
struct Without<'g> {
    graph: &'g dyn Neighbours,
    dir: &'g EdgeDirection,
    forbidden: &'g HashSet<(String, String)>,
    blocked: &'g HashSet<&'g str>,
    spur: &'g str,
}

impl Neighbours for Without<'_> {
    fn nexts(&self, current: &str) -> Vec<(String, Value)> {
        let mut nexts = self.graph.nexts(current);
        nexts.retain(|(_, e)| {
            let from = e.get("_from").and_then(Value::as_str).unwrap_or("");
            let to = e.get("_to").and_then(Value::as_str).unwrap_or("");
            let pair_ok = match self.dir {
                EdgeDirection::Inbound => {
                    !self.forbidden.contains(&(to.to_string(), from.to_string()))
                }
                _ => !self.forbidden.contains(&(from.to_string(), to.to_string())),
            };
            let node_ok = !self.blocked.contains(from) && !self.blocked.contains(to)
                || from == self.spur
                || to == self.spur;
            pair_ok && node_ok
        });
        nexts
    }
}

fn edge_weight(e: &Value, field: Option<&str>) -> Result<f64, String> {
    let Some(f) = field else {
        return Ok(1.0);
    };
    match e.get(f) {
        None => Ok(1.0),
        Some(v) => {
            let n = v.as_f64().ok_or_else(|| {
                format!("SHORTEST_PATH: weight field '{f}' must be a non-negative number")
            })?;
            if n < 0.0 {
                return Err(format!("SHORTEST_PATH: negative weight {n} on field '{f}'"));
            }
            Ok(n)
        }
    }
}

/// Longest path K_PATHS will search for. Bounds the DFS recursion depth.
const MAX_PATH_HOPS: usize = 64;

fn max_paths() -> usize {
    std::env::var("SOLIDB_MAX_PATHS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(256)
}

pub fn find_paths(
    sp: &ShortestPathClause,
    graph: &dyn Neighbours,
    start: &str,
    end: &str,
) -> Result<Vec<FoundPath>, String> {
    let wfield = sp.weight.as_deref();
    match sp.mode {
        PathFindMode::Shortest if wfield.is_some() => {
            Ok(dijkstra(graph, start, end, wfield)?.into_iter().collect())
        }
        PathFindMode::Shortest => Ok(bfs_one(graph, start, end).into_iter().collect()),
        PathFindMode::AllShortest => Ok(all_shortest(graph, start, end)),
        PathFindMode::KShortest => {
            let k = sp.k.unwrap_or(3).max(1).min(max_paths());
            Ok(yen(graph, start, end, &sp.direction, wfield, k)?)
        }
        PathFindMode::KPaths => {
            // The DFS below recurses one frame per hop, and a stack overflow
            // is not a panic: it takes the process. Clamp the depth.
            let min = sp.min_len.unwrap_or(1).min(MAX_PATH_HOPS);
            let max = sp.max_len.unwrap_or(5).max(min).min(MAX_PATH_HOPS);
            let limit = sp.limit.unwrap_or(100).min(max_paths());
            Ok(k_paths(graph, start, end, min, max, limit))
        }
    }
}

fn bfs_one(graph: &dyn Neighbours, start: &str, end: &str) -> Option<FoundPath> {
    let mut parent: HashMap<String, (Option<String>, Option<Value>)> = HashMap::new();
    let mut q = VecDeque::new();
    parent.insert(start.to_string(), (None, None));
    q.push_back(start.to_string());
    while let Some(cur) = q.pop_front() {
        if cur == end {
            return Some(rebuild(&parent, end));
        }
        for (n, e) in graph.nexts(&cur) {
            if parent.contains_key(&n) {
                continue;
            }
            parent.insert(n.clone(), (Some(cur.clone()), Some(e)));
            q.push_back(n);
        }
    }
    None
}

fn dijkstra(
    graph: &dyn Neighbours,
    start: &str,
    end: &str,
    wfield: Option<&str>,
) -> Result<Option<FoundPath>, String> {
    let mut dist: HashMap<String, f64> = HashMap::new();
    let mut parent: HashMap<String, (Option<String>, Option<Value>)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    dist.insert(start.to_string(), 0.0);
    parent.insert(start.to_string(), (None, None));
    heap.push(State {
        cost: 0.0,
        id: start.to_string(),
    });
    while let Some(State { cost, id }) = heap.pop() {
        if cost > *dist.get(&id).unwrap_or(&f64::INFINITY) {
            continue;
        }
        if id == end {
            let mut p = rebuild(&parent, end);
            p.weight = cost;
            return Ok(Some(p));
        }
        for (n, e) in graph.nexts(&id) {
            let w = edge_weight(&e, wfield)?;
            let nd = cost + w;
            if nd < *dist.get(&n).unwrap_or(&f64::INFINITY) {
                dist.insert(n.clone(), nd);
                parent.insert(n.clone(), (Some(id.clone()), Some(e)));
                heap.push(State { cost: nd, id: n });
            }
        }
    }
    Ok(None)
}

fn all_shortest(graph: &dyn Neighbours, start: &str, end: &str) -> Vec<FoundPath> {
    let mut dist: HashMap<String, usize> = HashMap::new();
    let mut preds: HashMap<String, Vec<(String, Value)>> = HashMap::new();
    let mut q = VecDeque::new();
    dist.insert(start.to_string(), 0);
    q.push_back(start.to_string());
    while let Some(cur) = q.pop_front() {
        let d = dist[&cur];
        if let Some(&de) = dist.get(end) {
            if d >= de {
                continue;
            }
        }
        for (n, e) in graph.nexts(&cur) {
            let nd = d + 1;
            match dist.get(&n).copied() {
                None => {
                    dist.insert(n.clone(), nd);
                    preds.insert(n.clone(), vec![(cur.clone(), e)]);
                    q.push_back(n);
                }
                Some(old) if nd == old => {
                    preds.entry(n).or_default().push((cur.clone(), e));
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    fn rec(
        node: &str,
        start: &str,
        preds: &HashMap<String, Vec<(String, Value)>>,
        verts: &mut Vec<String>,
        eds: &mut Vec<Value>,
        out: &mut Vec<FoundPath>,
    ) {
        // The number of shortest paths is exponential in a layered graph;
        // stop enumerating once the caller's cap is full rather than
        // truncating afterwards.
        if out.len() >= max_paths() {
            return;
        }
        if node == start {
            let mut v = verts.clone();
            v.push(start.to_string());
            v.reverse();
            let mut e = eds.clone();
            e.reverse();
            out.push(FoundPath {
                vertices: v,
                edges: e,
                weight: eds.len() as f64,
            });
            return;
        }
        let Some(ps) = preds.get(node) else {
            return;
        };
        verts.push(node.to_string());
        for (p, e) in ps {
            eds.push(e.clone());
            rec(p, start, preds, verts, eds, out);
            eds.pop();
        }
        verts.pop();
    }
    rec(
        end,
        start,
        &preds,
        &mut Vec::new(),
        &mut Vec::new(),
        &mut out,
    );
    out.truncate(max_paths());
    out
}

/// Yen's algorithm: successive shortest paths with spur deviations.
fn yen(
    graph: &dyn Neighbours,
    start: &str,
    end: &str,
    dir: &EdgeDirection,
    wfield: Option<&str>,
    k: usize,
) -> Result<Vec<FoundPath>, String> {
    let Some(first) = dijkstra(graph, start, end, wfield)? else {
        return Ok(vec![]);
    };
    let mut a = vec![first];
    let mut b: Vec<FoundPath> = Vec::new();
    for _ in 1..k {
        let prev = a.last().unwrap().clone();
        for i in 0..prev.vertices.len().saturating_sub(1) {
            let spur = prev.vertices[i].clone();
            let root = &prev.vertices[..=i];
            let mut forbidden: HashSet<(String, String)> = HashSet::new();
            for p in &a {
                if p.vertices.len() > i && &p.vertices[..=i] == root {
                    forbidden.insert((p.vertices[i].clone(), p.vertices[i + 1].clone()));
                }
            }
            let blocked_nodes: HashSet<&str> = root[..root.len().saturating_sub(1)]
                .iter()
                .map(|s| s.as_str())
                .collect();
            let without = Without {
                graph,
                dir,
                forbidden: &forbidden,
                blocked: &blocked_nodes,
                spur: &spur,
            };
            if let Some(spur_path) = dijkstra(&without, &spur, end, wfield)? {
                let mut verts = root[..root.len() - 1].to_vec();
                verts.extend(spur_path.vertices);
                let mut eds = prev.edges[..i].to_vec();
                eds.extend(spur_path.edges);
                let mut w = 0.0;
                for e in &eds {
                    w += edge_weight(e, wfield)?;
                }
                let cand = FoundPath {
                    vertices: verts,
                    edges: eds,
                    weight: w,
                };
                if !a.iter().any(|p| p.vertices == cand.vertices)
                    && !b.iter().any(|p| p.vertices == cand.vertices)
                {
                    b.push(cand);
                }
            }
        }
        if b.is_empty() {
            break;
        }
        b.sort_by(|x, y| x.weight.partial_cmp(&y.weight).unwrap_or(Ordering::Equal));
        a.push(b.remove(0));
    }
    Ok(a)
}

fn k_paths(
    graph: &dyn Neighbours,
    start: &str,
    end: &str,
    min: usize,
    max: usize,
    limit: usize,
) -> Vec<FoundPath> {
    let mut out = Vec::new();
    // Recursive walker: every argument is either search input or mutable
    // accumulator, so bundling them into a struct renames the problem rather
    // than removing it.
    #[allow(clippy::too_many_arguments)]
    fn dfs(
        graph: &dyn Neighbours,
        end: &str,
        min: usize,
        max: usize,
        limit: usize,
        cur: &str,
        seen: &mut HashSet<String>,
        verts: &mut Vec<String>,
        eds: &mut Vec<Value>,
        out: &mut Vec<FoundPath>,
    ) {
        if out.len() >= limit {
            return;
        }
        let hops = eds.len();
        if cur == end && hops >= min && hops <= max {
            out.push(FoundPath {
                vertices: verts.clone(),
                edges: eds.clone(),
                weight: hops as f64,
            });
        }
        if hops >= max {
            return;
        }
        for (n, e) in graph.nexts(cur) {
            if seen.contains(&n) {
                continue;
            }
            seen.insert(n.clone());
            verts.push(n.clone());
            eds.push(e);
            dfs(graph, end, min, max, limit, &n, seen, verts, eds, out);
            eds.pop();
            verts.pop();
            seen.remove(&n);
        }
    }
    let mut seen = HashSet::new();
    seen.insert(start.to_string());
    dfs(
        graph,
        end,
        min,
        max,
        limit,
        start,
        &mut seen,
        &mut vec![start.to_string()],
        &mut Vec::new(),
        &mut out,
    );
    out
}

fn rebuild(parent: &HashMap<String, (Option<String>, Option<Value>)>, end: &str) -> FoundPath {
    let mut verts = Vec::new();
    let mut eds = Vec::new();
    let mut cur = end.to_string();
    loop {
        verts.push(cur.clone());
        match parent.get(&cur) {
            Some((Some(p), Some(e))) => {
                eds.push(e.clone());
                cur = p.clone();
            }
            _ => break,
        }
    }
    verts.reverse();
    eds.reverse();
    FoundPath {
        weight: eds.len() as f64,
        vertices: verts,
        edges: eds,
    }
}

pub fn path_to_json(p: &FoundPath) -> Value {
    json!({
        "vertices": p.vertices,
        "edges": p.edges,
        "weight": p.weight,
    })
}
