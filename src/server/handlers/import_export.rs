use super::system::{sanitize_filename, AppState};
use crate::error::DbError;
use crate::storage::http_client::get_http_client;
use axum::{
    body::Body,
    extract::{Multipart, Path, State},
    http::HeaderMap,
    response::{Json, Response},
};
use base64::{engine::general_purpose, Engine as _};
use futures::stream::StreamExt;
use serde_json::Value;

pub async fn export_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    _headers: HeaderMap,
) -> Result<Response, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;
    let shard_config = collection.get_shard_config();
    let is_blob = collection.get_type() == "blob";

    // Prepare coordinator reference and secret for remote calls
    let coordinator_opt = state.shard_coordinator.clone();
    let cluster_manager_opt = state.cluster_manager.clone();
    let secret_env = state.cluster_secret();

    // Capture necessary variables for the async stream
    let db_name_clone = db_name.clone();
    let coll_name_clone = coll_name.clone();
    let collection_clone = collection.clone();
    let state_storage = state.storage.clone();

    let stream = async_stream::stream! {
        let num_shards = shard_config.as_ref().map(|c| c.num_shards).unwrap_or(0);

        if num_shards > 0 {
            // SHARDED EXPORT: Iterate over all physical shards
            // We need to determine where each shard is located
            let shard_table = if let Some(coord) = &coordinator_opt {
                coord.get_shard_table(&db_name_clone, &coll_name_clone)
            } else {
                None
            };

            let client = get_http_client();
            let my_node_id = if let Some(mgr) = &cluster_manager_opt {
                mgr.local_node_id()
            } else {
                "local".to_string()
            };

            for shard_id in 0..num_shards {
                let physical_name = format!("{}_s{}", coll_name_clone, shard_id);

                // Determine primary node for this shard
                let primary_node = if let Some(ref table) = shard_table {
                    table.assignments.get(&shard_id).map(|a| a.primary_node.clone()).unwrap_or_else(|| "unknown".to_string())
                } else {
                     // Fallback: assume local if no table (standalone mode?) or simple modulo
                     "local".to_string()
                };


                let is_local = primary_node == "local" || primary_node == my_node_id;

                if is_local {
                    // Export from LOCAL physical shard
                    if let Ok(db) = state_storage.get_database(&db_name_clone) {
                        if let Ok(phys_coll) = db.get_collection(&physical_name) {
                             // Scan documents (load all into memory - current limitation)
                             let docs = phys_coll.scan(None);

                             for doc in docs {
                                 // Yield document line
                                 let mut val = doc.to_value();
                                 if let Some(obj) = val.as_object_mut() {
                                     if let Some(ref config) = shard_config {
                                          obj.insert("_shardConfig".to_string(), serde_json::to_value(config).unwrap_or_default());
                                     }
                                 }
                                 if let Ok(json) = serde_json::to_string(&val) {
                                     yield Ok::<_, std::io::Error>(axum::body::Bytes::from(format!("{}\n", json)));
                                 }

                                 // Yield blob chunks if blob collection
                                 if is_blob {
                                     let key = &doc.key;
                                     let mut chunk_index: u32 = 0;
                                     loop {
                                         match phys_coll.get_blob_chunk(key, chunk_index) {
                                             Ok(Some(data)) => {
                                                 let chunk_header = serde_json::json!({
                                                     "_type": "blob_chunk",
                                                     "_doc_key": key,
                                                     "_chunk_index": chunk_index,
                                                     "_data_length": data.len()
                                                 });

                                                 if let Ok(header_json) = serde_json::to_string(&chunk_header) {
                                                     // Header line
                                                     yield Ok(axum::body::Bytes::from(format!("{}\n", header_json)));
                                                     // Binary data
                                                     yield Ok(axum::body::Bytes::from(data));
                                                     // Trailing newline delimiter
                                                     yield Ok(axum::body::Bytes::from("\n"));
                                                 }
                                                 chunk_index += 1;
                                             },
                                             Ok(None) => break,
                                             Err(e) => {
                                                 tracing::error!("Failed to read blob chunk {} for {}: {}", chunk_index, key, e);
                                                 break;
                                             }
                                         }
                                     }
                                 }
                             }
                        }
                    }
                } else {
                    // Export from REMOTE physical shard
                    if let Some(mgr) = &cluster_manager_opt {
                        if let Some(addr) = mgr.get_node_api_address(&primary_node) {
                            let url = format!("http://{}/_api/database/{}/collection/{}/export", addr, db_name_clone, physical_name);
                            tracing::info!("Exporting remote shard {} from {}", physical_name, addr);

                            let req = client.get(&url)
                                .header("X-Shard-Direct", "true")
                                .header("X-Cluster-Secret", &secret_env);

                            // Stream the response
                            match req.send().await {
                                Ok(mut res) => {
                                    if res.status().is_success() {
                                        loop {
                                            match res.chunk().await {
                                                Ok(Some(bytes)) => yield Ok(bytes),
                                                Ok(None) => break,
                                                Err(e) => {
                                                    tracing::error!("Error reading remote stream: {}", e);
                                                    break;
                                                }
                                            }
                                        }
                                    } else {
                                        tracing::error!("Remote export failed: {}", res.status());
                                    }
                                },
                                Err(e) => tracing::error!("Remote request failed: {}", e),
                            }
                        }
                    }
                }
            }

        } else {
            // NON-SHARDED: Existing logic (scan logical collection)
            // Note: Logical collection matches physical for non-sharded
            let docs = collection_clone.scan(None);

            for doc in docs {
                let mut val = doc.to_value();
            if let Some(obj) = val.as_object_mut() {
                if let Some(ref config) = shard_config {
                     obj.insert("_shardConfig".to_string(), serde_json::to_value(config).unwrap_or_default());
                }
                // Export collection type so restore knows how to create it
                obj.insert("_collectionType".to_string(), Value::String(if is_blob { "blob".to_string() } else { "document".to_string() }));
            }
            if let Ok(json) = serde_json::to_string(&val) {
                yield Ok::<_, std::io::Error>(axum::body::Bytes::from(format!("{}\n", json)));
            }

            // For blob collections, also export the blob chunks
            if is_blob {
                {
                    let coll = &collection_clone;
                     let key = &doc.key;
                     // Iterate chunks until none found
                     let mut chunk_index: u32 = 0;
                     loop {
                         match coll.get_blob_chunk(key, chunk_index) {
                             Ok(Some(data)) => {
                                 // Create a specific chunk document
                                 let chunk_doc = serde_json::json!( {
                                     "_type": "blob_chunk",
                                     "_collectionType": "blob", // redundant but helpful context
                                     "_doc_key": key,
                                     "_chunk_index": chunk_index,
                                     "_data_length": data.len() // Required for binary reading
                                 });

                                 if let Ok(chunk_json) = serde_json::to_string(&chunk_doc) {
                                     yield Ok(axum::body::Bytes::from(format!("{}\n", chunk_json)));
                                 }

                                 yield Ok(axum::body::Bytes::from(data));
                                 yield Ok(axum::body::Bytes::from("\n")); // Newline delimiter for binary

                                 chunk_index += 1;
                             },
                             Ok(None) => break, // No more chunks
                             Err(e) => {
                                 tracing::error!("Failed to read blob chunk {} for {}: {}", chunk_index, key, e);
                                 break;
                             }
                         }
                     }
                }
            }
            }
        }
    };

    let body = Body::from_stream(stream);

    Response::builder()
        .header("Content-Type", "application/x-ndjson")
        .header(
            "Content-Disposition",
            format!(
                "attachment; filename=\"{}-{}.jsonl\"",
                sanitize_filename(&db_name),
                sanitize_filename(&coll_name)
            ),
        )
        .body(body)
        .map_err(|e| DbError::InternalError(format!("Failed to build response: {}", e)))
}

