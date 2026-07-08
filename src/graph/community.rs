//! Assemble labelled communities from Louvain output.

use super::louvain::louvain;
use super::model::Graph;
use std::collections::{HashMap, HashSet};

/// A detected community: its member vertex ids, size, internal edge weight, and
/// the highest-degree members (useful as representatives / for summarization).
pub struct Community {
    pub id: usize,
    pub members: Vec<String>,
    pub size: usize,
    pub internal_edges: f64,
    pub top_nodes: Vec<(String, usize)>,
}

/// Run Louvain and assemble communities, dropping those below `min_size`.
/// Communities are re-id'd `0..k` by descending size (stable ties by smallest
/// member index) so ids are deterministic for a fixed seed.
pub fn detect_communities(
    graph: &Graph,
    resolution: f64,
    seed: u64,
    min_size: usize,
) -> Vec<Community> {
    let labels = louvain(graph, resolution, seed);
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for (node, &c) in labels.iter().enumerate() {
        groups.entry(c).or_default().push(node);
    }

    let mut group_vec: Vec<(usize, Vec<usize>)> = groups.into_iter().collect();
    group_vec.sort_by(|a, b| {
        b.1.len()
            .cmp(&a.1.len())
            .then_with(|| a.1.iter().min().cmp(&b.1.iter().min()))
    });

    let mut out = Vec::new();
    let mut next_id = 0;
    for (_, nodes) in group_vec {
        if nodes.len() < min_size {
            continue;
        }
        let node_set: HashSet<usize> = nodes.iter().copied().collect();

        // Internal edge weight — each intra-community undirected edge once.
        let mut internal = 0.0;
        for &i in &nodes {
            for &(j, w) in &graph.adjacency[i] {
                if j > i && node_set.contains(&j) {
                    internal += w;
                }
            }
        }

        let mut top: Vec<(String, usize)> = nodes
            .iter()
            .map(|&i| (graph.node_ids[i].clone(), graph.adjacency[i].len()))
            .collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        top.truncate(10);

        let members: Vec<String> = nodes.iter().map(|&i| graph.node_ids[i].clone()).collect();
        out.push(Community {
            id: next_id,
            size: members.len(),
            members,
            internal_edges: internal,
            top_nodes: top,
        });
        next_id += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::model::GraphBuilder;
    use super::*;

    #[test]
    fn assembles_two_communities_with_metadata() {
        let mut b = GraphBuilder::new();
        for (u, v) in [("0", "1"), ("1", "2"), ("2", "0")] {
            b.add_edge(u, v, 1.0);
        }
        for (u, v) in [("3", "4"), ("4", "5"), ("5", "3")] {
            b.add_edge(u, v, 1.0);
        }
        b.add_edge("2", "3", 1.0);
        let g = b.build();
        let comms = detect_communities(&g, 1.0, 42, 2);
        assert_eq!(comms.len(), 2);
        assert_eq!(comms[0].size, 3);
        // Each triangle has 3 internal edges.
        assert_eq!(comms[0].internal_edges, 3.0);
        assert!(!comms[0].top_nodes.is_empty());
    }

    #[test]
    fn drops_communities_below_min_size() {
        let mut b = GraphBuilder::new();
        b.add_edge("a", "b", 1.0); // size-2 component
        b.add_edge("c", "d", 1.0); // size-2 component
        let g = b.build();
        assert!(detect_communities(&g, 1.0, 1, 3).is_empty());
        assert_eq!(detect_communities(&g, 1.0, 1, 2).len(), 2);
    }
}
