use axum::{
    extract::{Multipart, Path, State},
    http::HeaderMap,
    response::Json,
};
use serde_json::Value;

use crate::error::DbError;
use crate::server::handlers::AppState;

/// Verify the X-Cluster-Secret header matches the configured keyfile.
///
/// These `/_internal/blob/*` routes are mounted on the public router with no
/// user-auth middleware, so they MUST authenticate inter-node traffic via the
/// cluster secret. Fails closed when no keyfile is configured to prevent the
/// empty-secret bypass.
fn verify_cluster_secret(state: &AppState, headers: &HeaderMap) -> Result<(), DbError> {
    let secret = state.cluster_secret();
    if secret.is_empty() {
        return Err(DbError::InternalError(
            "Cluster keyfile not configured".to_string(),
        ));
    }
    let request_secret = headers
        .get("X-Cluster-Secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !crate::server::auth::constant_time_eq(request_secret.as_bytes(), secret.as_bytes()) {
        return Err(DbError::BadRequest("Invalid cluster secret".to_string()));
    }

    Ok(())
}

/// Fetch a specific blob chunk from this node
/// GET /_internal/blob/replicate/:db/:collection/:key/chunk/:chunk_idx
pub async fn get_blob_chunk(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((db_name, coll_name, blob_key, chunk_idx_str)): Path<(String, String, String, String)>,
) -> Result<axum::body::Bytes, DbError> {
    verify_cluster_secret(&state, &headers)?;

    let chunk_idx = chunk_idx_str
        .parse::<u32>()
        .map_err(|_| DbError::BadRequest("Invalid chunk index".to_string()))?;

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    if collection.get_type() != "blob" {
        return Err(DbError::BadRequest(format!(
            "Collection '{}' is not a blob collection",
            coll_name
        )));
    }

    match collection.get_blob_chunk(&blob_key, chunk_idx) {
        Ok(Some(data)) => Ok(axum::body::Bytes::from(data)),
        Ok(None) => Err(DbError::DocumentNotFound(format!(
            "Chunk {} not found for blob {}",
            chunk_idx, blob_key
        ))),
        Err(e) => Err(e),
    }
}

/// Replicate blob chunks to a remote node
/// This uses the internal replication endpoint to send chunks
/// Replicate blob chunks to a remote node
/// This uses the internal replication endpoint to send chunks
pub async fn replicate_blob_to_node(
    target_node_address: &str, // e.g. "http://10.0.0.2:6745"
    database: &str,
    collection: &str,
    blob_key: &str,
    chunks: &[(u32, Vec<u8>)],
    metadata: Option<&Value>,
    cluster_secret: &str, // sent as X-Cluster-Secret to authenticate inter-node traffic
) -> Result<(), DbError> {
    // Skip if no chunks to replicate AND no metadata
    if chunks.is_empty() && metadata.is_none() {
        return Ok(());
    }

    let scheme = std::env::var("SOLIDB_CLUSTER_SCHEME").unwrap_or_else(|_| "http".to_string());
    let url_base = if target_node_address.contains("://") {
        target_node_address.to_string()
    } else {
        format!("{}://{}", scheme, target_node_address)
    };

    let url = format!(
        "{}/_internal/blob/replicate/{}/{}/{}",
        url_base, database, collection, blob_key
    );

    tracing::debug!(
        "Replicating {} chunks (metadata: {}) for blob {} to {}",
        chunks.len(),
        metadata.is_some(),
        blob_key,
        target_node_address
    );

    let client = reqwest::Client::new();
    let mut form = reqwest::multipart::Form::new();

    // Add metadata if present
    if let Some(meta) = metadata {
        let json_str = serde_json::to_string(meta)
            .map_err(|e| DbError::InternalError(format!("Failed to serialize metadata: {}", e)))?;
        let part = reqwest::multipart::Part::text(json_str)
            .mime_str("application/json")
            .map_err(|e| DbError::InternalError(format!("Invalid mime: {}", e)))?;
        form = form.part("metadata", part);
    }

    // Add each chunk as a field
    // Field name format: "chunk_<index>"
    for (index, data) in chunks {
        let part = reqwest::multipart::Part::bytes(data.clone());
        form = form.part(format!("chunk_{}", index), part);
    }

    let mut req_builder = client.post(&url).header("X-Cluster-Secret", cluster_secret);

    if let Some(trace_ctx) = crate::observability::get_current_trace_context() {
        req_builder = req_builder.header("traceparent", trace_ctx.to_header());
    }

    let response = req_builder
        .multipart(form)
        .send()
        .await
        .map_err(|e| DbError::InternalError(format!("Replication request failed: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(DbError::InternalError(format!(
            "Replication failed with status {}: {}",
            status, text
        )));
    }

    Ok(())
}

