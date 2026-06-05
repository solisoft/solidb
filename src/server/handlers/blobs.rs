use super::system::{sanitize_filename, AppState};
use crate::{
    error::DbError,
    storage::http_client::get_http_client,
    storage::query_cache,
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
        metadata.insert(
            "name".to_string(),
            Value::String(sanitize_filename(&fn_str)),
        );
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
                query_cache::get_query_cache().invalidate_collection(&coll_name);
                return Ok(Json(doc));
            } else {
                return Err(DbError::InternalError(
                    "Sharded blob collection requires ShardCoordinator".to_string(),
                ));
            }
        }
    }

    // Only reach here for non-sharded collections.
    // Always persist chunks + metadata on the receiving node first. Cluster
    // replication (when configured) is best-effort redundancy, not the primary
    // store — if it were the primary store, a single-node deployment with no
    // cluster keyfile (the common case) would silently lose every chunk while
    // still inserting the metadata document.
    for (idx, data) in &chunks_buffer {
        collection.put_blob_chunk(&blob_key, *idx, data)?;
    }
    collection.insert(doc_value.clone())?;

    if collection.get_type() == "blob" {
        if let Some(ref coordinator) = state.shard_coordinator {
            let my_address = coordinator.my_address();
            let peer_addresses: Vec<String> = coordinator
                .get_node_addresses()
                .into_iter()
                .filter(|addr| addr != &my_address && addr != "local")
                .collect();

            if !peer_addresses.is_empty() {
                let replication_factor = std::cmp::min(2, peer_addresses.len());
                let cluster_secret = coordinator.cluster_secret();
                tracing::info!(
                    "Replicating {} blob chunks for {}/{} to {} peer(s)",
                    chunks_buffer.len(),
                    db_name,
                    coll_name,
                    replication_factor
                );
                for (chunk_idx, chunk_data) in &chunks_buffer {
                    let start_node = (*chunk_idx as usize) % peer_addresses.len();
                    for i in 0..replication_factor {
                        let node_addr = &peer_addresses[(start_node + i) % peer_addresses.len()];
                        if let Err(e) = replicate_blob_to_node(
                            node_addr,
                            &db_name,
                            &coll_name,
                            &blob_key,
                            &[(*chunk_idx, chunk_data.clone())],
                            None,
                            &cluster_secret,
                        )
                        .await
                        {
                            tracing::warn!(
                                "Failed to replicate chunk {} to {}: {} (chunk is safe locally)",
                                chunk_idx,
                                node_addr,
                                e
                            );
                        }
                    }
                }
            }
        }
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

    // Invalidate cached listings so the new file is immediately visible.
    query_cache::get_query_cache().invalidate_collection(&coll_name);

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
    let doc = collection
        .get(&key)
        .map_err(|_| DbError::DocumentNotFound(format!("Blob not found: {}", key)))?;

    let content_type = doc
        .get("type")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let file_name = doc.get("name").and_then(|v| v.as_str().map(str::to_string));

    let total_chunks = doc.get("chunks").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let total_size = doc.get("size").and_then(|v| v.as_u64());

    // Drive the stream off the known chunk count so a missing chunk raises a
    // hard error rather than silently truncating the response.
    let db_name_clone = db_name.clone();
    let coll_name_clone = coll_name.clone();
    let key_clone = key.clone();
    let collection_clone = collection.clone();
    let coordinator_clone = state.shard_coordinator.clone();

    let stream = async_stream::stream! {
        for chunk_idx in 0..total_chunks {
            // Prefer local storage, fall back to the cluster.
            let local = collection_clone.get_blob_chunk(&key_clone, chunk_idx);
            if let Ok(Some(data)) = local {
                yield Ok::<_, std::io::Error>(axum::body::Bytes::from(data));
                continue;
            }

            if let Some(ref coordinator) = coordinator_clone {
                match fetch_blob_chunk_from_cluster(
                    coordinator,
                    &db_name_clone,
                    &coll_name_clone,
                    &key_clone,
                    chunk_idx,
                ).await {
                    Ok(Some(data)) => {
                        yield Ok(axum::body::Bytes::from(data));
                        continue;
                    }
                    Ok(None) => {
                        tracing::error!(
                            "Blob {} chunk {} missing on all nodes",
                            key_clone, chunk_idx
                        );
                    }
                    Err(e) => {
                        tracing::error!("Error fetching blob chunk {}: {}", chunk_idx, e);
                    }
                }
            }

            yield Err(std::io::Error::other(format!(
                "blob chunk {} of {} missing",
                chunk_idx, total_chunks
            )));
            return;
        }
    };

    let body = Body::from_stream(stream);

    let mut builder = Response::builder();
    builder = builder.header("Content-Type", content_type);
    if let Some(size) = total_size {
        builder = builder.header("Content-Length", size);
    }
    if let Some(name) = file_name {
        let safe_name = sanitize_filename(&name);
        builder = builder.header(
            "Content-Disposition",
            format!("attachment; filename=\"{}\"", safe_name),
        );
    }

    Ok(builder.body(body).unwrap())
}

/// Distribute blob chunks across the cluster for fault tolerance
/// This provides redundancy without requiring logical sharding of the collection
pub async fn distribute_blob_chunks_across_cluster(
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
    let cluster_secret = coordinator.cluster_secret();

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
                &cluster_secret,
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

        let client = get_http_client();
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
