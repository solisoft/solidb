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

fn nexts<'a>(
    edges: &'a [Value],
    current: &str,
    dir: &EdgeDirection,
) -> Vec<(String, &'a Value, f64)> {
    let mut out = Vec::new();
    for e in edges {
        let from = e.get("_from").and_then(Value::as_str);
        let to = e.get("_to").and_then(Value::as_str);
        let nxt = match dir {
            EdgeDirection::Outbound if from == Some(current) => to,
            EdgeDirection::Inbound if to == Some(current) => from,
            EdgeDirection::Any if from == Some(current) => to,
            EdgeDirection::Any if to == Some(current) => from,
            _ => None,
        };
        if let Some(n) = nxt {
            out.push((n.to_string(), e, 1.0));
        }
    }
    out
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

fn max_paths() -> usize {
    std::env::var("SOLIDB_MAX_PATHS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0)
        .unwrap_or(256)
}

pub fn find_paths(
    sp: &ShortestPathClause,
    edges: &[Value],
    start: &str,
    end: &str,
) -> Result<Vec<FoundPath>, String> {
    let wfield = sp.weight.as_deref();
    match sp.mode {
        PathFindMode::Shortest if wfield.is_some() => {
            Ok(dijkstra(edges, start, end, &sp.direction, wfield)?
                .into_iter()
                .collect())
        }
        PathFindMode::Shortest => Ok(bfs_one(edges, start, end, &sp.direction)
            .into_iter()
            .collect()),
        PathFindMode::AllShortest => Ok(all_shortest(edges, start, end, &sp.direction)),
        PathFindMode::KShortest => {
            let k = sp.k.unwrap_or(3).max(1).min(max_paths());
            Ok(k_shortest(edges, start, end, &sp.direction, wfield, k)?)
        }
        PathFindMode::KPaths => {
            let min = sp.min_len.unwrap_or(1);
            let max = sp.max_len.unwrap_or(5).max(min);
            let limit = sp.limit.unwrap_or(100).min(max_paths());
            Ok(k_paths(edges, start, end, &sp.direction, min, max, limit))
        }
    }
}

fn bfs_one(edges: &[Value], start: &str, end: &str, dir: &EdgeDirection) -> Option<FoundPath> {
    let mut parent: HashMap<String, (Option<String>, Option<Value>)> = HashMap::new();
    let mut q = VecDeque::new();
    parent.insert(start.to_string(), (None, None));
    q.push_back(start.to_string());
    while let Some(cur) = q.pop_front() {
        if cur == end {
            return Some(rebuild(&parent, end));
        }
        for (n, e, _) in nexts(edges, &cur, dir) {
            if parent.contains_key(&n) {
                continue;
            }
            parent.insert(n.clone(), (Some(cur.clone()), Some(e.clone())));
            q.push_back(n);
        }
    }
    None
}

fn dijkstra(
    edges: &[Value],
    start: &str,
    end: &str,
    dir: &EdgeDirection,
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
        for (n, e, _) in nexts(edges, &id, dir) {
            let w = edge_weight(e, wfield)?;
            let nd = cost + w;
            if nd < *dist.get(&n).unwrap_or(&f64::INFINITY) {
                dist.insert(n.clone(), nd);
                parent.insert(n.clone(), (Some(id.clone()), Some(e.clone())));
                heap.push(State { cost: nd, id: n });
            }
        }
    }
    Ok(None)
}

fn all_shortest(edges: &[Value], start: &str, end: &str, dir: &EdgeDirection) -> Vec<FoundPath> {
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
        for (n, e, _) in nexts(edges, &cur, dir) {
            let nd = d + 1;
            match dist.get(&n).copied() {
                None => {
                    dist.insert(n.clone(), nd);
                    preds.insert(n.clone(), vec![(cur.clone(), e.clone())]);
                    q.push_back(n);
                }
                Some(old) if nd == old => {
                    preds.entry(n).or_default().push((cur.clone(), e.clone()));
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

fn k_shortest(
    edges: &[Value],
    start: &str,
    end: &str,
    dir: &EdgeDirection,
    wfield: Option<&str>,
    k: usize,
) -> Result<Vec<FoundPath>, String> {
    yen(edges, start, end, dir, wfield, k)
}

/// Yen's algorithm: successive shortest paths with spur deviations.
fn yen(
    edges: &[Value],
    start: &str,
    end: &str,
    dir: &EdgeDirection,
    wfield: Option<&str>,
    k: usize,
) -> Result<Vec<FoundPath>, String> {
    let Some(first) = dijkstra(edges, start, end, dir, wfield)? else {
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
            let filtered: Vec<Value> = edges
                .iter()
                .filter(|e| {
                    let from = e.get("_from").and_then(Value::as_str).unwrap_or("");
                    let to = e.get("_to").and_then(Value::as_str).unwrap_or("");
                    let pair_ok = match dir {
                        EdgeDirection::Inbound => {
                            !forbidden.contains(&(to.to_string(), from.to_string()))
                        }
                        _ => !forbidden.contains(&(from.to_string(), to.to_string())),
                    };
                    let node_ok = !blocked_nodes.contains(from) && !blocked_nodes.contains(to)
                        || from == spur
                        || to == spur;
                    pair_ok && node_ok
                })
                .cloned()
                .collect();
            if let Some(spur_path) = dijkstra(&filtered, &spur, end, dir, wfield)? {
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
    edges: &[Value],
    start: &str,
    end: &str,
    dir: &EdgeDirection,
    min: usize,
    max: usize,
    limit: usize,
) -> Vec<FoundPath> {
    let mut out = Vec::new();
    fn dfs(
        edges: &[Value],
        dir: &EdgeDirection,
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
        for (n, e, _) in nexts(edges, cur, dir) {
            if seen.contains(&n) {
                continue;
            }
            seen.insert(n.clone());
            verts.push(n.clone());
            eds.push(e.clone());
            dfs(edges, dir, end, min, max, limit, &n, seen, verts, eds, out);
            eds.pop();
            verts.pop();
            seen.remove(&n);
        }
    }
    let mut seen = HashSet::new();
    seen.insert(start.to_string());
    dfs(
        edges,
        dir,
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