/// Handler for receiving replicated blob chunks
/// POST /_internal/blob/replicate/:db/:collection/:key
pub async fn receive_blob_replication(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((db_name, coll_name, blob_key)): Path<(String, String, String)>,
    mut multipart: Multipart,
) -> Result<Json<Value>, DbError> {
    verify_cluster_secret(&state, &headers)?;

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    if collection.get_type() != "blob" {
        return Err(DbError::BadRequest(format!(
            "Collection '{}' is not a blob collection",
            coll_name
        )));
    }

    tracing::debug!(
        "Receiving replicated chunks for blob {}/{}/{}",
        db_name,
        coll_name,
        blob_key
    );

    let mut chunks_received = 0;
    let mut metadata_inserted = false;

    // Process multipart fields
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| DbError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();

        if name == "metadata" {
            // Process metadata document
            let text = field
                .text()
                .await
                .map_err(|e| DbError::BadRequest(e.to_string()))?;
            if let Ok(doc_value) = serde_json::from_str::<Value>(&text) {
                // Reject metadata whose `_key` does not match the URL path: the
                // URL is the authoritative key (mirrors how chunks are stored),
                // and trusting the body lets a caller insert documents under a
                // different key than the chunks they uploaded.
                if let Some(meta_key) = doc_value.get("_key").and_then(|k| k.as_str()) {
                    if meta_key != blob_key {
                        return Err(DbError::BadRequest(format!(
                            "Metadata _key '{}' does not match URL path '{}'",
                            meta_key, blob_key
                        )));
                    }
                }
                tracing::info!("Inserting replicated metadata for blob {}", blob_key);
                // Insert metadata document
                // Note: This insert is local to this shard (Primary).
                // Standard replication logic should pick this up for replicas if enabled on insert.
                // However, we need to check if we should be replicating this INSERT to our followers?
                // Typically `collection.insert` is low-level.
                // If we are Primary, we should log it.
                collection.insert(doc_value.clone())?;

                // Replicate metadata insert via Log
                if let Some(ref log) = state.replication_log {
                    let entry = crate::sync::log::LogEntry {
                        sequence: 0,
                        node_id: "".to_string(),
                        database: db_name.clone(),
                        collection: coll_name.clone(),
                        operation: crate::sync::Operation::Insert,
                        key: blob_key.clone(),
                        data: serde_json::to_vec(&doc_value).ok(),
                        timestamp: chrono::Utc::now().timestamp_millis() as u64,
                        origin_sequence: None,
                    };
                    let _ = log.append(entry);
                }
                metadata_inserted = true;
            } else {
                tracing::warn!("Failed to parse metadata JSON");
            }
        } else if name.starts_with("chunk_") {
            // Extract index from field name "chunk_<index>"
            if let Ok(index) = name.trim_start_matches("chunk_").parse::<u32>() {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| DbError::BadRequest(e.to_string()))?;

                // Store chunk locally
                collection.put_blob_chunk(&blob_key, index, &data)?;
                chunks_received += 1;
            } else {
                tracing::warn!("Invalid chunk field name: {}", name);
            }
        }
    }

    tracing::debug!(
        "Stored {} replicated chunks (metadata: {}) for blob {}",
        chunks_received,
        metadata_inserted,
        blob_key
    );

    Ok(Json(serde_json::json!({
        "status": "ok",
        "chunks_received": chunks_received,
        "metadata_inserted": metadata_inserted
    })))
}

