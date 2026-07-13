//! PageRank implementation over the in-memory Graph model.
//!
//! Uses the power iteration method. Works on the undirected Graph for simplicity
//! (common for many graph analytics use cases in document DBs).
//! Also provides a directed variant for SDBQL PAGERANK on edge collections.

use super::model::Graph;
use std::collections::{HashMap, HashSet};

/// Compute PageRank scores for all nodes.
/// Returns parallel vec of scores (same order as graph.node_ids).
pub fn pagerank(graph: &Graph, damping: f64, iterations: usize, tol: f64) -> Vec<f64> {
    let n = graph.node_count();
    if n == 0 {
        return vec![];
    }

    let mut rank = vec![1.0 / n as f64; n];
    let mut new_rank = vec![0.0; n];

    let damping_factor = damping;
    let base = (1.0 - damping_factor) / n as f64;

    for _ in 0..iterations {
        new_rank.fill(base);

        for (i, neighbors) in graph.adjacency.iter().enumerate() {
            if neighbors.is_empty() {
                // dangling node: distribute uniformly (simplified)
                let share = rank[i] / n as f64;
                for nr in new_rank.iter_mut() {
                    *nr += damping_factor * share;
                }
                continue;
            }

            let out_weight: f64 = neighbors.iter().map(|&(_, w)| w).sum();
            if out_weight <= 0.0 {
                continue;
            }

            for &(j, w) in neighbors {
                new_rank[j] += damping_factor * (rank[i] * (w / out_weight));
            }
        }

        // Check convergence
        let mut diff = 0.0;
        for (r, nr) in rank.iter_mut().zip(new_rank.iter()) {
            diff += (*nr - *r).abs();
            *r = *nr;
        }

        if diff < tol {
            break;
        }
    }

    rank
}

/// Convenience: return (node_id, score) pairs sorted by score desc.
pub fn pagerank_scored(graph: &Graph, damping: f64, iterations: usize) -> Vec<(String, f64)> {
    let scores = pagerank(graph, damping, iterations, 1e-6);
    let mut pairs: Vec<_> = graph
        .node_ids
        .iter()
        .zip(scores)
        .map(|(id, s)| (id.clone(), s))
        .collect();

    pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    pairs
}

/// Compute directed PageRank scores given a map of out-neighbors (node -> vec of (target, weight)).
/// Nodes without outgoing edges are treated as dangling.
/// Returns vec of (node, score) sorted by score descending.
pub fn pagerank_directed(
    out_neighbors: &HashMap<String, Vec<(String, f64)>>,
    damping: f64,
    iterations: usize,
    tol: f64,
) -> Vec<(String, f64)> {
    let mut node_set: HashSet<String> = out_neighbors.keys().cloned().collect();
    for targets in out_neighbors.values() {
        for (t, _) in targets {
            node_set.insert(t.clone());
        }
    }
    let node_list: Vec<String> = node_set.into_iter().collect();
    let n = node_list.len();
    if n == 0 {
        return vec![];
    }

    let idx: HashMap<String, usize> = node_list
        .iter()
        .enumerate()
        .map(|(i, k)| (k.clone(), i))
        .collect();

    let mut rank = vec![1.0 / n as f64; n];
    let mut new_rank = vec![0.0; n];
    let damping_factor = damping;
    let base = (1.0 - damping_factor) / n as f64;

    for _ in 0..iterations {
        // Nodes with no outgoing edges are "dangling" and their rank must be
        // redistributed uniformly across the whole graph each iteration.
        // This crucially includes nodes that only ever appear as an edge target
        // (sinks): they are absent from `out_neighbors` entirely, so unless we
        // account for them here their rank mass leaks out and the scores drift
        // below a valid probability distribution.
        let mut dangling_mass = 0.0;
        for (i, node) in node_list.iter().enumerate() {
            let has_out = out_neighbors
                .get(node)
                .map(|tos| tos.iter().map(|(_, w)| *w).sum::<f64>() > 0.0)
                .unwrap_or(false);
            if !has_out {
                dangling_mass += rank[i];
            }
        }
        let dangling_share = damping_factor * dangling_mass / n as f64;
        new_rank.fill(base + dangling_share);

        for (i, node) in node_list.iter().enumerate() {
            if let Some(tos) = out_neighbors.get(node) {
                let out_weight: f64 = tos.iter().map(|(_, w)| *w).sum();
                if out_weight > 0.0 {
                    for (to, w) in tos {
                        let contrib = rank[i] * damping_factor * (w / out_weight);
                        if let Some(&j) = idx.get(to) {
                            new_rank[j] += contrib;
                        }
                    }
                }
                // out_weight <= 0.0 is handled as dangling above.
            }
        }

        let mut diff = 0.0;
        for (r, nr) in rank.iter_mut().zip(new_rank.iter()) {
            diff += (*nr - *r).abs();
            *r = *nr;
        }

        if diff < tol {
            break;
        }
    }

    let mut pairs: Vec<_> = node_list.into_iter().zip(rank).collect();
    pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::model::GraphBuilder;

    #[test]
    fn test_pagerank_basic() {
        let mut b = GraphBuilder::new();
        b.add_edge("a", "b", 1.0);
        b.add_edge("b", "c", 1.0);
        b.add_edge("c", "a", 1.0);
        let g = b.build();

        let scores = pagerank(&g, 0.85, 20, 1e-5);
        assert_eq!(scores.len(), 3);
        // All should be roughly equal in a cycle
        let avg = scores.iter().sum::<f64>() / 3.0;
        for s in &scores {
            assert!((s - avg).abs() < 0.1);
        }
    }

    #[test]
    fn test_pagerank_directed() {
        // Directed chain: a -> b -> c
        let mut out: std::collections::HashMap<String, Vec<(String, f64)>> =
            std::collections::HashMap::new();
        out.insert("a".to_string(), vec![("b".to_string(), 1.0)]);
        out.insert("b".to_string(), vec![("c".to_string(), 1.0)]);

        let scored = pagerank_directed(&out, 0.85, 20, 1e-5);
        assert_eq!(scored.len(), 3);
        // c should have highest score in this flow
        let scores_map: std::collections::HashMap<_, _> = scored.into_iter().collect();
        assert!(scores_map["c"] > scores_map["b"]);
        assert!(
            scores_map["b"] > scores_map["a"] || (scores_map["b"] - scores_map["a"]).abs() < 0.01
        );
    }

    #[test]
    fn test_pagerank_directed_conserves_rank_with_sinks() {
        // `c` only ever appears as a target (a sink absent from out_neighbors), and
        // `b` also has no outgoing edge here. Their rank mass must be redistributed,
        // not leaked, so the scores stay a valid distribution summing to ~1.0.
        let mut out: std::collections::HashMap<String, Vec<(String, f64)>> =
            std::collections::HashMap::new();
        out.insert(
            "a".to_string(),
            vec![("b".to_string(), 1.0), ("c".to_string(), 1.0)],
        );
        out.insert("b".to_string(), vec![("c".to_string(), 1.0)]);
        // "c" is a pure sink: never a key in out_neighbors.

        let scored = pagerank_directed(&out, 0.85, 100, 1e-9);
        assert_eq!(scored.len(), 3);
        let total: f64 = scored.iter().map(|(_, s)| *s).sum();
        assert!(
            (total - 1.0).abs() < 1e-6,
            "rank mass leaked: total = {} (expected ~1.0)",
            total
        );
    }
}
