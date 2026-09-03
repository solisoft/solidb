//! HTTP handlers for global GraphRAG: trigger a community-detection build and
//! query its status / results. The heavy build runs in a detached task so the
//! request returns immediately with a `request_id` to poll.

use super::system::AppState;
use crate::error::DbError;
use crate::graph::build::{run_build, BuildOptions};
use crate::sdbql::QueryExecutor;
use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

fn graph_executor<'a>(
    storage: &'a crate::storage::StorageEngine,
    db_name: &str,
) -> QueryExecutor<'a> {
    QueryExecutor::with_database(storage, db_name.to_string())
        .with_timeout(std::time::Duration::from_secs(30))
}

#[derive(Debug, Deserialize)]
pub struct NeighborsRequest {
    pub edge_collection: String,
    pub seeds: Value,
    #[serde(default)]
    pub options: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct GraphRagRequest {
    pub seed_collection: String,
    pub vector_index: String,
    pub edge_collection: String,
    pub query_vector: Vec<f32>,
    #[serde(default)]
    pub options: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct CommunitySearchRequest {
    pub query_text: String,
    #[serde(default)]
    pub options: Option<Value>,
}

/// `POST /_api/database/{db}/graph/neighbors`
pub async fn graph_neighbors(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<NeighborsRequest>,
) -> Result<Json<Value>, DbError> {
    // Expansion over an unindexed edge collection is a scan: keep it off
    // the async workers.
    let storage = state.storage.clone();
    let results = tokio::task::spawn_blocking(move || {
        graph_executor(&storage, &db_name).neighbors(&req.edge_collection, req.seeds, req.options)
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
    let count = results.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(Json(json!({ "results": results, "count": count })))
}

/// `POST /_api/database/{db}/graph/rag`
pub async fn graph_rag_search(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<GraphRagRequest>,
) -> Result<Json<Value>, DbError> {
    let executor = graph_executor(&state.storage, &db_name);
    let results = executor.graph_rag(
        &req.seed_collection,
        &req.vector_index,
        &req.edge_collection,
        json!(req.query_vector),
        req.options,
    )?;
    let count = results.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(Json(json!({ "results": results, "count": count })))
}

/// `POST /_api/database/{db}/graph/community/search`
pub async fn community_search(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<CommunitySearchRequest>,
) -> Result<Json<Value>, DbError> {
    let executor = graph_executor(&state.storage, &db_name);
    let results = executor.community_search(&req.query_text, req.options)?;
    let count = results.as_array().map(|a| a.len()).unwrap_or(0);
    Ok(Json(json!({ "results": results, "count": count })))
}

#[derive(Debug, Deserialize)]
pub struct BuildCommunitiesRequest {
    pub edge_collection: String,
    #[serde(default)]
    pub resolution: Option<f64>,
    #[serde(default)]
    pub min_community_size: Option<usize>,
    #[serde(default)]
    pub summarize: Option<bool>,
    #[serde(default)]
    pub max_communities: Option<usize>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
}

/// A build older than this with no terminal status is treated as abandoned
/// (its server died mid-run), so a stuck row can never wedge the endpoint.
const BUILD_STALE_AFTER: chrono::Duration = chrono::Duration::hours(1);

/// `POST /_api/database/{db}/graph/community/build`
pub async fn build_communities(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Json(req): Json<BuildCommunitiesRequest>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    // Fail fast if the edge collection doesn't exist.
    database.get_collection(&req.edge_collection)?;

    let request_id = uuid::Uuid::new_v4().to_string();
    let run_id = format!("run_{}", uuid::Uuid::new_v4().simple());
    let opts = BuildOptions {
        resolution: req.resolution.unwrap_or(1.0),
        min_community_size: req.min_community_size.unwrap_or(3),
        summarize: req.summarize.unwrap_or(false),
        max_communities: req.max_communities.unwrap_or(50),
        provider: req.provider.clone(),
        seed: req.seed.unwrap_or(42),
    };

    // Record a pending request so it can be polled.
    let requests = database.get_or_create_collection("_graph_build_requests")?;

    // Two builds racing on one edge collection would each purge the other's
    // output on completion, leaving `_graph_runs.latest_run_id` pointing at
    // deleted communities. Serialize them.
    if let Some(pending) = active_build_for(&requests, &req.edge_collection) {
        return Err(DbError::ConflictError(format!(
            "a community build for '{}' is already running (request_id {})",
            req.edge_collection, pending
        )));
    }

    let _ = requests.insert(json!({
        "_key": request_id,
        "status": "pending",
        "edge_collection": req.edge_collection,
        "run_id": run_id,
        "summarize": opts.summarize,
        "started_at": chrono::Utc::now().to_rfc3339(),
    }));

    // Run the build detached; update the request row on completion.
    let storage = state.storage.clone();
    let db = db_name.clone();
    let edge = req.edge_collection.clone();
    let rid = run_id.clone();
    let req_id = request_id.clone();
    tokio::spawn(async move {
        let result = run_build(storage.clone(), &db, &edge, &rid, &opts).await;
        if let Ok(database) = storage.get_database(&db) {
            if let Ok(requests) = database.get_or_create_collection("_graph_build_requests") {
                let update = match &result {
                    Ok(outcome) => json!({
                        "_key": req_id, "status": "done",
                        "edge_collection": edge, "run_id": rid,
                        "communities_found": outcome.communities_found,
                        "summarized": outcome.summarized,
                    }),
                    Err(e) => json!({
                        "_key": req_id, "status": "failed",
                        "edge_collection": edge, "run_id": rid,
                        "error": e.to_string(),
                    }),
                };
                let _ = requests.update(&req_id, update);
            }
        }
    });

    Ok(Json(json!({
        "request_id": request_id,
        "run_id": run_id,
        "status": "pending",
    })))
}

/// The request id of a still-running build for `edge_collection`, if any.
/// A `pending` row whose `started_at` is older than [`BUILD_STALE_AFTER`] — or
/// that predates the field — is ignored: its task cannot outlive its process.
fn active_build_for(
    requests: &crate::storage::Collection,
    edge_collection: &str,
) -> Option<String> {
    let cutoff = chrono::Utc::now() - BUILD_STALE_AFTER;
    requests.scan(None).into_iter().find_map(|d| {
        let v = d.to_value();
        let pending = v.get("status").and_then(|x| x.as_str()) == Some("pending");
        let same_edge = v.get("edge_collection").and_then(|x| x.as_str()) == Some(edge_collection);
        let fresh = v
            .get("started_at")
            .and_then(|x| x.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .is_some_and(|t| t.with_timezone(&chrono::Utc) > cutoff);
        (pending && same_edge && fresh).then(|| d.key.clone())
    })
}

/// `GET /_api/database/{db}/graph/community/build/{request_id}`
pub async fn build_status(
    State(state): State<AppState>,
    Path((db_name, request_id)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    // A status read must not create the collection it reads from.
    let requests = database.get_collection("_graph_build_requests")?;
    let doc = requests.get(&request_id)?;
    Ok(Json(doc.to_value()))
}

/// `GET /_api/database/{db}/graph/communities?run_id=&edge_collection=&limit=&offset=`
///
/// Community documents embed their full `members` array, so the listing is
/// paginated (default 100) rather than returning every community at once.
pub async fn list_communities(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let coll = match database.get_collection("_graph_communities") {
        Ok(c) => c,
        Err(_) => return Ok(Json(json!({ "communities": [], "total": 0 }))),
    };
    let run_id = params.get("run_id");
    let edge_collection = params.get("edge_collection");
    let limit: usize = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let offset: usize = params
        .get("offset")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let matching: Vec<Value> = coll
        .scan(None)
        .into_iter()
        .map(|doc| doc.to_value())
        .filter(|v| {
            let run_ok = run_id
                .is_none_or(|rid| v.get("run_id").and_then(|x| x.as_str()) == Some(rid.as_str()));
            let edge_ok = edge_collection.is_none_or(|ec| {
                v.get("edge_collection").and_then(|x| x.as_str()) == Some(ec.as_str())
            });
            run_ok && edge_ok
        })
        .collect();

    let total = matching.len();
    let items: Vec<Value> = matching.into_iter().skip(offset).take(limit).collect();
    Ok(Json(
        json!({ "communities": items, "total": total, "limit": limit, "offset": offset }),
    ))
}
