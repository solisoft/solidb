//! Column blocks of a time series collection: see `storage::blocks`.

use super::system::AppState;
use crate::error::DbError;
use crate::storage::blocks::{BlocksDef, BlocksStatus};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct SealedResponse {
    /// Blocks added by this call.
    pub sealed: usize,
    #[serde(flatten)]
    pub status: BlocksStatus,
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, DbError> + Send + 'static,
) -> Result<T, DbError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))?
}

fn undeclared(collection: &str) -> DbError {
    DbError::BadRequest(format!("'{collection}' keeps no column blocks"))
}

/// The fields a collection keeps in blocks, and how much of it they cover.
pub async fn get_column_blocks(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<BlocksStatus>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    blocking(move || {
        collection
            .column_blocks_status()
            .ok_or_else(|| undeclared(&coll_name))
    })
    .await
    .map(Json)
}

/// Declare, or replace, the fields kept in blocks. Blocks of an earlier
/// definition are deleted; new ones are sealed as documents arrive, or at
/// once by `POST …/blocks/seal`.
pub async fn set_column_blocks(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(def): Json<BlocksDef>,
) -> Result<Json<BlocksStatus>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    blocking(move || {
        collection.set_column_blocks(def)?;
        collection
            .column_blocks_status()
            .ok_or_else(|| undeclared(&coll_name))
    })
    .await
    .map(Json)
}

/// Stop keeping blocks and delete them. The documents are untouched.
pub async fn delete_column_blocks(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<StatusCode, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    blocking(move || collection.drop_column_blocks()).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Seal every document no block covers yet, the newest included.
pub async fn seal_column_blocks(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<SealedResponse>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    blocking(move || {
        if collection.column_blocks_definition().is_none() {
            return Err(undeclared(&coll_name));
        }
        let sealed = collection.seal_column_blocks()?;
        let status = collection
            .column_blocks_status()
            .ok_or_else(|| undeclared(&coll_name))?;
        Ok(SealedResponse { sealed, status })
    })
    .await
    .map(Json)
}
