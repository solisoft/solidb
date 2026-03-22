use super::system::AppState;
use crate::{
    error::DbError,
    sync::{LogEntry, Operation},
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    response::Json,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
pub struct CreateUploadRequest {
    pub file_name: Option<String>,
    pub mime_type: Option<String>,
    pub total_size: u64,
    pub chunk_size: Option<u32>,
}

/// POST /_api/blob/{db}/{collection}/upload
/// Create a new resumable upload session.
pub async fn create_upload_session(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(body): Json<CreateUploadRequest>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;

    // Validate/auto-create blob collection (same pattern as upload_blob)
    match database.get_collection(&coll_name) {
        Ok(coll) => {
            if coll.get_type() != "blob" {
                return Err(DbError::BadRequest(format!(
                    "Collection '{}' is not a blob collection.",
                    coll_name
                )));
            }
        }
        Err(DbError::CollectionNotFound(_)) => {
            tracing::info!("Auto-creating blob collection {}/{}", db_name, coll_name);
            database.create_collection(coll_name.clone(), Some("blob".to_string()))?;
        }
        Err(e) => return Err(e),
    }

    if body.total_size == 0 {
        return Err(DbError::BadRequest(
            "total_size must be greater than 0".to_string(),
        ));
    }

    let info = state.upload_session_store.create(
        db_name,
        coll_name,
        body.file_name,
        body.mime_type,
        body.total_size,
        body.chunk_size,
    );

    Ok(Json(serde_json::json!({
        "upload_id": info.upload_id,
        "blob_key": info.blob_key,
        "chunk_size": info.chunk_size,
        "total_chunks": info.total_chunks,
    })))
}

/// POST /_api/blob/{db}/{collection}/upload/{upload_id}/{chunk_index}
/// Upload a single chunk (raw binary body). Returns progress + missing chunks.
pub async fn upload_chunk(
    State(state): State<AppState>,
    Path((_db_name, _coll_name, upload_id, chunk_index)): Path<(String, String, String, u32)>,
    body: Bytes,
) -> Result<Json<Value>, DbError> {
    // Look up session and write chunk
    let (db_name, coll_name, total_chunks, received_count, missing) = {
        let mut session = state
            .upload_session_store
            .get_mut(&upload_id)
            .ok_or_else(|| {
                DbError::DocumentNotFound("Upload session not found or expired".to_string())
            })?;

        if chunk_index >= session.total_chunks {
            return Err(DbError::BadRequest(format!(
                "chunk_index {} out of range (total_chunks: {})",
                chunk_index, session.total_chunks
            )));
        }

        // Write temp chunk to storage
        let database = state.storage.get_database(&session.db_name)?;
        let collection = database.get_collection(&session.collection_name)?;
        collection.put_blob_chunk_tmp(&upload_id, chunk_index, &body)?;

        // Update session tracking
        if !session.received_chunks[chunk_index as usize] {
            session.bytes_received += body.len() as u64;
        }
        session.received_chunks[chunk_index as usize] = true;
        session.last_activity = std::time::Instant::now();

        let received_count = session.received_chunks.iter().filter(|&&r| r).count() as u32;
        let missing: Vec<u32> = session
            .received_chunks
            .iter()
            .enumerate()
            .filter(|(_, &r)| !r)
            .map(|(i, _)| i as u32)
            .collect();

        (
            session.db_name.clone(),
            session.collection_name.clone(),
            session.total_chunks,
            received_count,
            missing,
        )
    };

    Ok(Json(serde_json::json!({
        "upload_id": upload_id,
        "chunk_index": chunk_index,
        "received_chunks": received_count,
        "total_chunks": total_chunks,
        "complete": missing.is_empty(),
        "missing_chunks": missing,
        "db": db_name,
        "collection": coll_name,
    })))
}

/// GET /_api/blob/{db}/{collection}/upload/{upload_id}
/// Get upload session status (for resuming after disconnect).
pub async fn get_upload_status(
    State(state): State<AppState>,
    Path((_db_name, _coll_name, upload_id)): Path<(String, String, String)>,
) -> Result<Json<Value>, DbError> {
    let session = state.upload_session_store.get(&upload_id).ok_or_else(|| {
        DbError::DocumentNotFound("Upload session not found or expired".to_string())
    })?;

    let received_count = session.received_chunks.iter().filter(|&&r| r).count() as u32;
    let missing: Vec<u32> = session
        .received_chunks
        .iter()
        .enumerate()
        .filter(|(_, &r)| !r)
        .map(|(i, _)| i as u32)
        .collect();

    Ok(Json(serde_json::json!({
        "upload_id": upload_id,
        "blob_key": session.blob_key,
        "db": session.db_name,
        "collection": session.collection_name,
        "file_name": session.file_name,
        "mime_type": session.mime_type,
        "total_size": session.total_size,
        "chunk_size": session.chunk_size,
        "total_chunks": session.total_chunks,
        "received_chunks": received_count,
        "bytes_received": session.bytes_received,
        "complete": missing.is_empty(),
        "missing_chunks": missing,
    })))
}

/// DELETE /_api/blob/{db}/{collection}/upload/{upload_id}
/// Abort an upload session and clean up temp chunks.
pub async fn abort_upload(
    State(state): State<AppState>,
    Path((_db_name, _coll_name, upload_id)): Path<(String, String, String)>,
) -> Result<Json<Value>, DbError> {
    let session = state
        .upload_session_store
        .remove(&upload_id)
        .ok_or_else(|| {
            DbError::DocumentNotFound("Upload session not found or expired".to_string())
        })?;

    // Clean up temp chunks from storage
    if let Ok(db) = state.storage.get_database(&session.db_name) {
        if let Ok(coll) = db.get_collection(&session.collection_name) {
            if let Err(e) = coll.delete_upload_chunks(&upload_id) {
                tracing::warn!(
                    "Failed to clean up temp chunks for aborted upload {}: {}",
                    upload_id,
                    e
                );
            }
        }
    }

    Ok(Json(serde_json::json!({
        "upload_id": upload_id,
        "status": "aborted",
    })))
}

/// POST /_api/blob/{db}/{collection}/upload/{upload_id}/complete
/// Finalize: promote temp chunks to permanent, create metadata doc, trigger replication.
pub async fn complete_upload(
    State(state): State<AppState>,
    Path((_db_name, _coll_name, upload_id)): Path<(String, String, String)>,
) -> Result<Json<Value>, DbError> {
    // Verify all chunks received
    let (db_name, coll_name, blob_key, file_name, mime_type, total_size, total_chunks) = {
        let session = state.upload_session_store.get(&upload_id).ok_or_else(|| {
            DbError::DocumentNotFound("Upload session not found or expired".to_string())
        })?;

        let missing: Vec<u32> = session
            .received_chunks
            .iter()
            .enumerate()
            .filter(|(_, &r)| !r)
            .map(|(i, _)| i as u32)
            .collect();

        if !missing.is_empty() {
            return Err(DbError::ConflictError(
                serde_json::json!({
                    "error": "Upload incomplete",
                    "missing_chunks": missing,
                    "received": session.total_chunks - missing.len() as u32,
                    "total": session.total_chunks,
                })
                .to_string(),
            ));
        }

        (
            session.db_name.clone(),
            session.collection_name.clone(),
            session.blob_key.clone(),
            session.file_name.clone(),
            session.mime_type.clone(),
            session.total_size,
            session.total_chunks,
        )
    };

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Finalize: move temp chunks to permanent storage
    collection.finalize_blob_upload(&upload_id, &blob_key, total_chunks)?;

    // Build metadata document
    let mut metadata = serde_json::Map::new();
    metadata.insert("_key".to_string(), Value::String(blob_key.clone()));
    if let Some(name) = file_name {
        metadata.insert("name".to_string(), Value::String(name));
    }
    if let Some(mt) = mime_type {
        metadata.insert("type".to_string(), Value::String(mt));
    }
    metadata.insert("size".to_string(), Value::Number(total_size.into()));
    metadata.insert("chunks".to_string(), Value::Number(total_chunks.into()));
    metadata.insert(
        "created".to_string(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    let doc_value = Value::Object(metadata);

    // Distribute across cluster if available (same pattern as upload_blob)
    if collection.get_type() == "blob" {
        if let Some(ref coordinator) = state.shard_coordinator {
            // Re-read the finalized chunks for distribution
            let mut chunks_buffer: Vec<(u32, Vec<u8>)> = Vec::new();
            for i in 0..total_chunks {
                if let Some(data) = collection.get_blob_chunk(&blob_key, i)? {
                    chunks_buffer.push((i, data));
                }
            }
            super::blobs::distribute_blob_chunks_across_cluster(
                coordinator,
                &db_name,
                &coll_name,
                &blob_key,
                &chunks_buffer,
                &doc_value,
                &state.storage,
            )
            .await?;
        } else {
            collection.insert(doc_value.clone())?;
        }
    } else {
        collection.insert(doc_value.clone())?;
    }

    // Log for replication
    if let Some(ref log) = state.replication_log {
        let entry = LogEntry {
            sequence: 0,
            node_id: "".to_string(),
            database: db_name.clone(),
            collection: coll_name.clone(),
            operation: Operation::Insert,
            key: blob_key.clone(),
            data: serde_json::to_vec(&doc_value).ok(),
            timestamp: chrono::Utc::now().timestamp_millis() as u64,
            origin_sequence: None,
        };
        let _ = log.append(entry);
    }

    // Remove session
    state.upload_session_store.remove(&upload_id);

    Ok(Json(doc_value))
}
