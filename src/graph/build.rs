//! Orchestrate a community-detection run and persist its output.
//!
//! Scans an edge collection → builds the graph → detects communities →
//! writes `_graph_communities`, optionally `_community_summaries` (with a
//! fulltext index for `COMMUNITY_SEARCH`), and records the run in `_graph_runs`.
//!
//! Detection is synchronous and CPU-bound (a full edge scan, then Louvain), so
//! it runs on the blocking pool; only summarization awaits (the LLM call).
//! A completed run purges its predecessors for the same edge collection, which
//! keeps the derived collections bounded and stops `COMMUNITY_SEARCH` from
//! blending results across runs.

use super::community::{detect_communities, Community};
use super::model::GraphBuilder;
use super::summarize::{keyword_summary, llm_summary};
use crate::error::{DbError, DbResult};
use crate::server::llm_client::LLMClient;
use crate::storage::{Database, StorageEngine};
use serde_json::{json, Value};
use std::sync::Arc;

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
    storage: Arc<StorageEngine>,
    db_name: &str,
    edge_collection: &str,
    run_id: &str,
    opts: &BuildOptions,
) -> DbResult<BuildOutcome> {
    let database = storage.get_database(db_name)?;
    database.get_collection(edge_collection)?; // fail fast

    // 1-3. Scan, build the graph, run Louvain, persist the communities. All
    // synchronous and CPU-bound, so it goes to the blocking pool rather than
    // stalling a Tokio worker for the duration of the scan.
    let communities = {
        let storage = storage.clone();
        let db_name = db_name.to_string();
        let edge_collection = edge_collection.to_string();
        let run_id = run_id.to_string();
        let (resolution, seed, min_size) = (opts.resolution, opts.seed, opts.min_community_size);
        tokio::task::spawn_blocking(move || {
            detect_and_persist(
                &storage,
                &db_name,
                &edge_collection,
                &run_id,
                resolution,
                seed,
                min_size,
            )
        })
        .await
        .map_err(|e| DbError::InternalError(format!("community build task failed: {}", e)))??
    };

    // 4. Optional summaries.
    let mut summarized = 0usize;
    if opts.summarize && !communities.is_empty() {
        let summ_coll = database.get_or_create_collection("_community_summaries")?;
        // The LLM client is optional — fall back to keyword summaries if it
        // can't be constructed (no provider/keys configured in _env).
        let client =
            LLMClient::from_storage(&storage, db_name, opts.provider.as_deref(), None).ok();

        for c in communities.iter().take(opts.max_communities) {
            let member_docs = fetch_member_docs(&storage, db_name, &c.members, 30);
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
            if summ_coll.insert(doc).is_ok() {
                summarized += 1;
            }
        }

        // Index summaries for COMMUNITY_SEARCH fulltext retrieval.
        if summarized > 0
            && summ_coll
                .get_fulltext_index("_community_summaries_ft")
                .is_none()
        {
            let _ = summ_coll.create_fulltext_index(
                "_community_summaries_ft".to_string(),
                vec!["summary".to_string(), "title".to_string()],
                Some(3),
            );
        }
    }

    // 5. Retire the previous run's output, now that this run's is queryable.
    purge_previous_runs(&database, edge_collection, run_id);

    // 6. Record the run (upsert by edge collection name).
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

/// Phases 1-3, synchronous: scan `edge_collection`, build the undirected graph,
/// detect communities, and write them to `_graph_communities`.
fn detect_and_persist(
    storage: &StorageEngine,
    db_name: &str,
    edge_collection: &str,
    run_id: &str,
    resolution: f64,
    seed: u64,
    min_community_size: usize,
) -> DbResult<Vec<Community>> {
    let database = storage.get_database(db_name)?;
    let edge = database.get_collection(edge_collection)?;

    let mut builder = GraphBuilder::new();
    for doc in edge.scan(None) {
        if let (Some(f), Some(t)) = (doc.get("_from"), doc.get("_to")) {
            if let (Some(f), Some(t)) = (f.as_str(), t.as_str()) {
                builder.add_edge(f, t, 1.0);
            }
        }
    }
    let graph = builder.build();

    let communities = detect_communities(&graph, resolution, seed, min_community_size);

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

    Ok(communities)
}

/// Delete the output of every run for `edge_collection` other than
/// `keep_run_id`. Without this, `_graph_communities` / `_community_summaries`
/// grow with every build and `COMMUNITY_SEARCH` — which only filters by run
/// when given a `run_id` or an `edge_collection` — returns one near-duplicate
/// per historical run.
fn purge_previous_runs(database: &Database, edge_collection: &str, keep_run_id: &str) {
    for name in ["_graph_communities", "_community_summaries"] {
        let Ok(coll) = database.get_collection(name) else {
            continue;
        };
        let stale: Vec<String> = coll
            .scan(None)
            .into_iter()
            .filter(|d| {
                let belongs = d
                    .get("edge_collection")
                    .is_some_and(|v| v.as_str() == Some(edge_collection));
                let older = d
                    .get("run_id")
                    .is_none_or(|v| v.as_str() != Some(keep_run_id));
                belongs && older
            })
            .map(|d| d.key)
            .collect();
        for key in stale {
            let _ = coll.delete(&key);
        }
    }
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
