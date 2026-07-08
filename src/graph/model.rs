//! Undirected weighted graph built from an edge collection.

use std::collections::HashMap;

/// Undirected weighted graph with interned node ids. `adjacency` is symmetric
/// (each undirected edge appears in both endpoints' lists). `total_weight` is
/// `m` — the sum of edge weights counting each undirected edge once.
pub struct Graph {
    pub node_ids: Vec<String>,
    pub adjacency: Vec<Vec<(usize, f64)>>,
    pub degrees: Vec<f64>,
    pub total_weight: f64,
}

impl Graph {
    pub fn node_count(&self) -> usize {
        self.node_ids.len()
    }
}

/// Accumulates edges (summing weights of parallel edges, ignoring self-loops)
/// and interns vertex ids, then produces a [`Graph`].
#[derive(Default)]
pub struct GraphBuilder {
    index: HashMap<String, usize>,
    node_ids: Vec<String>,
    edges: HashMap<(usize, usize), f64>,
}

impl GraphBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    fn intern(&mut self, id: &str) -> usize {
        if let Some(&i) = self.index.get(id) {
            return i;
        }
        let i = self.node_ids.len();
        self.node_ids.push(id.to_string());
        self.index.insert(id.to_string(), i);
        i
    }

    /// Add an undirected edge. Self-loops are dropped (they don't affect
    /// community structure). Parallel edges accumulate their weight.
    pub fn add_edge(&mut self, from: &str, to: &str, weight: f64) {
        let a = self.intern(from);
        let b = self.intern(to);
        if a == b {
            return;
        }
        let key = if a < b { (a, b) } else { (b, a) };
        *self.edges.entry(key).or_insert(0.0) += weight;
    }

    pub fn node_count(&self) -> usize {
        self.node_ids.len()
    }

    pub fn build(self) -> Graph {
        let n = self.node_ids.len();
        let mut adjacency = vec![Vec::new(); n];
        let mut degrees = vec![0.0; n];
        let mut total_weight = 0.0;
        for ((a, b), w) in self.edges {
            adjacency[a].push((b, w));
            adjacency[b].push((a, w));
            degrees[a] += w;
            degrees[b] += w;
            total_weight += w;
        }
        Graph {
            node_ids: self.node_ids,
            adjacency,
            degrees,
            total_weight,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interns_nodes_and_sums_parallel_edges() {
        let mut b = GraphBuilder::new();
        b.add_edge("a", "b", 1.0);
        b.add_edge("b", "a", 2.0); // same undirected pair -> weight sums
        b.add_edge("a", "a", 5.0); // self-loop dropped
        let g = b.build();
        assert_eq!(g.node_count(), 2);
        assert_eq!(g.total_weight, 3.0);
        assert_eq!(g.degrees[0], 3.0);
        assert_eq!(g.degrees[1], 3.0);
    }
}
