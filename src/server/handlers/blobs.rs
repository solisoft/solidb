use super::system::AppState;
use crate::{
    error::DbError,
    sync::blob_replication::replicate_blob_to_node,
    sync::{LogEntry, Operation},
};
use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    response::Json,
    response::Response,
};
use futures::StreamExt;
use serde_json::Value;

// ==================== Blob Handlers ====================

pub async fn upload_blob(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    multipart_result: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Json<Value>, DbError> {
    let mut multipart = multipart_result.map_err(|e| DbError::BadRequest(e.to_string()))?;
    let database = state.storage.get_database(&db_name)?;

    // Try to get the collection, auto-create as blob collection if it doesn't exist
    let collection = match database.get_collection(&coll_name) {
        Ok(coll) => {
            // Collection exists - check if it's a blob collection
            if coll.get_type() != "blob" {
                return Err(DbError::BadRequest(format!("Collection '{}' is not a blob collection. Please create it as a blob collection first.", coll_name)));
            }
            coll
        }
        Err(DbError::CollectionNotFound(_)) => {
            // Auto-create blob collection
            tracing::info!("Auto-creating blob collection {}/{}", db_name, coll_name);
            database.create_collection(coll_name.clone(), Some("blob".to_string()))?;
            database.get_collection(&coll_name)?
        }
        Err(e) => return Err(e),
    };

    let mut file_name = None;
    let mut mime_type = None;
    let mut total_size = 0usize;
    let mut chunk_count = 0u32;
    // Generate a temporary key or use one if we support PUT (for now auto-generate)
    let blob_key = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
    tracing::info!(
        "Starting upload_blob for {}/{} with key {}",
        db_name,
        coll_name,
        blob_key
    );

    let mut chunks_buffer: Vec<(u32, Vec<u8>)> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| DbError::BadRequest(e.to_string()))?
    {
        if let Some(name) = field.name() {
            tracing::info!("Processing field: {}", name);
            if name == "file" {
                if let Some(fname) = field.file_name() {
                    file_name = Some(fname.to_string());
                }
                if let Some(mtype) = field.content_type() {
                    mime_type = Some(mtype.to_string());
                }

                let mut stream = field;
                while let Some(chunk_res) = stream.next().await {
                    let chunk = chunk_res.map_err(|e| {
                        tracing::error!("Chunk error: {}", e);
                        DbError::BadRequest(e.to_string())
                    })?;
                    let data = chunk.to_vec();
                    let len = data.len();
                    tracing::debug!("Received chunk size: {}", len);

                    if len > 0 {
                        chunks_buffer.push((chunk_count, data));
                        total_size += len;
                        chunk_count += 1;
                    }
                }
                tracing::info!(
                    "Buffered file. Total size: {}, chunks: {}",
                    total_size,
                    chunks_buffer.len()
                );
            }
        }
    }

    // Create metadata document
    let mut metadata = serde_json::Map::new();
    metadata.insert("_key".to_string(), Value::String(blob_key.clone()));
    if let Some(fn_str) = file_name {
        metadata.insert("name".to_string(), Value::String(fn_str));
    }
    if let Some(mt_str) = mime_type {
        metadata.insert("type".to_string(), Value::String(mt_str));
    }
    metadata.insert("size".to_string(), Value::Number(total_size.into()));
    metadata.insert("chunks".to_string(), Value::Number(chunk_count.into()));
    metadata.insert(
        "created".to_string(),
        Value::String(chrono::Utc::now().to_rfc3339()),
    );
    let doc_value = Value::Object(metadata);

    // Check for sharding
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                tracing::info!(
                    "[BLOB_UPLOAD] Using ShardCoordinator for {}/{}",
                    db_name,
                    coll_name
                );
                let doc = coordinator
                    .upload_blob(
                        &db_name,
                        &coll_name,
                        &shard_config,
                        doc_value,
                        chunks_buffer,
                    )
                    .await?;
                return Ok(Json(doc));
            } else {
                return Err(DbError::InternalError(
                    "Sharded blob collection requires ShardCoordinator".to_string(),
                ));
            }
        }
    }

    // Only reach here for non-sharded collections
    // For blob collections, distribute chunks across the cluster for fault tolerance
    if collection.get_type() == "blob" {
        if let Some(ref _coordinator) = state.shard_coordinator {
            // Distribute blob chunks across available nodes
            tracing::info!(
                "Distributing {} blob chunks for {}/{} across cluster",
                chunks_buffer.len(),
                db_name,
                coll_name
            );
            distribute_blob_chunks_across_cluster(
                state.shard_coordinator.as_ref().unwrap(),
                &db_name,
                &coll_name,
                &blob_key,
                &chunks_buffer,
                &doc_value,
                &state.storage,
            )
            .await?;
        } else {
            // No coordinator available, store locally as fallback
            tracing::warn!("No cluster coordinator available, storing blob chunks locally");
            for (idx, data) in &chunks_buffer {
                collection.put_blob_chunk(&blob_key, *idx, data)?;
            }
            collection.insert(doc_value.clone())?;
        }
    } else {
        // Regular document collection - store locally
        for (idx, data) in &chunks_buffer {
            collection.put_blob_chunk(&blob_key, *idx, data)?;
        }
        collection.insert(doc_value.clone())?;
    }

    // Log operation for replication (if enabled for other collections, keep logging for consistency)
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

    Ok(Json(doc_value))
}

