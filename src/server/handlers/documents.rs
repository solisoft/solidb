use super::system::{is_physical_shard_collection, is_protected_collection, AppState};
use crate::{
    error::DbError,
    server::response::ApiResponse,
    sync::{LogEntry, Operation},
    transaction::TransactionId,
    triggers::{fire_collection_triggers, TriggerEvent},
};
use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::Json,
};
use serde::Deserialize;
use serde_json::Value;

// ==================== Helper Functions ====================

pub fn get_transaction_id(headers: &HeaderMap) -> Option<TransactionId> {
    headers
        .get("X-Transaction-ID")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| {
            // Support "tx:123" or just "123"
            let id_str = s.strip_prefix("tx:").unwrap_or(s);
            id_str.parse::<u64>().ok()
        })
        .map(TransactionId::from_u64)
}

// ==================== Structs ====================

/// Copy shard data from a source node (used for healing)
#[derive(Debug, Deserialize)]
pub struct CopyShardRequest {
    pub source_address: String,
}

// ==================== Handlers ====================

pub async fn insert_document(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check for transaction context
    if let Some(tx_id) = get_transaction_id(&headers) {
        let tx_manager = state.storage.transaction_manager()?;
        let tx_arc = tx_manager.get(tx_id)?;
        let mut tx = tx_arc
            .write()
            .map_err(|_| DbError::InternalError("Transaction lock poisoned".into()))?;
        let wal = tx_manager.wal();

        let doc = collection.insert_tx(&mut tx, wal, data)?;

        // No replication log for transactional write yet (will happen on commit)

        return Ok(Json(doc.to_value()));
    }

    // Check for sharding
    // If sharded and we have a coordinator, use it
    if let Some(shard_config) = collection.get_shard_config() {
        tracing::info!(
            "[INSERT] shard_config found: num_shards={}",
            shard_config.num_shards
        );
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                // Check for direct shard access (prevention of infinite loops)
                if !headers.contains_key("X-Shard-Direct") {
                    tracing::info!(
                        "[INSERT] Using ShardCoordinator for {}/{}",
                        db_name,
                        coll_name
                    );
                    let doc = coordinator
                        .insert(&db_name, &coll_name, &shard_config, data)
                        .await?;

                    // NOTE: Don't add to replication log here!
                    // If we forwarded to another node, that node adds to its log.
                    // If we stored locally (we're the primary), ShardCoordinator already
                    // returned from collection.insert() which doesn't add to log -
                    // but the X-Shard-Direct path on the primary handles replication.
                    // So replication log entry is only added by the PRIMARY node via X-Shard-Direct path.

                    return Ok(Json(doc));
                }
                // If X-Shard-Direct header present, fall through to direct insert (replica receiving forwarded data)
            } else {
                // Sharded collection but no coordinator - this is an error state
                tracing::error!(
                    "[INSERT] Sharded collection {}/{} but no shard_coordinator available!",
                    db_name,
                    coll_name
                );
                return Err(DbError::InternalError(
                    "Sharded collection requires ShardCoordinator".to_string(),
                ));
            }
        }
    }

    // Only reach here for:
    // 1. Non-sharded collections
    // 2. Sharded with X-Shard-Direct header (PRIMARY receiving forwarded insert)
    let doc = collection.insert(data)?;

    // Add to replication log ONLY for non-sharded collections
    // Physical shard collections are partitioned across the cluster - do NOT replicate them
    // to all nodes (that would defeat the purpose of sharding for horizontal scaling)
    let is_shard = is_physical_shard_collection(&coll_name);
    if !is_shard {
        if let Some(ref log) = state.replication_log {
            let entry = LogEntry {
                sequence: 0,
                node_id: "".to_string(),
                database: db_name.clone(),
                collection: coll_name.clone(),
                operation: Operation::Insert,
                key: doc.key.clone(),
                data: serde_json::to_vec(&doc.to_value()).ok(),
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            };
            let _ = log.append(entry);
        }
    }

    // Fire triggers for the insert
    if !coll_name.starts_with('_') {
        let notifier = state.queue_worker.as_ref().map(|w| w.notifier());
        let _ = fire_collection_triggers(
            &state.storage,
            notifier.as_ref(),
            &db_name,
            &coll_name,
            TriggerEvent::Insert,
            &doc,
            None,
        );
    }

    Ok(Json(doc.to_value()))
}

