//! Foreign keys of a collection: see `storage::foreign_keys`.

use super::system::AppState;
use crate::error::DbError;
use crate::storage::collection::references::{Orphans, ReferencedBy};
use crate::storage::foreign_keys::{ForeignKey, OnDelete};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateForeignKeyRequest {
    /// The field of this collection that holds the parent's `_key`.
    pub field: String,
    /// The parent collection, in the same database.
    pub references: String,
    /// `restrict` (the default), `cascade` or `set_null`.
    #[serde(default)]
    pub on_delete: OnDelete,
    /// Defaults to `fk_<field>`.
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CheckForeignKeyRequest {
    pub field: String,
    pub references: String,
}

#[derive(Debug, Serialize)]
pub struct ForeignKeysResponse {
    pub collection: String,
    pub foreign_keys: Vec<ForeignKey>,
    pub referenced_by: Vec<ReferencedBy>,
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, DbError> + Send + 'static,
) -> Result<T, DbError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))?
}

/// The constraints declared on a collection, and those that refer to it.
pub async fn list_foreign_keys(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<ForeignKeysResponse>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    Ok(Json(ForeignKeysResponse {
        foreign_keys: collection.foreign_keys(),
        referenced_by: collection.referenced_by(),
        collection: coll_name,
    }))
}

/// Declare a constraint. Refused (409) when stored documents break it.
pub async fn create_foreign_key(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<CreateForeignKeyRequest>,
) -> Result<(StatusCode, Json<ForeignKey>), DbError> {
    // A cascade is written by storage and never reaches the replication
    // log: whoever reads it (a peer, an offline-sync client) would keep
    // the children. The log is active exactly when it has had a reader.
    if state
        .replication_log
        .as_ref()
        .is_some_and(|log| log.is_active())
    {
        return Err(DbError::OperationNotSupported(
            "Foreign keys are not supported on a node that replicates (its replication log is active)"
                .to_string(),
        ));
    }
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    let key = blocking(move || {
        collection.add_foreign_key(req.name, &req.field, &req.references, req.on_delete)
    })
    .await?;
    Ok((StatusCode::CREATED, Json(key)))
}

/// Remove a constraint. Its index stays.
pub async fn delete_foreign_key(
    State(state): State<AppState>,
    Path((db_name, coll_name, name)): Path<(String, String, String)>,
) -> Result<StatusCode, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    blocking(move || collection.drop_foreign_key(&name)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The documents a constraint would be refused for, without declaring it.
pub async fn check_foreign_key(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<CheckForeignKeyRequest>,
) -> Result<Json<Orphans>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    let orphans =
        blocking(move || collection.foreign_key_orphans(&req.field, &req.references)).await?;
    Ok(Json(orphans))
}
