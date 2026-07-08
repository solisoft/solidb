//! Global GraphRAG: community detection over an edge collection plus optional
//! LLM-generated community summaries.
//!
//! - [`model`] builds an undirected weighted graph from an edge scan.
//! - [`louvain`] is a pure, seeded community-detection implementation.
//! - [`community`] assembles labelled communities from the Louvain output.
//! - [`summarize`] turns a community into a title/summary/keywords, via a
//!   deterministic keyword fallback or an injected [`crate::server::llm_client::LLMClient`].
//! - [`build`] orchestrates a full run and persists it to `_`-prefixed
//!   collections (`_graph_communities`, `_community_summaries`, `_graph_runs`).

pub mod build;
pub mod community;
pub mod louvain;
pub mod model;
pub mod summarize;

pub use community::{detect_communities, Community};
pub use model::{Graph, GraphBuilder};