pub async fn download_blob(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
) -> Result<Response, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    if collection.get_type() != "blob" {
        return Err(DbError::BadRequest(format!(
            "Collection '{}' is not a blob collection.",
            coll_name
        )));
    }

    // Check for sharding
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                tracing::info!(
                    "[BLOB_DOWNLOAD] Using ShardCoordinator for {}/{}",
                    db_name,
                    coll_name
                );
                return coordinator
                    .download_blob(&db_name, &coll_name, &shard_config, &key)
                    .await;
            } else {
                return Err(DbError::InternalError(
                    "Sharded blob collection requires ShardCoordinator".to_string(),
                ));
            }
        }
    }

    // Only reach here for non-sharded collections
    // For blob collections, chunks may be distributed across the cluster
    // First check if metadata exists locally
    if collection.get(&key).is_err() {
        return Err(DbError::DocumentNotFound(format!(
            "Blob not found: {}",
            key
        )));
    }

    let content_type = if let Ok(doc) = collection.get(&key) {
        if let Some(v) = doc.get("type") {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else {
                "application/octet-stream".to_string()
            }
        } else {
            "application/octet-stream".to_string()
        }
    } else {
        "application/octet-stream".to_string()
    };

    let file_name = if let Ok(doc) = collection.get(&key) {
        if let Some(v) = doc.get("name") {
            v.as_str().map(|s| s.to_string())
        } else {
            None
        }
    } else {
        None
    };

    // Create a stream that yields chunks
    // We need to move ownership of required data into the stream
    let db_name_clone = db_name.clone();
    let coll_name_clone = coll_name.clone();
    let key_clone = key.clone();
    let collection_clone = collection.clone();
    let coordinator_clone = state.shard_coordinator.clone();

    let stream = async_stream::stream! {
        let mut chunk_idx = 0;
        loop {
            // First try local storage
            match collection_clone.get_blob_chunk(&key_clone, chunk_idx) {
                Ok(data) => {
                    yield Ok::<_, std::io::Error>(axum::body::Bytes::from(data.unwrap_or_default()));
                    chunk_idx += 1;
                }
                Err(_) => {
                    // Not found locally (or end of file)
                    // If we have a cluster coordinator, check other nodes
                    if let Some(ref coordinator) = coordinator_clone {
                        match fetch_blob_chunk_from_cluster(
                            coordinator,
                            &db_name_clone,
                            &coll_name_clone,
                            &key_clone,
                            chunk_idx
                        ).await {
                            Ok(Some(data)) => {
                                // Found on another node
                                yield Ok(axum::body::Bytes::from(data));
                                chunk_idx += 1;
                            }
                            Ok(None) => {
                                // Not found anywhere - assume end of file
                                break;
                            }
                            Err(e) => {
                                // Error fetching
                                tracing::error!("Error fetching blob chunk: {}", e);
                                break;
                            }
                        }
                    } else {
                        // No cluster to check, must be EOF
                        break;
                    }
                }
            }
        }
    };

    let body = Body::from_stream(stream);

    let mut builder = Response::builder();
    builder = builder.header("Content-Type", content_type);
    if let Some(name) = file_name {
        builder = builder.header(
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", name),
        );
    }

    Ok(builder.body(body).unwrap())
}

