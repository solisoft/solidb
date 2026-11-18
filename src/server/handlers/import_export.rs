use super::system::{sanitize_filename, AppState};
use crate::error::DbError;
use crate::storage::http_client::get_http_client;
use axum::{
    body::Body,
    extract::{Multipart, Path, Query, State},
    http::HeaderMap,
    response::{Json, Response},
};
use base64::{engine::general_purpose, Engine as _};
use futures::stream::StreamExt;
use serde::Deserialize;
use serde_json::Value;

const MAX_BLOB_CHUNK_SIZE: usize = 16 * 1024 * 1024; // 16 MiB

#[derive(Debug, Default, Deserialize)]
pub struct ImportQuery {
    /// Import mode: "insert" (default) or "upsert".
    /// Upsert updates existing `_key`s instead of failing the batch.
    #[serde(default)]
    pub mode: Option<String>,
    /// `defer`: the references of the documents are not looked up as they
    /// are written, so that children may come before their parents, in
    /// this file or in a later import; once the file is in, the
    /// collection's foreign keys are checked and the answer lists the
    /// orphans (`orphans`). Nothing is taken back. Admin of the database.
    #[serde(default)]
    pub foreign_keys: Option<String>,
}

/// Apply a batch of documents using insert or upsert semantics.
fn import_doc_batch(
    collection: &crate::storage::Collection,
    batch_docs: Vec<Value>,
    (upsert, defer): (bool, bool),
) -> Result<usize, DbError> {
    if defer {
        return crate::storage::foreign_keys::deferring(|| {
            import_doc_batch(collection, batch_docs, (upsert, false))
        });
    }
    let batch_len = batch_docs.len();
    if upsert {
        let mut pairs: Vec<(String, Value)> = Vec::with_capacity(batch_len);
        for mut doc in batch_docs {
            let key = if let Some(obj) = doc.as_object_mut() {
                if let Some(k) = obj.get("_key").and_then(|v| v.as_str()) {
                    k.to_string()
                } else {
                    let k = uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string();
                    obj.insert("_key".to_string(), Value::String(k.clone()));
                    k
                }
            } else {
                uuid::Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)).to_string()
            };
            pairs.push((key, doc));
        }
        collection.upsert_batch(pairs)?;
        Ok(batch_len)
    } else {
        collection.insert_batch(batch_docs)?;
        Ok(batch_len)
    }
}

/// `?format=parquet|arrow` on an export; JSON lines without it.
#[derive(Debug, Default, Deserialize)]
pub struct ExportQuery {
    #[serde(default)]
    pub format: Option<String>,
}

