//! Foreign keys of a collection: see `storage::foreign_keys`.

use super::system::AppState;
use crate::error::DbError;
use crate::storage::collection::references::{ForeignKeySpec, Orphans, ReferencedBy};
use crate::storage::foreign_keys::ForeignKey;
use crate::sync::log::LogEntry;
use crate::sync::protocol::Operation;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ForeignKeysResponse {
    pub collection: String,
    pub foreign_keys: Vec<ForeignKey>,
    pub referenced_by: Vec<ReferencedBy>,
}

fn log_definition(
    state: &AppState,
    database: &str,
    collection: &str,
    operation: Operation,
    name: &str,
    data: Option<Vec<u8>>,
) {
    if let Some(ref log) = state.replication_log {
        let _ = log.append(LogEntry::new_op(
            database, collection, operation, name, data,
        ));
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, DbError> + Send + 'static,
) -> Result<T, DbError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))?
}

/// The collection a request names, or why it cannot hold a foreign key.
fn child(
    state: &AppState,
    database: &str,
    collection: &str,
) -> Result<crate::storage::Collection, DbError> {
    let db = state.storage.get_database(database)?;
    db.get_collection(collection).map_err(|e| {
        if db.is_columnar_collection(collection) {
            crate::storage::foreign_keys::columnar(collection)
        } else {
            e
        }
    })
}

/// The constraints declared on a collection, and those that refer to it.
pub async fn list_foreign_keys(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<ForeignKeysResponse>, DbError> {
    let collection = child(&state, &db_name, &coll_name)?;
    Ok(Json(ForeignKeysResponse {
        foreign_keys: collection.foreign_keys(),
        referenced_by: collection.referenced_by(),
        collection: coll_name,
    }))
}

/// Declare a constraint. Refused (409) when stored documents break it.
///
/// `references` as a list (`["people", "firms"]`) declares one key per
/// collection on the same field, by `_id`: an edge end that links several
/// vertex collections. All or none; the answer is then
/// `{"foreign_keys": [...]}`.
pub async fn create_foreign_key(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(mut req): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), DbError> {
    let collection = child(&state, &db_name, &coll_name)?;
    if let Some(parents) = req.get("references").and_then(|r| r.as_array()).cloned() {
        let parents: Vec<String> = parents
            .iter()
            .map(|p| p.as_str().map(str::to_string))
            .collect::<Option<_>>()
            .ok_or_else(|| {
                DbError::BadRequest("'references' is a collection, or a list of them".to_string())
            })?;
        req["references"] = serde_json::Value::String(String::new());
        let spec: ForeignKeySpec = serde_json::from_value(req)
            .map_err(|e| DbError::BadRequest(format!("Invalid foreign key: {}", e)))?;
        if spec.name.is_some() || spec.by == Some(crate::storage::foreign_keys::ReferenceBy::Key) {
            return Err(DbError::BadRequest(
                "A list of collections is declared by _id, and its keys are named after them"
                    .to_string(),
            ));
        }
        let keys =
            blocking(move || collection.add_foreign_keys_to(&spec.field, &parents, spec.on_delete))
                .await?;
        for key in &keys {
            log_definition(
                &state,
                &db_name,
                &coll_name,
                Operation::CreateForeignKey,
                &key.name,
                serde_json::to_vec(key).ok(),
            );
        }
        return Ok((
            StatusCode::CREATED,
            Json(serde_json::json!({ "foreign_keys": keys })),
        ));
    }
    let req: ForeignKeySpec = serde_json::from_value(req)
        .map_err(|e| DbError::BadRequest(format!("Invalid foreign key: {}", e)))?;
    let key = blocking(move || collection.add_foreign_key(req)).await?;
    // Peers store the definition as declared here; the writes its deletes
    // cascade into reach them as entries of their own (`observe_cascades`).
    log_definition(
        &state,
        &db_name,
        &coll_name,
        Operation::CreateForeignKey,
        &key.name,
        serde_json::to_vec(&key).ok(),
    );
    let answer = serde_json::to_value(&key)
        .map_err(|e| DbError::InternalError(format!("Failed to write the key: {}", e)))?;
    Ok((StatusCode::CREATED, Json(answer)))
}

/// Both ends of an edge collection: the collections its `_from` and its
/// `_to` name a document of.
#[derive(Debug, serde::Deserialize)]
pub struct EdgeEndsRequest {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub on_delete: crate::storage::foreign_keys::OnDelete,
}

#[derive(Debug, Serialize)]
pub struct EdgeEndsResponse {
    pub from: ForeignKey,
    pub to: ForeignKey,
}

/// Declare the keys of both ends of an edge collection in one call: both
/// are declared, or neither.
pub async fn create_edge_end_keys(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<EdgeEndsRequest>,
) -> Result<(StatusCode, Json<EdgeEndsResponse>), DbError> {
    let collection = child(&state, &db_name, &coll_name)?;
    let (from, to) =
        blocking(move || collection.add_edge_end_keys(&req.from, &req.to, req.on_delete)).await?;
    for key in [&from, &to] {
        log_definition(
            &state,
            &db_name,
            &coll_name,
            Operation::CreateForeignKey,
            &key.name,
            serde_json::to_vec(key).ok(),
        );
    }
    Ok((StatusCode::CREATED, Json(EdgeEndsResponse { from, to })))
}

/// Remove a constraint. Its index stays.
pub async fn delete_foreign_key(
    State(state): State<AppState>,
    Path((db_name, coll_name, name)): Path<(String, String, String)>,
) -> Result<StatusCode, DbError> {
    let collection = child(&state, &db_name, &coll_name)?;
    let dropped = name.clone();
    blocking(move || collection.drop_foreign_key(&dropped)).await?;
    log_definition(
        &state,
        &db_name,
        &coll_name,
        Operation::DropForeignKey,
        &name,
        None,
    );
    Ok(StatusCode::NO_CONTENT)
}

/// The documents a constraint would be refused for, without declaring it.
pub async fn check_foreign_key(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<ForeignKeySpec>,
) -> Result<Json<Orphans>, DbError> {
    let collection = child(&state, &db_name, &coll_name)?;
    let orphans = blocking(move || collection.foreign_key_orphans(&req)).await?;
    Ok(Json(orphans))
}
