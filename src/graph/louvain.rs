//! Louvain community detection — a pure, seeded implementation.
//!
//! Standard two-phase Louvain: repeated local moving (greedy modularity gain)
//! followed by aggregation of communities into super-nodes, until a pass no
//! longer merges anything. Deterministic for a fixed `seed`.

use super::model::Graph;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;
use std::collections::HashMap;

/// Detect communities. Returns a community label per node, aligned with
/// `graph.node_ids`. `resolution` > 1 yields smaller communities; `seed` makes
/// the (order-dependent) result reproducible.
pub fn louvain(graph: &Graph, resolution: f64, seed: u64) -> Vec<usize> {
    let n = graph.node_ids.len();
    if n == 0 {
        return Vec::new();
    }
    if graph.total_weight <= 0.0 {
        // No edges: each node is its own community.
        return (0..n).collect();
    }

    let mut level = Level::from_graph(graph);
    let mut orig_to_super: Vec<usize> = (0..n).collect();

    loop {
        let comms = level.local_moving(resolution, seed);
        let (comm_of_super, k) = compact(&comms);
        for s in orig_to_super.iter_mut() {
            *s = comm_of_super[*s];
        }
        if k == level.n {
            break; // nothing merged -> converged
        }
        level = level.aggregate(&comm_of_super, k);
        if level.n <= 1 {
            break;
        }
    }
    orig_to_super
}

struct Level {
    n: usize,
    adj: Vec<Vec<(usize, f64)>>, // symmetric, self-loop-free
    k: Vec<f64>,                 // weighted degree per node
    m2: f64,                     // 2m — invariant across levels
}

impl Level {
    fn from_graph(g: &Graph) -> Self {
        let n = g.node_ids.len();
        let adj = g.adjacency.clone();
        let k = g.degrees.clone();
        let m2: f64 = k.iter().sum();
        Level { n, adj, k, m2 }
    }

    /// Greedy local moving until no node changes community.
    fn local_moving(&self, resolution: f64, seed: u64) -> Vec<usize> {
        let mut comm: Vec<usize> = (0..self.n).collect();
        let mut sigma_tot: Vec<f64> = self.k.clone(); // total degree per community
        let mut rng = StdRng::seed_from_u64(seed);
        let mut order: Vec<usize> = (0..self.n).collect();
        order.shuffle(&mut rng);

        let mut improved = true;
        let mut iterations = 0;
        while improved && iterations < 100 {
            improved = false;
            iterations += 1;
            for &i in &order {
                let ci = comm[i];
                // Weight from i to each neighbouring community.
                let mut k_i_to: HashMap<usize, f64> = HashMap::new();
                for &(j, w) in &self.adj[i] {
                    if j == i {
                        continue;
                    }
                    *k_i_to.entry(comm[j]).or_insert(0.0) += w;
                }
                // Isolate i from its community.
                sigma_tot[ci] -= self.k[i];
                // Gain of (re)joining ci is the baseline to beat.
                let mut best_comm = ci;
                let mut best_gain = k_i_to.get(&ci).copied().unwrap_or(0.0)
                    - resolution * sigma_tot[ci] * self.k[i] / self.m2;
                for (&c, &k_i_in) in &k_i_to {
                    let gain = k_i_in - resolution * sigma_tot[c] * self.k[i] / self.m2;
                    if gain > best_gain {
                        best_gain = gain;
                        best_comm = c;
                    }
                }
                sigma_tot[best_comm] += self.k[i];
                if best_comm != ci {
                    comm[i] = best_comm;
                    improved = true;
                }
            }
        }
        comm
    }

    /// Collapse each community into a super-node. Degrees are preserved
    /// (`k'[c] = Σ member degrees`), so `m2` is unchanged.
    fn aggregate(&self, comm_of: &[usize], kcount: usize) -> Level {
        let mut edge_map: HashMap<(usize, usize), f64> = HashMap::new();
        for i in 0..self.n {
            let ci = comm_of[i];
            for &(j, w) in &self.adj[i] {
                if j <= i {
                    continue; // process each undirected pair once; skip self-loops
                }
                let cj = comm_of[j];
                if ci != cj {
                    let key = if ci < cj { (ci, cj) } else { (cj, ci) };
                    *edge_map.entry(key).or_insert(0.0) += w;
                }
            }
        }
        let mut adj = vec![Vec::new(); kcount];
        for ((a, b), w) in edge_map {
            adj[a].push((b, w));
            adj[b].push((a, w));
        }
        let mut k = vec![0.0; kcount];
        for i in 0..self.n {
            k[comm_of[i]] += self.k[i];
        }
        Level {
            n: kcount,
            adj,
            k,
            m2: self.m2,
        }
    }
}

/// Renumber arbitrary community ids into a dense `0..k` range.
fn compact(comms: &[usize]) -> (Vec<usize>, usize) {
    let mut map: HashMap<usize, usize> = HashMap::new();
    let mut out = vec![0usize; comms.len()];
    let mut next = 0;
    for (i, &c) in comms.iter().enumerate() {
        let id = *map.entry(c).or_insert_with(|| {
            let v = next;
            next += 1;
            v
        });
        out[i] = id;
    }
    (out, next)
}

#[cfg(test)]
mod tests {
    use super::super::model::GraphBuilder;
    use super::*;

    fn community_count(labels: &[usize]) -> usize {
        let mut s: Vec<usize> = labels.to_vec();
        s.sort_unstable();
        s.dedup();
        s.len()
    }

    #[test]
    fn two_triangles_with_a_bridge_yield_two_communities() {
        let mut b = GraphBuilder::new();
        for (u, v) in [("0", "1"), ("1", "2"), ("2", "0")] {
            b.add_edge(u, v, 1.0);
        }
        for (u, v) in [("3", "4"), ("4", "5"), ("5", "3")] {
            b.add_edge(u, v, 1.0);
        }
        b.add_edge("2", "3", 1.0); // bridge
        let g = b.build();
        let labels = louvain(&g, 1.0, 42);
        assert_eq!(community_count(&labels), 2);
        // The two triangles are internally coherent.
        assert_eq!(labels[0], labels[1]);
        assert_eq!(labels[1], labels[2]);
        assert_eq!(labels[3], labels[4]);
        assert_eq!(labels[4], labels[5]);
        assert_ne!(labels[0], labels[3]);
    }

    #[test]
    fn complete_graph_is_one_community() {
        let mut b = GraphBuilder::new();
        let nodes = ["a", "b", "c", "d"];
        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                b.add_edge(nodes[i], nodes[j], 1.0);
            }
        }
        let g = b.build();
        assert_eq!(community_count(&louvain(&g, 1.0, 7)), 1);
    }

    #[test]
    fn disconnected_components_are_separate() {
        let mut b = GraphBuilder::new();
        b.add_edge("a", "b", 1.0);
        b.add_edge("c", "d", 1.0);
        let g = b.build();
        let labels = louvain(&g, 1.0, 1);
        assert_eq!(community_count(&labels), 2);
        assert_eq!(labels[0], labels[1]);
        assert_ne!(labels[0], labels[2]);
    }

    #[test]
    fn empty_and_edgeless_graphs_are_safe() {
        let empty = GraphBuilder::new().build();
        assert!(louvain(&empty, 1.0, 1).is_empty());
    }
}