pub async fn export_collection(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Query(query): Query<ExportQuery>,
    _headers: HeaderMap,
) -> Result<Response, DbError> {
    if let Some(format) = query.format.as_deref().filter(|f| *f != "jsonl") {
        return export_typed(state, db_name, coll_name, format, false).await;
    }
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
                            let url = crate::cluster::http::peer_url(&addr, &format!("/_api/database/{}/collection/{}/export", db_name_clone, physical_name));
                            tracing::info!("Exporting remote shard {} from {}", physical_name, addr);

                            let req = client.get(&url)
                                // A whole shard can outlast the shared client's 60 s total.
                                .timeout(crate::cluster::http::stream_timeout())
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
    Query(query): Query<ImportQuery>,
    claims: Option<axum::Extension<crate::server::auth::Claims>>,
    mut multipart: Multipart,
) -> Result<Json<Value>, DbError> {
    let upsert = query
        .mode
        .as_deref()
        .map(|m| m.eq_ignore_ascii_case("upsert"))
        .unwrap_or(false);

    let defer = match query.foreign_keys.as_deref() {
        None => false,
        Some(mode) if mode.eq_ignore_ascii_case("defer") => true,
        Some(other) => {
            return Err(DbError::BadRequest(format!(
                "foreign_keys: '{}' is not a mode; the one there is is 'defer'",
                other
            )))
        }
    };
    // Leaving orphans behind is an administrator's choice.
    if let (true, Some(claims)) = (defer, claims.as_deref()) {
        crate::server::authz_middleware::enforce(
            claims,
            &state,
            crate::server::authorization::PermissionAction::Admin,
            Some(&db_name),
        )
        .await?;
    }
    let how = (upsert, defer);

    let database = state.storage.get_database(&db_name)?;
    // Import is a bulk write: the same tiers as the document API apply, or
    // `_scripts` could be installed by uploading a file.
    let actor = crate::server::handlers::query::write_actor_for_collection(
        &state,
        claims.as_deref(),
        &db_name,
        &coll_name,
    )
    .await;
    let collection = database
        .get_collection_for_write(&coll_name, actor)
        .or_else(|e| {
            if matches!(e, DbError::Forbidden(_)) {
                return Err(e);
            }
            // Auto-create
            tracing::info!("Auto-creating collection '{}' during import", coll_name);
            database.create_collection(coll_name.clone(), None)?;
            database.get_collection_for_write(&coll_name, actor)
        })?;

    // Check sharding config once
    let shard_config = collection.get_shard_config();
    let _is_sharded = shard_config
        .as_ref()
        .map(|c| c.num_shards > 0)
        .unwrap_or(false);

    let mut imported_count = 0;
    let mut failed_count = 0;
    // Why the first failed document or batch was refused: the answer used to
    // give a count only, and the reason was in the server's log.
    let mut first_error: Option<String> = None;

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
                                        if data_len > MAX_BLOB_CHUNK_SIZE as u64 {
                                            return Err(DbError::BadRequest(format!(
                                                "Blob chunk size {} exceeds maximum allowed size of {}",
                                                data_len,
                                                MAX_BLOB_CHUNK_SIZE
                                            )));
                                        }
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
                                                    first_error
                                                        .get_or_insert_with(|| e.to_string());
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
                                                        first_error
                                                            .get_or_insert_with(|| e.to_string());
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
                                        let to_write = std::mem::take(&mut batch_docs);
                                        match import_doc_batch(&collection, to_write, how) {
                                            Ok(n) => imported_count += n,
                                            Err(e) => {
                                                tracing::error!("Batch import error: {}", e);
                                                failed_count += batch_len;
                                                first_error.get_or_insert_with(|| e.to_string());
                                            }
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                tracing::warn!("Failed to parse JSON line during import: {}", e);
                                failed_count += 1;
                                first_error.get_or_insert_with(|| e.to_string());
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

                // A last line the file does not end with a newline. It used
                // to be dropped without a word: the documents before it were
                // counted and this one was neither imported nor failed.
                if !buffer.iter().all(|b| b.is_ascii_whitespace()) {
                    match serde_json::from_slice::<Value>(&buffer) {
                        Ok(doc)
                            if doc.get("_type").and_then(|t| t.as_str()) == Some("blob_chunk") =>
                        {
                            failed_count += 1;
                            first_error.get_or_insert_with(|| {
                                "the file ends on a blob chunk header with no data after it"
                                    .to_string()
                            });
                        }
                        Ok(doc) => batch_docs.push(doc),
                        Err(e) => {
                            tracing::warn!("Failed to parse JSON line during import: {}", e);
                            failed_count += 1;
                            first_error.get_or_insert_with(|| e.to_string());
                        }
                    }
                }

                // Process remaining documents
                if !batch_docs.is_empty() {
                    let batch_len = batch_docs.len();
                    match import_doc_batch(&collection, batch_docs, how) {
                        Ok(n) => imported_count += n,
                        Err(e) => {
                            tracing::error!("Batch import error: {}", e);
                            failed_count += batch_len;
                            first_error.get_or_insert_with(|| e.to_string());
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

                if let Value::Array(batch_docs) = json {
                    // Moved, not cloned: the parsed tree is already several
                    // times the size of the body it came from.
                    match import_doc_batch(&collection, batch_docs, how) {
                        Ok(n) => imported_count += n,
                        Err(e) => return Err(e),
                    }
                } else if json.is_object() {
                    // Single document import
                    import_doc_batch(&collection, vec![json], how)?;
                    imported_count += 1;
                }
            }
        }
    }

    let mut answer = serde_json::json!({
        "status": "imported",
        "count": imported_count,
        "failed": failed_count
    });
    if let Some(error) = first_error {
        answer["error"] = Value::String(error);
    }
    // The check a deferred load ends with: what each foreign key of the
    // collection finds without its parent, now that the file is in.
    if defer {
        let checked = collection.clone();
        let orphans = tokio::task::spawn_blocking(move || {
            checked
                .foreign_keys()
                .into_iter()
                .map(|key| {
                    let spec = crate::storage::collection::references::ForeignKeySpec {
                        field: key.field.clone(),
                        references: key.references.clone(),
                        on_delete: key.on_delete,
                        by: Some(key.by),
                        name: Some(key.name.clone()),
                        also: key.also.clone(),
                    };
                    checked.foreign_key_orphans(&spec).map(|found| {
                        serde_json::json!({
                            "foreign_key": key.name,
                            "field": key.field,
                            "references": key.references,
                            "count": found.count,
                            "sample": found.sample,
                        })
                    })
                })
                .collect::<Result<Vec<Value>, DbError>>()
        })
        .await
        .map_err(|e| DbError::InternalError(format!("Task error: {}", e)))??;
        answer["orphans"] = Value::Array(orphans);
    }
    Ok(Json(answer))
}

/// A collection exported as Parquet or an Arrow IPC stream (card #980),
/// written on a blocking thread and sent as it is written: nothing holds
/// more than a batch and a megabyte of output.
pub(crate) async fn export_typed(
    state: AppState,
    db_name: String,
    name: String,
    format: &str,
    columnar: bool,
) -> Result<Response, DbError> {
    #[cfg(feature = "export-arrow")]
    {
        use crate::storage::arrow_export::{export_collection, export_columnar, ExportFormat};
        let format = ExportFormat::parse(format).ok_or_else(|| {
            DbError::BadRequest(format!(
                "Unknown export format '{format}': jsonl, parquet or arrow"
            ))
        })?;
        let database = state.storage.get_database(&db_name)?;
        // Checked before the response starts: an error after it is a cut
        // stream.
        let source = if columnar {
            let col = crate::storage::ColumnarCollection::load(
                name.clone(),
                &db_name,
                database.db_arc(),
            )?;
            // A sharded collection's rows, gathered from its shards.
            match col.sharding() {
                Some(shards) => {
                    let (coord, db, n) = (
                        state.shard_coordinator.clone(),
                        database.clone(),
                        name.clone(),
                    );
                    let rows = tokio::task::spawn_blocking(move || {
                        crate::sharding::columnar::gather(coord.as_deref(), &db, &n, &shards)
                    })
                    .await
                    .map_err(|e| DbError::InternalError(format!("Task join error: {}", e)))??;
                    Err(col.gathered(rows)?)
                }
                None => Err(col),
            }
        } else {
            let collection = database.get_collection(&name)?;
            if collection
                .get_shard_config()
                .is_some_and(|c| c.num_shards > 0)
            {
                return Err(DbError::BadRequest(
                    "A sharded collection is exported as JSON lines only".to_string(),
                ));
            }
            if collection.get_type() == "blob" {
                return Err(DbError::BadRequest(
                    "A blob collection is exported as JSON lines only".to_string(),
                ));
            }
            Ok(collection)
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(8);
        tokio::task::spawn_blocking(move || {
            let out = ChannelWriter {
                tx: tx.clone(),
                buf: Vec::with_capacity(ChannelWriter::PIECE),
            };
            let done = match &source {
                Ok(collection) => export_collection(collection, format, out),
                Err(columnar) => export_columnar(columnar, format, out),
            };
            if let Err(e) = done {
                let _ = tx.blocking_send(Err(std::io::Error::other(e.to_string())));
            }
        });
        let stream = async_stream::stream! {
            while let Some(piece) = rx.recv().await {
                yield piece;
            }
        };
        let filename = sanitize_filename(&format!("{name}.{}", format.extension()));
        Response::builder()
            .header("Content-Type", format.content_type())
            .header(
                "Content-Disposition",
                format!("attachment; filename=\"{filename}\""),
            )
            .body(Body::from_stream(stream))
            .map_err(|e| DbError::InternalError(e.to_string()))
    }
    #[cfg(not(feature = "export-arrow"))]
    {
        let _ = (state, db_name, name, columnar);
        Err(DbError::BadRequest(format!(
            "Export format '{format}' needs a server built with the `export-arrow` feature"
        )))
    }
}

/// Sends what is written to it in pieces of [`Self::PIECE`]; the rest when
/// dropped. A piece the client no longer reads is an error, which stops the
/// export.
#[cfg(feature = "export-arrow")]
struct ChannelWriter {
    tx: tokio::sync::mpsc::Sender<Result<bytes::Bytes, std::io::Error>>,
    buf: Vec<u8>,
}

#[cfg(feature = "export-arrow")]
impl ChannelWriter {
    const PIECE: usize = 1 << 20;

    fn send(&mut self) -> std::io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let piece = bytes::Bytes::from(std::mem::replace(
            &mut self.buf,
            Vec::with_capacity(Self::PIECE),
        ));
        self.tx
            .blocking_send(Ok(piece))
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "client gone"))
    }
}

#[cfg(feature = "export-arrow")]
impl std::io::Write for ChannelWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        if self.buf.len() >= Self::PIECE {
            self.send()?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.send()
    }
}

#[cfg(feature = "export-arrow")]
impl Drop for ChannelWriter {
    fn drop(&mut self) {
        let _ = self.send();
    }
}
