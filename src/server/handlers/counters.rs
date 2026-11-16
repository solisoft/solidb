//! Counter fields of a collection: see `storage::counters`.

use super::system::AppState;
use crate::error::DbError;
use crate::storage::collection::counters::CounterSpec;
use crate::storage::counters::CounterRecord;
use crate::sync::counters;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct CountersResponse {
    pub collection: String,
    pub counters: Vec<CounterRecord>,
}

#[derive(Debug, Deserialize)]
pub struct CounterNextRequest {
    pub next: u64,
}

#[derive(Debug, Deserialize)]
pub struct CounterNodeRequest {
    /// The node that numbers from now on; null for whichever node receives
    /// the insert (an instance that runs alone).
    pub node: Option<String>,
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, DbError> + Send + 'static,
) -> Result<T, DbError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))?
}

/// The counters of a collection, each with where it stands.
pub async fn list_counters(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<CountersResponse>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    Ok(Json(CountersResponse {
        counters: blocking(move || collection.counter_records()).await?,
        collection: coll_name,
    }))
}

/// Declare a counter. Refused (409) when stored documents repeat a value
/// of the field or hold its first number.
pub async fn create_counter(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<CounterSpec>,
) -> Result<(StatusCode, Json<CounterRecord>), DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    let log = state.replication_log.clone();
    let record =
        blocking(move || counters::declare(&collection, log.as_deref(), &db_name, &coll_name, req))
            .await?;
    Ok((StatusCode::CREATED, Json(record)))
}

/// Stop numbering a field. The field and its unique index stay.
pub async fn delete_counter(
    State(state): State<AppState>,
    Path((db_name, coll_name, field)): Path<(String, String, String)>,
) -> Result<StatusCode, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    let log = state.replication_log.clone();
    blocking(move || counters::drop(&collection, log.as_deref(), &db_name, &coll_name, &field))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Set the number the next insert gets.
pub async fn set_counter_next(
    State(state): State<AppState>,
    Path((db_name, coll_name, field)): Path<(String, String, String)>,
    Json(req): Json<CounterNextRequest>,
) -> Result<Json<CounterRecord>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    let log = state.replication_log.clone();
    let record = blocking(move || {
        counters::set_next(
            &collection,
            log.as_deref(),
            &db_name,
            &coll_name,
            &field,
            req.next,
        )
    })
    .await?;
    Ok(Json(record))
}

/// Hand the numbering to another node. Asked of the node that numbers.
pub async fn set_counter_node(
    State(state): State<AppState>,
    Path((db_name, coll_name, field)): Path<(String, String, String)>,
    Json(req): Json<CounterNodeRequest>,
) -> Result<Json<CounterRecord>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    let log = state.replication_log.clone();
    let record = blocking(move || {
        counters::move_to(
            &collection,
            log.as_deref(),
            &db_name,
            &coll_name,
            &field,
            req.node,
        )
    })
    .await?;
    Ok(Json(record))
}
