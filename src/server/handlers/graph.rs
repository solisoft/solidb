//! HTTP handlers for global GraphRAG: trigger a community-detection build and
//! query its status / results. The heavy build runs in a detached task so the
//! request returns immediately with a `request_id` to poll.

use super::system::AppState;
use crate::error::DbError;
use crate::graph::build::{run_build, BuildOptions};
use axum::{
    extract::{Path, Query, State},
    response::Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

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
    let _ = requests.insert(json!({
        "_key": request_id,
        "status": "pending",
        "edge_collection": req.edge_collection,
        "run_id": run_id,
        "summarize": opts.summarize,
    }));

    // Run the build detached; update the request row on completion.
    let storage = state.storage.clone();
    let db = db_name.clone();
    let edge = req.edge_collection.clone();
    let rid = run_id.clone();
    let req_id = request_id.clone();
    tokio::spawn(async move {
        let result = run_build(&storage, &db, &edge, &rid, &opts).await;
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

/// `GET /_api/database/{db}/graph/community/build/{request_id}`
pub async fn build_status(
    State(state): State<AppState>,
    Path((db_name, request_id)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let requests = database.get_or_create_collection("_graph_build_requests")?;
    let doc = requests.get(&request_id)?;
    Ok(Json(doc.to_value()))
}

/// `GET /_api/database/{db}/graph/communities?run_id=&edge_collection=`
pub async fn list_communities(
    State(state): State<AppState>,
    Path(db_name): Path<String>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let coll = match database.get_collection("_graph_communities") {
        Ok(c) => c,
        Err(_) => return Ok(Json(json!({ "communities": [] }))),
    };
    let run_id = params.get("run_id");
    let edge_collection = params.get("edge_collection");

    let mut items: Vec<Value> = Vec::new();
    for doc in coll.scan(None) {
        let v = doc.to_value();
        if let Some(rid) = run_id {
            if v.get("run_id").and_then(|x| x.as_str()) != Some(rid.as_str()) {
                continue;
            }
        }
        if let Some(ec) = edge_collection {
            if v.get("edge_collection").and_then(|x| x.as_str()) != Some(ec.as_str()) {
                continue;
            }
        }
        items.push(v);
    }
    Ok(Json(json!({ "communities": items })))
}