/// Batch insert endpoint for internal shard forwarding
/// Accepts an array of documents and inserts them all in one request
pub async fn insert_documents_batch(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(documents): Json<Vec<Value>>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // This is always a direct shard operation (internal API)
    // X-Shard-Direct should be required
    if !headers.contains_key("X-Shard-Direct") {
        return Err(DbError::BadRequest(
            "Batch endpoint requires X-Shard-Direct header".to_string(),
        ));
    }

    // Use upsert for physical shard collections (prevents duplicates during resharding)
    // Physical shards have names like "users_s0", "users_s1", etc.
    let is_physical_shard = coll_name.contains("_s")
        && coll_name
            .chars()
            .last()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);

    let insert_count = if is_physical_shard {
        // Convert documents to (key, doc) pairs for upsert
        let keyed_docs: Vec<(String, Value)> = documents
            .iter()
            .map(|doc| {
                let key = doc
                    .get("_key")
                    .and_then(|k| k.as_str())
                    .unwrap_or("")
                    .to_string();
                (key, doc.clone())
            })
            .filter(|(key, _)| !key.is_empty())
            .collect();

        collection.upsert_batch(keyed_docs)?
    } else {
        collection.insert_batch(documents.clone())?.len()
    };

    // NOTE: Do NOT log to replication log for sharded data!
    // This endpoint is for internal shard operations (X-Shard-Direct).
    // Each node only stores its assigned shards - data is partitioned across the cluster.

    // Forward to replica nodes if this is a primary shard
    // Parse shard ID from collection name (e.g., "users_s0" -> shard 0)
    // IMPORTANT: Skip replica forwarding during migrations to prevent duplication
    let is_migration = headers.contains_key("X-Migration");
    if is_migration {
        tracing::debug!(
            "BATCH: Skipping replica forwarding - migration operation for {}/{}",
            db_name,
            coll_name
        );
    } else if let Some(ref coordinator) = state.shard_coordinator {
        // Check if coordinator is currently rebalancing - skip replica forwarding during resharding
        // to prevent timeouts and deadlocks
        let is_rebalancing = coordinator.is_rebalancing();
        if is_rebalancing {
            tracing::debug!(
                "BATCH: Skipping replica forwarding during rebalancing for {}/{}",
                db_name,
                coll_name
            );
        } else {
            // Extract base collection name and shard ID
            if let Some(idx) = coll_name.rfind("_s") {
                let base_coll = &coll_name[..idx];
                if let Ok(shard_id) = coll_name[idx + 2..].parse::<u16>() {
                    // Get shard table to find replica nodes
                    if let Some(table) = coordinator.get_shard_table(&db_name, base_coll) {
                        if let Some(assignment) = table.assignments.get(&shard_id) {
                            if !assignment.replica_nodes.is_empty() {
                                // Forward to replicas in parallel
                                let client = reqwest::Client::new();
                                let secret = state.cluster_secret();

                                if let Some(ref cluster_manager) = state.cluster_manager {
                                    let mut futures = Vec::new();

                                    for replica_node in &assignment.replica_nodes {
                                        if let Some(addr) =
                                            cluster_manager.get_node_api_address(replica_node)
                                        {
                                            let url = format!(
                                                "http://{}/_api/database/{}/document/{}/_replica",
                                                addr, db_name, coll_name
                                            );
                                            tracing::debug!("REPLICA FWD: Forwarding {} docs to replica {} at {}", documents.len(), replica_node, addr);

                                            let client = client.clone();
                                            let secret = secret.clone();
                                            let docs = documents.clone();

                                            let future = async move {
                                                let _ = tokio::time::timeout(
                                                    std::time::Duration::from_secs(10), // 10 second timeout for replicas
                                                    client
                                                        .post(&url)
                                                        .header("X-Shard-Direct", "true")
                                                        .header("X-Cluster-Secret", &secret)
                                                        .json(&docs)
                                                        .send(),
                                                )
                                                .await;
                                            };
                                            futures.push(future);
                                        }
                                    }

                                    // Fire and forget - don't wait for replicas
                                    tokio::spawn(async move {
                                        futures::future::join_all(futures).await;
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(Json(serde_json::json!({
        "inserted": insert_count,
        "success": true
    })))
}

/// Replica insert endpoint - stores documents without further forwarding
/// This is called by primary nodes to replicate data to their replicas
pub async fn insert_documents_replica(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    headers: HeaderMap,
    Json(documents): Json<Vec<Value>>,
) -> Result<Json<Value>, DbError> {
    // Require X-Shard-Direct header
    if !headers.contains_key("X-Shard-Direct") {
        return Err(DbError::BadRequest(
            "Replica endpoint requires X-Shard-Direct header".to_string(),
        ));
    }

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Use upsert to prevent duplicates (replicas may already have some data)
    // Convert documents to (key, doc) pairs for upsert
    let keyed_docs: Vec<(String, Value)> = documents
        .iter()
        .map(|doc| {
            let key = doc
                .get("_key")
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .to_string();
            (key, doc.clone())
        })
        .filter(|(key, _)| !key.is_empty())
        .collect();

    let insert_count = collection.upsert_batch(keyed_docs)?;

    tracing::debug!(
        "REPLICA: Stored {} docs for {}/{}",
        insert_count,
        db_name,
        coll_name
    );

    Ok(Json(serde_json::json!({
        "inserted": insert_count,
        "success": true
    })))
}

/// Verify that documents exist in a collection
/// Used by migration to confirm documents arrived before deleting from source
/// POST /_api/database/{db}/document/{coll}/_verify
/// Body: { "keys": ["key1", "key2", ...] }
/// Returns: { "found": ["key1"], "missing": ["key2"], "total_checked": 2 }
pub async fn verify_documents_exist(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(request): Json<serde_json::Value>,
) -> Result<Json<Value>, DbError> {
    let keys = request
        .get("keys")
        .and_then(|k| k.as_array())
        .ok_or_else(|| DbError::BadRequest("Missing 'keys' array in request body".to_string()))?;

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    let mut found: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();

    for key_value in keys {
        if let Some(key) = key_value.as_str() {
            match collection.get(key) {
                Ok(_) => found.push(key.to_string()),
                Err(_) => missing.push(key.to_string()),
            }
        }
    }

    let total_checked = found.len() + missing.len();
    tracing::debug!(
        "VERIFY: Checked {} docs in {}/{}: {} found, {} missing",
        total_checked,
        db_name,
        coll_name,
        found.len(),
        missing.len()
    );

    Ok(Json(serde_json::json!({
        "found": found,
        "missing": missing,
        "total_checked": total_checked
    })))
}

pub async fn copy_shard_data(
    State(state): State<AppState>,
    Path((db_name, coll_name)): Path<(String, String)>,
    Json(request): Json<CopyShardRequest>,
) -> Result<Json<Value>, DbError> {
    tracing::info!(
        "COPY_SHARD: Copying {}/{} from {}",
        db_name,
        coll_name,
        request.source_address
    );

    // Step 1: Check Source Count using Metadata API
    let secret = state.cluster_secret();
    let client = reqwest::Client::new();

    // Get doc count first to avoid massive transfer if already in sync
    let meta_url = format!(
        "http://{}/_api/database/{}/collection/{}",
        request.source_address, db_name, coll_name
    );
    let meta_res = client
        .get(&meta_url)
        .header("X-Cluster-Secret", &secret)
        .header("X-Shard-Direct", "true")
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await;

    let mut source_count = 0;
    let mut check_count = false;

    if let Ok(res) = meta_res {
        if res.status().is_success() {
            if let Ok(json) = res.json::<serde_json::Value>().await {
                if let Some(c) = json.get("count").and_then(|v| v.as_u64()) {
                    source_count = c as usize;
                    check_count = true;
                }
            }
        }
    }

    // Ensure collection exists locally
    let database = state.storage.get_database(&db_name)?;
    let collection = match database.get_collection(&coll_name) {
        Ok(c) => c,
        Err(_) => {
            database.create_collection(coll_name.clone(), None)?;
            database.get_collection(&coll_name)?
        }
    };

    // Skip if in sync (count matches)
    if check_count {
        let local_count = collection.count();
        if local_count == source_count {
            // Already in sync
            tracing::info!(
                "COPY_SHARD: Skipping sync for {}/{} (Count match: {})",
                db_name,
                coll_name,
                local_count
            );
            return Ok(Json(serde_json::json!({
                "copied": 0,
                "success": true,
                "skipped": true
            })));
        }
        tracing::info!(
            "COPY_SHARD: Count mismatch for {}/{} (Local: {}, Source: {}). Truncating before sync.",
            db_name,
            coll_name,
            local_count,
            source_count
        );
        let _ = collection.truncate();
    }

    // Query all documents from source shard
    let url = format!(
        "http://{}/_api/database/{}/cursor",
        request.source_address, db_name
    );
    let query = format!("FOR doc IN {} RETURN doc", coll_name);
    // Reuse secret from above or fetch again
    // let secret = state.cluster_secret();
    // We already have 'secret' and 'client' in scope from earlier meta check block

    let res = client
        .post(&url)
        .header("X-Cluster-Secret", &secret)
        .json(&serde_json::json!({ "query": query }))
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| DbError::InternalError(format!("Request failed: {}", e)))?;

    if !res.status().is_success() {
        let status = res.status();
        let body_text = res
            .text()
            .await
            .unwrap_or_else(|_| "Could not read error body".to_string());
        tracing::error!(
            "COPY_SHARD: Source query failed. Status: {}, Body: {}",
            status,
            body_text
        );
        return Err(DbError::InternalError(format!(
            "Source query failed: {}. Body: {}",
            status, body_text
        )));
    }

    let body: serde_json::Value = res
        .json()
        .await
        .map_err(|e| DbError::InternalError(format!("Parse failed: {}", e)))?;

    let docs = body
        .get("result")
        .and_then(|r| r.as_array())
        .ok_or_else(|| DbError::InternalError("No result array".to_string()))?;

    // Use upsert to prevent duplicates (shard may already have some data)
    let keyed_docs: Vec<(String, serde_json::Value)> = docs
        .iter()
        .map(|doc| {
            let key = doc
                .get("_key")
                .and_then(|k| k.as_str())
                .unwrap_or("")
                .to_string();
            (key, doc.clone())
        })
        .filter(|(key, _)| !key.is_empty())
        .collect();
    let count = keyed_docs.len();
    collection.upsert_batch(keyed_docs)?;

    tracing::info!(
        "COPY_SHARD: Copied {} docs to {}/{}",
        count,
        db_name,
        coll_name
    );

    Ok(Json(serde_json::json!({
        "copied": count,
        "success": true
    })))
}

pub async fn get_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<ApiResponse<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check for sharding
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                let doc = coordinator.get(&db_name, &coll_name, &key).await?;

                let mut doc_value = doc;
                let replicas = coordinator.get_replicas(&key, &shard_config);
                if let Value::Object(ref mut map) = doc_value {
                    map.insert("_replicas".to_string(), serde_json::json!(replicas));
                }

                return Ok(ApiResponse::new(doc_value, &headers));
            }
        }
    }

    let doc = collection.get(&key)?;
    Ok(ApiResponse::new(doc.to_value(), &headers))
}

pub async fn update_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
    headers: HeaderMap,
    Query(params): Query<std::collections::HashMap<String, String>>,
    Json(mut data): Json<Value>,
) -> Result<Json<Value>, DbError> {
    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check for upsert query param
    let upsert = params.get("upsert").map(|v| v == "true").unwrap_or(false);

    // Check for transaction context
    if let Some(tx_id) = get_transaction_id(&headers) {
        let tx_manager = state.storage.transaction_manager()?;
        let tx_arc = tx_manager.get(tx_id)?;
        let mut tx = tx_arc
            .write()
            .map_err(|_| DbError::InternalError("Transaction lock poisoned".into()))?;
        let wal = tx_manager.wal();

        let doc = collection.update_tx(&mut tx, wal, &key, data)?;
        return Ok(Json(doc.to_value()));
    }

    // Check for sharding
    // If sharded and we have a coordinator, use it
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                // Check for direct shard access
                if !headers.contains_key("X-Shard-Direct") {
                    let doc = coordinator
                        .update(&db_name, &coll_name, &shard_config, &key, data)
                        .await?;
                    return Ok(Json(doc));
                }
            }
        }
    }

    // Get old document for trigger (before update)
    let old_doc_value = collection.get(&key).ok().map(|d| d.to_value());

    // Try update, or insert if upsert=true and document not found
    let (doc, was_upsert) = match collection.update(&key, data.clone()) {
        Ok(doc) => (doc, false),
        Err(DbError::DocumentNotFound(_)) if upsert => {
            // Ensure _key is set for insert
            if let Value::Object(ref mut obj) = data {
                obj.insert("_key".to_string(), Value::String(key.clone()));
            }
            (collection.insert(data)?, true)
        }
        Err(e) => return Err(e),
    };

    // Record to replication log ONLY for non-sharded collections
    // Physical shard collections are partitioned across the cluster - do NOT replicate them
    // to all nodes (that would defeat the purpose of sharding for horizontal scaling)
    let is_shard = is_physical_shard_collection(&coll_name);
    let is_sharded_logical = collection.get_shard_config().is_some();
    if !is_shard && !is_sharded_logical {
        if let Some(ref log) = state.replication_log {
            let entry = LogEntry {
                sequence: 0,
                node_id: "".to_string(),
                database: db_name.clone(),
                collection: coll_name.clone(),
                operation: Operation::Update,
                key: doc.key.clone(),
                data: serde_json::to_vec(&doc.to_value()).ok(),
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            };
            let _ = log.append(entry);
        }
    }

    // Fire triggers for the update (or insert if upsert)
    if !coll_name.starts_with('_') {
        let notifier = state.queue_worker.as_ref().map(|w| w.notifier());
        let event = if was_upsert {
            TriggerEvent::Insert
        } else {
            TriggerEvent::Update
        };
        let _ = fire_collection_triggers(
            &state.storage,
            notifier.as_ref(),
            &db_name,
            &coll_name,
            event,
            &doc,
            old_doc_value.as_ref(),
        );
    }

    Ok(Json(doc.to_value()))
}