pub async fn import_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    mut multipart: Multipart,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name).or_else(|_| {
        // Auto-create
        tracing::info!("Auto-creating collection '{}' during import", coll_name);
        database.create_collection(coll_name.clone(), None)?;
        database.get_collection(&coll_name)
    })?;

    // Check sharding config once
    let shard_config = collection.get_shard_config();
    let _is_sharded = shard_config
        .as_ref()
        .map(|c| c.num_shards > 0)
        .unwrap_or(false);

    let mut imported_count = 0;
    let mut failed_count = 0;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| DbError::BadRequest(e.to_string()))?
    {
        if field.name() == Some("file") {
            let mut stream = field;
            let mut buffer = Vec::new();

            // Read first chunk to detect format
            if let Some(Ok(first_chunk)) = stream.next().await {
                buffer.extend_from_slice(&first_chunk);
            } else {
                continue; // Empty file
            }

            let first_char = buffer
                .iter()
                .find(|&&b| !b.is_ascii_whitespace())
                .copied()
                .unwrap_or(b' ');

            if first_char == b'{' {
                // Streaming Mode (JSONL / Mixed Binary)
                let mut batch_docs: Vec<Value> = Vec::with_capacity(1000);

                loop {
                    // Try to extract lines from buffer
                    while let Some(newline_pos) = buffer.iter().position(|&b| b == b'\n') {
                        let line_bytes: Vec<u8> = buffer.drain(0..=newline_pos).collect();
                        let line_slice = &line_bytes[..line_bytes.len() - 1]; // Trim newline

                        if line_slice.iter().all(|b| b.is_ascii_whitespace()) {
                            continue;
                        }

                        // Try parsing JSON
                        match serde_json::from_slice::<Value>(line_slice) {
                            Ok(doc) => {
                                // Check for Blob Chunk Header
                                let is_blob_chunk = doc
                                    .get("_type")
                                    .and_then(|t| t.as_str())
                                    .map(|t| t == "blob_chunk")
                                    .unwrap_or(false);

                                if is_blob_chunk {
                                    // Handle Binary Chunk
                                    if let Some(data_len) =
                                        doc.get("_data_length").and_then(|v| v.as_u64())
                                    {
                                        let required_len = data_len as usize;
                                        let total_required = required_len + 1; // +1 for trailing newline

                                        // Ensure we have enough bytes
                                        while buffer.len() < total_required {
                                            match stream.next().await {
                                                Some(Ok(chunk)) => buffer.extend_from_slice(&chunk),
                                                Some(Err(e)) => {
                                                    return Err(DbError::BadRequest(e.to_string()))
                                                }
                                                None => {
                                                    return Err(DbError::BadRequest(
                                                        "Unexpected EOF reading binary chunk"
                                                            .to_string(),
                                                    ))
                                                }
                                            }
                                        }

                                        // Extract binary data
                                        let chunk_data: Vec<u8> =
                                            buffer.drain(0..required_len).collect();
                                        // Consume trailing newline
                                        if !buffer.is_empty() && buffer[0] == b'\n' {
                                            buffer.drain(0..1);
                                        }

                                        // Put chunk (Directly, chunks are not batched usually)
                                        if let (Some(key), Some(index)) = (
                                            doc.get("_doc_key").and_then(|s| s.as_str()),
                                            doc.get("_chunk_index").and_then(|n| n.as_u64()),
                                        ) {
                                            match collection.put_blob_chunk(
                                                key,
                                                index as u32,
                                                &chunk_data,
                                            ) {
                                                Ok(_) => imported_count += 1,
                                                Err(e) => {
                                                    tracing::error!(
                                                        "Failed to import blob chunk: {}",
                                                        e
                                                    );
                                                    failed_count += 1;
                                                }
                                            }
                                        }
                                    } else {
                                        // Legacy Base64 chunk or other format
                                        if let (Some(key), Some(index), Some(data_b64)) = (
                                            doc.get("_doc_key").and_then(|s| s.as_str()),
                                            doc.get("_chunk_index").and_then(|n| n.as_u64()),
                                            doc.get("_blob_data").and_then(|s| s.as_str()),
                                        ) {
                                            if let Ok(data) =
                                                general_purpose::STANDARD.decode(data_b64)
                                            {
                                                match collection.put_blob_chunk(
                                                    key,
                                                    index as u32,
                                                    &data,
                                                ) {
                                                    Ok(_) => imported_count += 1,
                                                    Err(e) => {
                                                        tracing::error!(
                                                            "Failed to import blob chunk: {}",
                                                            e
                                                        );
                                                        failed_count += 1;
                                                    }
                                                }
                                            }
                                        }
                                    }
                                } else {
                                    // Regular Document
                                    batch_docs.push(doc);
                                    if batch_docs.len() >= 1000 {
                                        let batch_len = batch_docs.len();
                                        let result = collection.insert_batch(batch_docs.clone());
                                        match result {
                                            Ok(_) => imported_count += batch_len,
                                            Err(e) => {
                                                tracing::error!("Batch import error: {}", e);
                                                failed_count += batch_len; // Simplified error tracking
                                            }
                                        }
                                        batch_docs.clear();
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse JSON line during import: {}", e);
                                failed_count += 1;
                            }
                        }
                    }

                    // Read next chunk
                    match stream.next().await {
                        Some(Ok(chunk)) => buffer.extend_from_slice(&chunk),
                        Some(Err(e)) => return Err(DbError::BadRequest(e.to_string())),
                        None => break, // EOF
                    }
                }

                // Process remaining documents
                if !batch_docs.is_empty() {
                    let batch_len = batch_docs.len();
                    let result = collection.insert_batch(batch_docs);
                    match result {
                        Ok(_) => imported_count += batch_len,
                        Err(e) => {
                            tracing::error!("Batch import error: {}", e);
                            failed_count += batch_len;
                        }
                    }
                }
            } else {
                // Legacy Import Mode (Whole Array)
                // This assumes the file is small enough to fit in memory
                // or the chunks allow it.
                // Reconstruct the full body
                let mut full_body = buffer;
                while let Some(chunk_res) = stream.next().await {
                    let chunk = chunk_res.map_err(|e| DbError::BadRequest(e.to_string()))?;
                    full_body.extend_from_slice(&chunk);
                }

                let json: Value = serde_json::from_slice(&full_body)
                    .map_err(|e| DbError::BadRequest(format!("Invalid JSON: {}", e)))?;

                if let Some(arr) = json.as_array() {
                    let batch_docs = arr.clone();
                    let batch_len = batch_docs.len();
                    let result = collection.insert_batch(batch_docs);
                    match result {
                        Ok(_) => imported_count += batch_len,
                        Err(e) => {
                            return Err(e);
                        }
                    }
                } else if let Some(_obj) = json.as_object() {
                    // Single document import
                    collection.insert(json)?;
                    imported_count += 1;
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "status": "imported",
        "count": imported_count,
        "failed": failed_count
    })))
}
