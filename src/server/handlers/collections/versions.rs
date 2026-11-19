//! A versioned collection's history over HTTP (card #1001): restore it to
//! an instant, and name instants. Every change is logged for the other
//! nodes: a restore as the document writes it made, a name as
//! `Operation::SetVersionTag`.

use super::super::system::AppState;
use crate::error::DbError;
use crate::storage::Collection;
use crate::sync::log::LogEntry;
use crate::sync::protocol::Operation;
use axum::{
    extract::{Path, State},
    response::Json,
};
use serde::Deserialize;
use serde_json::{json, Value};

/// `POST …/restore`: the instant as `as_of` (epoch millis, RFC3339, or
/// `{"micros": n}`) or `tag` (a named instant), and optionally the `keys`
/// to restore (all the collection's when absent).
#[derive(Debug, Deserialize)]
pub struct RestoreRequest {
    #[serde(default, rename = "as_of", alias = "asOf")]
    pub as_of: Option<Value>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub keys: Option<Vec<String>>,
}

/// `POST …/version-tags`: a name, and the instant (epoch millis, RFC3339
/// or `{"micros": n}`); now when absent.
#[derive(Debug, Deserialize)]
pub struct VersionTagRequest {
    pub name: String,
    #[serde(default, rename = "as_of", alias = "asOf")]
    pub as_of: Option<Value>,
}

fn versioned(state: &AppState, db: &str, name: &str) -> Result<Collection, DbError> {
    let collection = state.storage.get_database(db)?.get_collection(name)?;
    if collection
        .get_shard_config()
        .is_some_and(|c| c.num_shards > 0)
    {
        // Its history is read across its shards (card #1002), but a restore
        // or a name would have to be made on each of them.
        return Err(DbError::OperationNotSupported(format!(
            "'{name}' is sharded: restore and version tags are not supported there yet"
        )));
    }
    Ok(collection)
}

fn instant(state: &Collection, as_of: Option<&Value>, tag: Option<&str>) -> Result<u64, DbError> {
    match (as_of, tag) {
        (Some(v), None) => crate::sdbql::executor::as_of_micros(v),
        (None, Some(tag)) => state
            .version_tag(tag)
            .ok_or_else(|| DbError::BadRequest(format!("no version tag '{tag}'"))),
        _ => Err(DbError::BadRequest(
            "give either `as_of` or `tag`".to_string(),
        )),
    }
}

/// POST /_api/database/{db}/collection/{name}/restore
pub async fn restore_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<RestoreRequest>,
) -> Result<Json<Value>, DbError> {
    let collection = versioned(&state, &db_name, &coll_name)?;
    let at = instant(&collection, req.as_of.as_ref(), req.tag.as_deref())?;
    let log = state.replication_log.clone();
    let (db, name) = (db_name.clone(), coll_name.clone());
    let restored = tokio::task::spawn_blocking(move || {
        let restored = collection.restore_as_of(at, req.keys.as_deref())?;
        if let Some(log) = log.as_deref() {
            let now = chrono::Utc::now().timestamp_millis() as u64;
            let entry = |operation, key: &str, data: Option<Vec<u8>>| LogEntry {
                sequence: 0,
                node_id: String::new(),
                database: db.clone(),
                collection: name.clone(),
                operation,
                key: key.to_string(),
                data,
                timestamp: now,
                origin_sequence: None,
            };
            let mut entries: Vec<LogEntry> = restored
                .written
                .iter()
                .filter_map(|doc| {
                    let key = doc.get("_key")?.as_str()?;
                    Some(entry(Operation::Update, key, serde_json::to_vec(doc).ok()))
                })
                .collect();
            entries.extend(
                restored
                    .removed
                    .iter()
                    .map(|key| entry(Operation::Delete, key, None)),
            );
            if !entries.is_empty() {
                let _ = log.append_batch(entries);
            }
        }
        Ok::<_, DbError>(restored)
    })
    .await
    .map_err(|e| DbError::InternalError(format!("Task join error: {e}")))??;
    Ok(Json(json!({
        "collection": coll_name,
        "as_of_micros": at,
        "restored": restored.written.len(),
        "removed": restored.removed.len(),
    })))
}

/// GET /_api/database/{db}/collection/{name}/version-tags
pub async fn list_version_tags(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
) -> Result<Json<Value>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    let tags: Vec<Value> = collection
        .version_tags()
        .into_iter()
        .map(|(name, micros)| json!({"name": name, "micros": micros}))
        .collect();
    Ok(Json(json!({ "tags": tags })))
}

/// POST /_api/database/{db}/collection/{name}/version-tags
pub async fn create_version_tag(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(req): Json<VersionTagRequest>,
) -> Result<Json<Value>, DbError> {
    let collection = versioned(&state, &db_name, &coll_name)?;
    let micros = match req.as_of.as_ref() {
        Some(v) => Some(crate::sdbql::executor::as_of_micros(v)?),
        None => None,
    };
    let at = crate::sync::validation::set_version_tag(
        &collection,
        state.replication_log.as_deref(),
        &db_name,
        &coll_name,
        &req.name,
        micros,
    )?;
    Ok(Json(json!({ "name": req.name.trim(), "micros": at })))
}

/// DELETE /_api/database/{db}/collection/{name}/version-tags/{tag}
pub async fn delete_version_tag(
    State(state): State<AppState>,
    Path((db_name, coll_name, tag)): Path<(String, String, String)>,
) -> Result<Json<Value>, DbError> {
    let collection = state
        .storage
        .get_database(&db_name)?
        .get_collection(&coll_name)?;
    let removed = crate::sync::validation::delete_version_tag(
        &collection,
        state.replication_log.as_deref(),
        &db_name,
        &coll_name,
        &tag,
    )?;
    if !removed {
        return Err(DbError::DocumentNotFound(format!("version tag '{tag}'")));
    }
    Ok(Json(json!({ "name": tag, "removed": true })))
}