/// Distribute blob chunks across the cluster for fault tolerance
/// This provides redundancy without requiring logical sharding of the collection
async fn distribute_blob_chunks_across_cluster(
    coordinator: &crate::sharding::coordinator::ShardCoordinator,
    db_name: &str,
    coll_name: &str,
    blob_key: &str,
    chunks: &[(u32, Vec<u8>)],
    metadata: &serde_json::Value,
    storage: &crate::storage::StorageEngine,
) -> Result<(), DbError> {
    // Get available nodes
    let node_addresses = coordinator.get_node_addresses();
    if node_addresses.is_empty() {
        return Err(DbError::InternalError(
            "No nodes available for blob chunk distribution".to_string(),
        ));
    }

    tracing::info!(
        "Distributing blob chunks to {} nodes: {:?}",
        node_addresses.len(),
        node_addresses
    );

    // For each chunk, replicate to multiple nodes for redundancy
    // We'll use a simple round-robin distribution with replication factor of min(3, node_count)
    let replication_factor = std::cmp::min(3, node_addresses.len());

    for (chunk_idx, chunk_data) in chunks {
        // Select target nodes for this chunk using round-robin
        let start_node = (*chunk_idx as usize) % node_addresses.len();
        let target_nodes: Vec<_> = (0..replication_factor)
            .map(|i| &node_addresses[(start_node + i) % node_addresses.len()])
            .collect();

        tracing::debug!(
            "Chunk {} will be stored on nodes: {:?}",
            chunk_idx,
            target_nodes
        );

        // Replicate chunk to each target node
        for node_addr in target_nodes {
            if let Err(e) = replicate_blob_to_node(
                node_addr,
                db_name,
                coll_name,
                blob_key,
                &[(*chunk_idx, chunk_data.clone())],
                None, // No metadata for individual chunks
                "",   // No auth needed for internal replication
            )
            .await
            {
                tracing::warn!(
                    "Failed to replicate chunk {} to {}: {}",
                    chunk_idx,
                    node_addr,
                    e
                );
                // Continue with other nodes - don't fail the whole operation
            }
        }
    }

    // Store metadata document locally (this will be synced via regular replication)
    let database = storage.get_database(db_name)?;
    let collection = database.get_collection(coll_name)?;
    collection.insert(metadata.clone())?;

    tracing::info!(
        "Successfully distributed {} chunks for blob {} across {} nodes",
        chunks.len(),
        blob_key,
        replication_factor
    );

    Ok(())
}

/// Fetch a blob chunk from other nodes in the cluster
async fn fetch_blob_chunk_from_cluster(
    coordinator: &crate::sharding::coordinator::ShardCoordinator,
    db_name: &str,
    coll_name: &str,
    blob_key: &str,
    chunk_idx: u32,
) -> Result<Option<Vec<u8>>, DbError> {
    let node_addresses = coordinator.get_node_addresses();

    // Try each node to find the chunk
    for node_addr in &node_addresses {
        // Skip local node (we already checked it)
        if node_addr == "local" {
            continue;
        }

        let scheme = std::env::var("SOLIDB_CLUSTER_SCHEME").unwrap_or_else(|_| "http".to_string());
        let url = if node_addr.contains("://") {
            format!(
                "{}/_internal/blob/replicate/{}/{}/{}/chunk/{}",
                node_addr, db_name, coll_name, blob_key, chunk_idx
            )
        } else {
            format!(
                "{}://{}/_internal/blob/replicate/{}/{}/{}/chunk/{}",
                scheme, node_addr, db_name, coll_name, blob_key, chunk_idx
            )
        };

        let client = reqwest::Client::new();
        let secret = coordinator.cluster_secret();

        match client
            .get(&url)
            .header("X-Cluster-Secret", &secret)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => match response.bytes().await {
                Ok(bytes) => {
                    let data = bytes.to_vec();
                    tracing::debug!(
                        "Fetched chunk {} for blob {} from {}",
                        chunk_idx,
                        blob_key,
                        node_addr
                    );
                    return Ok(Some(data));
                }
                Err(e) => {
                    tracing::warn!("Failed to read chunk data from {}: {}", node_addr, e);
                }
            },
            Ok(response) => {
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    // Chunk not on this node, try next
                    continue;
                } else {
                    tracing::warn!(
                        "Failed to fetch chunk from {}: status {}",
                        node_addr,
                        response.status()
                    );
                }
            }
            Err(e) => {
                tracing::warn!("Network error fetching chunk from {}: {}", node_addr, e);
            }
        }
    }

    // Chunk not found on any node
    tracing::debug!(
        "Chunk {} for blob {} not found on any node",
        chunk_idx,
        blob_key
    );
    Ok(None)
}
