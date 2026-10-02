use super::blob_range::{parse_range, record_chunk_layout, ChunkLayout, RangeRequest};
use super::system::{sanitize_filename, AppState};
use crate::{
    error::DbError,
    storage::http_client::get_http_client,
    storage::query_cache,
    sync::blob_replication::replicate_blob_to_node,
    sync::{LogEntry, Operation},
};
use axum::{
    body::{Body, Bytes},
    extract::{Multipart, Path, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::Json,
    response::Response,
};
use futures::StreamExt;
use serde_json::Value;

// ==================== Blob Handlers ====================

/// Size of the chunks a multipart upload is stored in. Same as a resumable
/// upload's default and the Lua `upload` helper.
const MULTIPART_CHUNK_SIZE: usize = 1024 * 1024;

pub async fn upload_blob(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    claims: Option<axum::Extension<crate::server::auth::Claims>>,
    multipart_result: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Json<Value>, DbError> {
    let mut multipart = multipart_result.map_err(|e| DbError::BadRequest(e.to_string()))?;
    // A blob upload writes documents and chunks: same tiers as the document API.
    crate::storage::check_write_access(
        &coll_name,
        crate::server::handlers::query::write_actor_from_claims(claims.as_deref()),
    )?;
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
    // Network reads arrive in whatever sizes the transport delivered —
    // thousands of a few KB each for a podcast episode. Re-cut them into
    // fixed-size chunks so the stored layout is one number (see
    // `blob_range::record_chunk_layout`) and a range request can compute
    // which chunk it starts in.
    let mut pending: Vec<u8> = Vec::with_capacity(MULTIPART_CHUNK_SIZE);

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
                    tracing::debug!("Received chunk size: {}", chunk.len());
                    total_size += chunk.len();
                    let mut rest: &[u8] = &chunk;
                    while !rest.is_empty() {
                        let take = (MULTIPART_CHUNK_SIZE - pending.len()).min(rest.len());
                        pending.extend_from_slice(&rest[..take]);
                        rest = &rest[take..];
                        if pending.len() == MULTIPART_CHUNK_SIZE {
                            let full = std::mem::replace(
                                &mut pending,
                                Vec::with_capacity(MULTIPART_CHUNK_SIZE),
                            );
                            chunks_buffer.push((chunk_count, full));
                            chunk_count += 1;
                        }
                    }
                }
                if !pending.is_empty() {
                    chunks_buffer.push((chunk_count, std::mem::take(&mut pending)));
                    chunk_count += 1;
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
    let chunk_sizes: Vec<u64> = chunks_buffer.iter().map(|(_, d)| d.len() as u64).collect();
    record_chunk_layout(&mut metadata, &chunk_sizes);
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

/// `GET` (and, through axum, `HEAD`) `/_api/blob/{db}/{collection}/{key}`.
///
/// Non-sharded blobs honour one `Range: bytes=…` header — `a-b`, `a-` or
/// `-n` — with `206 Partial Content`, so browsers and podcast apps can stream
/// and seek audio and video. A range past the end answers `416`; anything
/// malformed, multi-range or under `If-Range` (no validator is ever sent, so
/// none can match) is ignored and the whole blob is sent with `200`.
/// Sharded collections always answer `200` with the whole blob.
pub async fn download_blob(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
    method: Method,
    headers: HeaderMap,
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

    // Ranges are defined for GET only (RFC 9110 §14.2); a HEAD reports the
    // full representation. Without a recorded size there is nothing to
    // resolve a range against, so such a blob is served whole, as before.
    let range = match total_size {
        Some(total) if method == Method::GET && !headers.contains_key(header::IF_RANGE) => {
            parse_range(
                headers.get(header::RANGE).and_then(|v| v.to_str().ok()),
                total,
            )
        }
        _ => RangeRequest::Full,
    };

    let mut builder = Response::builder();
    // The stored type is attacker-controlled, so never let a browser sniff
    // its way to something executable.
    builder = builder.header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    if total_size.is_some() {
        builder = builder.header(header::ACCEPT_RANGES, "bytes");
    }

    if let (RangeRequest::Unsatisfiable, Some(total)) = (range, total_size) {
        return builder
            .status(StatusCode::RANGE_NOT_SATISFIABLE)
            .header(header::CONTENT_RANGE, format!("bytes */{}", total))
            .header(header::CONTENT_LENGTH, 0)
            .body(Body::empty())
            .map_err(|e| DbError::InternalError(format!("Failed to build blob response: {}", e)));
    }

    // `content_type` is the blob document's `type` field, written by whoever
    // uploaded the blob. `Response::builder().header(...)` defers validation
    // to `.body()`, so a stored type containing a newline or any
    // non-visible-ASCII byte made the old `.unwrap()` panic on every request
    // for that blob. Validate it here and fall back instead.
    let header_value = HeaderValue::from_str(&content_type).unwrap_or_else(|_| {
        tracing::warn!(
            "Blob {} declares an unusable content type; serving as octet-stream",
            key
        );
        HeaderValue::from_static("application/octet-stream")
    });
    builder = builder.header(header::CONTENT_TYPE, header_value);
    if let Some(name) = file_name {
        let safe_name = sanitize_filename(&name);
        builder = builder.header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", safe_name),
        );
    }

    let source = BlobChunkSource {
        collection: collection.clone(),
        coordinator: state.shard_coordinator.clone(),
        db_name: db_name.clone(),
        coll_name: coll_name.clone(),
        key: key.clone(),
        total_chunks,
    };

    let body = match (range, total_size) {
        (RangeRequest::Partial { start, end }, Some(total)) => {
            let layout = ChunkLayout::from_doc(&doc, total_chunks, total);
            builder = builder
                .status(StatusCode::PARTIAL_CONTENT)
                .header(
                    header::CONTENT_RANGE,
                    format!("bytes {}-{}/{}", start, end, total),
                )
                .header(header::CONTENT_LENGTH, end - start + 1);
            Body::from_stream(range_stream(source, layout, total, start, end))
        }
        _ => {
            if let Some(size) = total_size {
                builder = builder.header(header::CONTENT_LENGTH, size);
            }
            Body::from_stream(full_stream(source))
        }
    };

    builder
        .body(body)
        .map_err(|e| DbError::InternalError(format!("Failed to build blob response: {}", e)))
}

/// Where a download reads chunks from: the local collection first, then the
/// other nodes of the cluster.
struct BlobChunkSource {
    collection: crate::storage::Collection,
    coordinator: Option<std::sync::Arc<crate::sharding::coordinator::ShardCoordinator>>,
    db_name: String,
    coll_name: String,
    key: String,
    total_chunks: u32,
}

impl BlobChunkSource {
    /// Chunk `chunk_idx`, or an error naming it when no node has it — a
    /// missing chunk must fail the response, never silently truncate it.
    async fn fetch(&self, chunk_idx: u32) -> Result<Vec<u8>, std::io::Error> {
        // Prefer local storage, fall back to the cluster.
        if let Ok(Some(data)) = self.collection.get_blob_chunk(&self.key, chunk_idx) {
            return Ok(data);
        }

        if let Some(ref coordinator) = self.coordinator {
            match fetch_blob_chunk_from_cluster(
                coordinator,
                &self.db_name,
                &self.coll_name,
                &self.key,
                chunk_idx,
            )
            .await
            {
                Ok(Some(data)) => return Ok(data),
                Ok(None) => {
                    tracing::error!("Blob {} chunk {} missing on all nodes", self.key, chunk_idx);
                }
                Err(e) => {
                    tracing::error!("Error fetching blob chunk {}: {}", chunk_idx, e);
                }
            }
        }

        Err(std::io::Error::other(format!(
            "blob chunk {} of {} missing",
            chunk_idx, self.total_chunks
        )))
    }
}

/// Every chunk, in order. Driven off the known chunk count so a missing chunk
/// raises a hard error rather than silently truncating the response.
fn full_stream(
    source: BlobChunkSource,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> {
    async_stream::stream! {
        for chunk_idx in 0..source.total_chunks {
            match source.fetch(chunk_idx).await {
                Ok(data) => yield Ok(Bytes::from(data)),
                Err(e) => {
                    yield Err(e);
                    return;
                }
            }
        }
    }
}

/// Bytes `start..=end` of the blob, and nothing else.
///
/// With a recorded layout the first chunk read is the one holding `start`.
/// A blob stored before layouts were recorded is walked from chunk 0, the
/// chunks before the range read and dropped. Either way reading stops at the
/// chunk holding `end`, and only the requested slice of the first and last
/// chunk is sent. `Content-Length` was promised up front, so a chunk whose
/// length contradicts the layout, or a blob that ends early, fails the
/// response instead of sending the wrong bytes.
fn range_stream(
    source: BlobChunkSource,
    layout: Option<ChunkLayout>,
    total: u64,
    start: u64,
    end: u64,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> {
    async_stream::stream! {
        let (mut chunk_idx, mut offset) = match &layout {
            Some(layout) => layout.locate(start, total).unwrap_or((source.total_chunks, 0)),
            None => (0, 0),
        };

        while chunk_idx < source.total_chunks && offset <= end {
            let data = match source.fetch(chunk_idx).await {
                Ok(data) => data,
                Err(e) => {
                    yield Err(e);
                    return;
                }
            };
            let len = data.len() as u64;
            if let Some(layout) = &layout {
                let expected = layout.chunk_len(chunk_idx, total);
                if len != expected {
                    tracing::error!(
                        "Blob {} chunk {} is {} bytes, its document records {}",
                        source.key, chunk_idx, len, expected
                    );
                    yield Err(std::io::Error::other(format!(
                        "blob chunk {} is {} bytes, its document records {}",
                        chunk_idx, len, expected
                    )));
                    return;
                }
            }

            let chunk_end = offset + len; // exclusive
            if chunk_end > start {
                let from = start.saturating_sub(offset) as usize;
                let to = ((end + 1).min(chunk_end) - offset) as usize;
                yield Ok(Bytes::from(data).slice(from..to));
            }
            offset = chunk_end;
            chunk_idx += 1;
        }

        if offset <= end {
            yield Err(std::io::Error::other(format!(
                "blob {} ends at byte {}, before the requested end {}",
                source.key, offset, end
            )));
        }
    }
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

        let scheme = crate::cluster::http::cluster_scheme().to_string();
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