pub async fn delete_document(
    State(state): State<AppState>,
    Path((db_name, coll_name, key)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, DbError> {
    // Protect system collections from direct document deletion
    if is_protected_collection(&db_name, &coll_name) {
        return Err(DbError::BadRequest(format!(
            "Cannot delete documents from protected collection: {}",
            coll_name
        )));
    }

    let database = state.storage.get_database(&db_name)?;
    let collection = database.get_collection(&coll_name)?;

    // Check for transaction context
    if let Some(tx_id) = get_transaction_id(&headers) {
        let tx_manager = state.storage.transaction_manager()?;
        let tx_arc = tx_manager.get(tx_id)?;
        let mut tx = tx_arc
            .write()
            .map_err(|_| DbError::InternalError("Transaction lock poisoned".into()))?;
        let wal = tx_manager.wal();

        collection.delete_tx(&mut tx, wal, &key)?;
        return Ok(StatusCode::NO_CONTENT);
    }

    // Check for sharding
    if let Some(shard_config) = collection.get_shard_config() {
        if shard_config.num_shards > 0 {
            if let Some(ref coordinator) = state.shard_coordinator {
                if !headers.contains_key("X-Shard-Direct") {
                    coordinator
                        .delete(&db_name, &coll_name, &shard_config, &key)
                        .await?;
                    return Ok(StatusCode::NO_CONTENT);
                }
            }
        }
    }

    // Get document before deletion (for trigger)
    let old_doc = collection.get(&key).ok();

    collection.delete(&key)?;

    // If this is a blob collection, trigger compaction to reclaim space from deleted chunks immediately
    if collection.get_type() == "blob" {
        tracing::info!(
            "Compacting blob collection {}/{} after deletion of {}",
            db_name,
            coll_name,
            key
        );
        collection.compact();
    }

    // Record to replication log ONLY for non-sharded collections
    // Physical shard collections are partitioned across the cluster - do NOT replicate them
    // to all nodes (that would defeat the purpose of sharding for horizontal scaling)
    let is_shard = is_physical_shard_collection(&coll_name);
    let is_sharded_logical = collection.get_shard_config().is_some();
    if !is_shard && !is_sharded_logical {
        if let Some(ref log) = state.replication_log {
            let entry = LogEntry {
                sequence: 0,
                node_id: state
                    .cluster_manager
                    .as_ref()
                    .map(|m| m.local_node_id())
                    .unwrap_or_else(|| "".to_string()),
                database: db_name.clone(),
                collection: coll_name.clone(),
                operation: Operation::Delete,
                key: key.clone(),
                data: None,
                timestamp: chrono::Utc::now().timestamp_millis() as u64,
                origin_sequence: None,
            };
            let _ = log.append(entry);
        }
    }

    // Fire triggers for the delete
    if !coll_name.starts_with('_') {
        if let Some(old_doc) = old_doc {
            let notifier = state.queue_worker.as_ref().map(|w| w.notifier());
            let old_doc_value = old_doc.to_value();
            let _ = fire_collection_triggers(
                &state.storage,
                notifier.as_ref(),
                &db_name,
                &coll_name,
                TriggerEvent::Delete,
                &old_doc,
                Some(&old_doc_value),
            );
        }
    }

    Ok(StatusCode::NO_CONTENT)
}
