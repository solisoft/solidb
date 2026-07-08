//! Orchestrate a community-detection run and persist its output.
//!
//! Scans an edge collection → builds the graph → detects communities →
//! writes `_graph_communities`, optionally `_community_summaries` (with a
//! fulltext index for `COMMUNITY_SEARCH`), and records the run in `_graph_runs`.

use super::community::detect_communities;
use super::model::GraphBuilder;
use super::summarize::{keyword_summary, llm_summary};
use crate::error::DbResult;
use crate::server::llm_client::LLMClient;
use crate::storage::StorageEngine;
use serde_json::{json, Value};

pub struct BuildOptions {
    pub resolution: f64,
    pub min_community_size: usize,
    pub summarize: bool,
    pub max_communities: usize,
    pub provider: Option<String>,
    pub seed: u64,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            resolution: 1.0,
            min_community_size: 3,
            summarize: false,
            max_communities: 50,
            provider: None,
            seed: 42,
        }
    }
}

pub struct BuildOutcome {
    pub communities_found: usize,
    pub summarized: usize,
}

/// Run a full build for `edge_collection` in `db_name`, tagging all output with
/// `run_id`. Individual document writes are best-effort (a single failed insert
/// does not abort the run).
pub async fn run_build(
    storage: &StorageEngine,
    db_name: &str,
    edge_collection: &str,
    run_id: &str,
    opts: &BuildOptions,
) -> DbResult<BuildOutcome> {
    let database = storage.get_database(db_name)?;
    let edge = database.get_collection(edge_collection)?;

    // 1. Build the undirected graph from _from/_to.
    let mut builder = GraphBuilder::new();
    for doc in edge.scan(None) {
        let v = doc.to_value();
        if let (Some(f), Some(t)) = (
            v.get("_from").and_then(|x| x.as_str()),
            v.get("_to").and_then(|x| x.as_str()),
        ) {
            builder.add_edge(f, t, 1.0);
        }
    }
    let graph = builder.build();

    // 2. Detect communities.
    let communities =
        detect_communities(&graph, opts.resolution, opts.seed, opts.min_community_size);

    // 3. Persist communities.
    let comm_coll = database.get_or_create_collection("_graph_communities")?;
    for c in &communities {
        let doc = json!({
            "_key": format!("{}:{}", run_id, c.id),
            "run_id": run_id,
            "edge_collection": edge_collection,
            "community_id": c.id,
            "size": c.size,
            "internal_edges": c.internal_edges,
            "members": c.members,
            "top_nodes": c.top_nodes.iter()
                .map(|(id, deg)| json!({ "id": id, "degree": deg }))
                .collect::<Vec<_>>(),
        });
        let _ = comm_coll.insert(doc);
    }

    // 4. Optional summaries.
    let mut summarized = 0usize;
    if opts.summarize && !communities.is_empty() {
        let summ_coll = database.get_or_create_collection("_community_summaries")?;
        // The LLM client is optional — fall back to keyword summaries if it
        // can't be constructed (no provider/keys configured in _env).
        let client = LLMClient::from_storage(storage, db_name, opts.provider.as_deref()).ok();

        for c in communities.iter().take(opts.max_communities) {
            let member_docs = fetch_member_docs(storage, db_name, &c.members, 30);
            let summary = match &client {
                Some(cl) => llm_summary(cl, c, &member_docs)
                    .await
                    .unwrap_or_else(|_| keyword_summary(c, &member_docs)),
                None => keyword_summary(c, &member_docs),
            };
            let doc = json!({
                "_key": format!("{}:{}", run_id, c.id),
                "run_id": run_id,
                "edge_collection": edge_collection,
                "community_id": c.id,
                "title": summary.title,
                "summary": summary.summary,
                "keywords": summary.keywords,
                "size": c.size,
                "member_sample": c.members.iter().take(10).cloned().collect::<Vec<_>>(),
            });
            let _ = summ_coll.insert(doc);
            summarized += 1;
        }
    }

    // 5. Record the run (upsert by edge collection name).
    let runs = database.get_or_create_collection("_graph_runs")?;
    let run_doc = json!({
        "_key": edge_collection,
        "latest_run_id": run_id,
        "communities_found": communities.len(),
    });
    if runs.get(edge_collection).is_ok() {
        let _ = runs.update(edge_collection, run_doc);
    } else {
        let _ = runs.insert(run_doc);
    }

    Ok(BuildOutcome {
        communities_found: communities.len(),
        summarized,
    })
}

/// Fetch up to `limit` member documents by their `"coll/key"` ids.
fn fetch_member_docs(
    storage: &StorageEngine,
    db_name: &str,
    members: &[String],
    limit: usize,
) -> Vec<Value> {
    let database = match storage.get_database(db_name) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for m in members.iter().take(limit) {
        if let Some((coll, key)) = m.split_once('/') {
            if let Ok(c) = database.get_collection(coll) {
                if let Ok(doc) = c.get(key) {
                    out.push(doc.to_value());
                }
            }
        }
    }
    out
}
