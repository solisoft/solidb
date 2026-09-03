//! Driver handlers for Graph RAG operations.

use super::DriverHandler;
use crate::driver::protocol::{DriverError, Response};
use crate::sdbql::QueryExecutor;
use serde_json::{json, Value};

fn graph_executor<'a>(handler: &'a DriverHandler, database: &str) -> QueryExecutor<'a> {
    QueryExecutor::with_database(&handler.storage, database.to_string())
        .with_timeout(std::time::Duration::from_secs(30))
}

pub fn handle_graph_neighbors(
    handler: &DriverHandler,
    database: String,
    edge_collection: String,
    seeds: Value,
    options: Option<Value>,
) -> Response {
    match graph_executor(handler, &database).neighbors(&edge_collection, seeds, options) {
        Ok(results) => {
            let count = results.as_array().map(|a| a.len()).unwrap_or(0);
            Response::ok(json!({ "results": results, "count": count }))
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_graph_rag(
    handler: &DriverHandler,
    database: String,
    seed_collection: String,
    vector_index: String,
    edge_collection: String,
    query_vector: Vec<f32>,
    options: Option<Value>,
) -> Response {
    match graph_executor(handler, &database).graph_rag(
        &seed_collection,
        &vector_index,
        &edge_collection,
        json!(query_vector),
        options,
    ) {
        Ok(results) => {
            let count = results.as_array().map(|a| a.len()).unwrap_or(0);
            Response::ok(json!({ "results": results, "count": count }))
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_community_search(
    handler: &DriverHandler,
    database: String,
    query_text: String,
    options: Option<Value>,
) -> Response {
    match graph_executor(handler, &database).community_search(&query_text, options) {
        Ok(results) => {
            let count = results.as_array().map(|a| a.len()).unwrap_or(0);
            Response::ok(json!({ "results": results, "count": count }))
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}