/// Handler for receiving forwarded blob uploads (used by ShardCoordinator)
/// POST /_internal/blob/upload/:db/:collection
pub async fn receive_blob_upload(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((db_name, coll_name)): Path<(String, String)>,
    mut multipart: Multipart,
) -> Result<Json<Value>, DbError> {
    verify_cluster_secret(&state, &headers)?;

    let database = state.storage.get_database(&db_name)?;

    // Auto-create the physical shard blob collection if it doesn't exist
    // This can happen when blob uploads are forwarded to a node before the shard was explicitly created
    let collection = match database.get_collection(&coll_name) {
        Ok(coll) => coll,
        Err(_) => {
            tracing::info!(
                "Auto-creating blob shard collection: {}/{}",
                db_name,
                coll_name
            );
            database.create_collection(coll_name.clone(), Some("blob".to_string()))?;
            let coll = database.get_collection(&coll_name)?;
            // Ensure the in-memory collection object knows it's a blob immediately
            // (In case the read back from RocksDB was too fast or cached)
            if coll.get_type() != "blob" {
                tracing::warn!(
                    "Collection created as blob but loaded as {}, forcing type update",
                    coll.get_type()
                );
                // The set_type method on Collection updates both memory and disk
                if let Err(e) = coll.set_type("blob") {
                    tracing::error!("Failed to force set collection type: {}", e);
                }
            }
            coll
        }
    };

    if collection.get_type() != "blob" {
        return Err(DbError::BadRequest(format!(
            "Collection '{}' is not a blob collection",
            coll_name
        )));
    }

    let mut metadata: Option<Value> = None;
    let mut chunks_received = 0;

    // Process multipart fields
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| DbError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();

        if name == "metadata" {
            // Process metadata document
            let text = field
                .text()
                .await
                .map_err(|e| DbError::BadRequest(e.to_string()))?;
            metadata = Some(
                serde_json::from_str::<Value>(&text)
                    .map_err(|e| DbError::BadRequest(format!("Invalid metadata JSON: {}", e)))?,
            );
        } else if name.starts_with("chunk_") {
            // Extract index from field name "chunk_<index>"
            if let Ok(index) = name.trim_start_matches("chunk_").parse::<u32>() {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| DbError::BadRequest(e.to_string()))?;

                // Get blob key from metadata (should be set by now)
                if let Some(ref meta) = metadata {
                    if let Some(key_val) = meta.get("_key").and_then(|k| k.as_str()) {
                        // Store chunk locally
                        collection.put_blob_chunk(key_val, index, &data)?;
                        chunks_received += 1;
                    } else {
                        return Err(DbError::BadRequest(
                            "Metadata missing _key field".to_string(),
                        ));
                    }
                } else {
                    return Err(DbError::BadRequest(
                        "Metadata field must come before chunks".to_string(),
                    ));
                }
            } else {
                tracing::warn!("Invalid chunk field name: {}", name);
            }
        }
    }

    // Insert metadata document
    if let Some(meta) = metadata {
        let blob_key = meta
            .get("_key")
            .and_then(|k| k.as_str())
            .ok_or_else(|| DbError::BadRequest("Metadata must contain _key".to_string()))?;

        tracing::info!("Inserting forwarded blob metadata for blob {}", blob_key);
        collection.insert(meta.clone())?;

        // NOTE: Don't add to replication log for sharded data!
        // Each node only stores its assigned shards - data is partitioned, not replicated.

        tracing::debug!(
            "Stored {} chunks for forwarded blob {}",
            chunks_received,
            blob_key
        );

        Ok(Json(meta))
    } else {
        Err(DbError::BadRequest(
            "No metadata provided in blob upload".to_string(),
        ))
    }
}
