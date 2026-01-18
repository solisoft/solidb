//! Batch operations for sharded collections
//!
//! This module handles batch insert and upsert operations with shard coordination.

#![allow(clippy::too_many_arguments, clippy::type_complexity, clippy::result_large_err)]

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::cluster::manager::ClusterManager;
use crate::sharding::coordinator::{CollectionShardConfig, ShardTable};
use crate::sharding::router::ShardRouter;
use crate::storage::StorageEngine;
use crate::DbError;

/// Insert a batch of documents with shard coordination
pub async fn insert_batch(
    storage: &Arc<StorageEngine>,
    cluster_manager: &Option<Arc<ClusterManager>>,
    shard_tables: &RwLock<HashMap<String, ShardTable>>,
    cluster_secret: &str,
    database: &str,
    collection: &str,
    config: &CollectionShardConfig,
    documents: Vec<serde_json::Value>,
) -> Result<(usize, usize), DbError> {
    let table = shard_tables
        .read()
        .unwrap()
        .get(&format!("{}.{}", database, collection))
        .cloned()
        .ok_or_else(|| DbError::InternalError("Shard table not found".to_string()))?;

    let local_id = if let Some(mgr) = &cluster_manager {
        mgr.local_node_id()
    } else {
        "local".to_string()
    };

    // Group documents by shard
    let mut shard_batches: HashMap<u16, Vec<serde_json::Value>> = HashMap::new();

    for mut doc in documents {
        // Ensure _key exists
        let key = if let Some(k) = doc.get("_key").and_then(|v| v.as_str()) {
            k.to_string()
        } else {
            let k = uuid::Uuid::now_v7().to_string();
            if let Some(obj) = doc.as_object_mut() {
                obj.insert("_key".to_string(), serde_json::Value::String(k.clone()));
            }
            k
        };

        // Determine shard
        let shard_key_value = if config.shard_key == "_key" {
            key
        } else {
            doc.get(&config.shard_key)
                .and_then(|v| v.as_str())
                .unwrap_or(&key)
                .to_string()
        };

        let shard_id = ShardRouter::route(&shard_key_value, config.num_shards);
        shard_batches.entry(shard_id).or_default().push(doc);
    }

    let mut total_success = 0usize;
    let mut total_fail = 0usize;
    let client = reqwest::Client::new();

    // Collect futures for parallel processing
    let mut local_batches = Vec::new();
    let mut remote_futures = Vec::new();

    // Separate local and remote batches
    for (shard_id, batch) in shard_batches {
        let physical_coll = format!("{}_s{}", collection, shard_id);

        let assignment = match table.assignments.get(&shard_id) {
            Some(a) => a,
            None => {
                total_fail += batch.len();
                continue;
            }
        };

        let primary_node = &assignment.primary_node;

        if primary_node == &local_id || primary_node == "local" {
            // Queue local batch (with shard_id for replica forwarding)
            local_batches.push((shard_id, physical_coll, batch));
        } else {
            // Queue remote batch as future
            if let Some(mgr) = &cluster_manager {
                if let Some(addr) = mgr.get_node_api_address(primary_node) {
                    let url = format!(
                        "http://{}/_api/database/{}/document/{}/_batch",
                        addr, database, physical_coll
                    );
                    tracing::info!(
                        "INSERT BATCH: Queuing {} docs for remote shard {} at {}",
                        batch.len(),
                        physical_coll,
                        addr
                    );

                    let batch_size = batch.len();
                    let client = client.clone();
                    let secret = cluster_secret.to_string();

                    let future = async move {
                        let res = client
                            .post(&url)
                            .header("X-Shard-Direct", "true")
                            .header("X-Cluster-Secret", &secret)
                            .json(&batch)
                            .send()
                            .await;

                        match res {
                            Ok(r) if r.status().is_success() => (batch_size, 0usize),
                            Ok(r) => {
                                tracing::error!("Remote batch insert failed: {}", r.status());
                                (0, batch_size)
                            }
                            Err(e) => {
                                tracing::error!("Remote batch insert request failed: {}", e);
                                (0, batch_size)
                            }
                        }
                    };
                    remote_futures.push(future);
                } else {
                    total_fail += batch.len();
                }
            } else {
                total_fail += batch.len();
            }
        }
    }

    // Process local batches and forward to replicas
    let mut replica_futures = Vec::new();

    for (shard_id, physical_coll, batch) in local_batches {
        let db = storage.get_database(database)?;
        let coll = db.get_collection(&physical_coll)?;

        // Convert to keyed docs for upsert (prevents duplicates)
        let keyed_docs: Vec<(String, serde_json::Value)> = batch
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

        match coll.upsert_batch(keyed_docs) {
            Ok(count) => {
                total_success += count;

                // Forward to replica nodes for fault tolerance
                if let Some(assignment) = table.assignments.get(&shard_id) {
                    if !assignment.replica_nodes.is_empty() {
                        if let Some(mgr) = &cluster_manager {
                            for replica_node in &assignment.replica_nodes {
                                if let Some(addr) = mgr.get_node_api_address(replica_node) {
                                    let url = format!(
                                        "http://{}/_api/database/{}/document/{}/_replica",
                                        addr, database, physical_coll
                                    );
                                    tracing::debug!(
                                        "REPLICA: Forwarding {} docs to replica {} at {}",
                                        batch.len(),
                                        physical_coll,
                                        addr
                                    );

                                    let client = client.clone();
                                    let secret = cluster_secret.to_string();
                                    let batch = batch.clone();

                                    let future = async move {
                                        let _ = client
                                            .post(&url)
                                            .header("X-Shard-Direct", "true")
                                            .header("X-Cluster-Secret", &secret)
                                            .json(&batch)
                                            .send()
                                            .await;
                                    };
                                    replica_futures.push(future);
                                }
                            }
                        }
                    }
                }
            }
            Err(_) => {
                total_fail += batch.len();
            }
        }
    }

    // Process remote batches in PARALLEL
    if !remote_futures.is_empty() {
        let results = futures::future::join_all(remote_futures).await;
        for (success, fail) in results {
            total_success += success;
            total_fail += fail;
        }
    }

    // Process replica forwarding in PARALLEL (fire-and-forget, don't wait)
    if !replica_futures.is_empty() {
        futures::future::join_all(replica_futures).await;
    }

    Ok((total_success, total_fail))
}

/// Upsert a batch of documents to shards (insert-or-update to prevent duplicates)
/// Used during resharding to avoid creating duplicate documents
/// Returns a list of keys that were SUCCESSFULLY upserted
pub async fn upsert_batch_to_shards(
    storage: &Arc<StorageEngine>,
    cluster_manager: &Option<Arc<ClusterManager>>,
    shard_tables: &RwLock<HashMap<String, ShardTable>>,
    recently_failed_nodes: &RwLock<HashMap<String, std::time::Instant>>,
    cluster_secret: &str,
    database: &str,
    collection: &str,
    config: &CollectionShardConfig,
    documents: Vec<(String, serde_json::Value)>, // (key, doc) pairs
) -> Result<Vec<String>, DbError> {
    let table = shard_tables
        .read()
        .unwrap()
        .get(&format!("{}.{}", database, collection))
        .cloned()
        .ok_or_else(|| DbError::InternalError("Shard table not found".to_string()))?;

    let local_id = if let Some(mgr) = &cluster_manager {
        mgr.local_node_id()
    } else {
        "local".to_string()
    };

    // Group documents by target shard
    let mut shard_batches: HashMap<u16, Vec<(String, serde_json::Value)>> = HashMap::new();

    for (key, doc) in documents {
        let shard_key_value = if config.shard_key == "_key" {
            key.clone()
        } else {
            doc.get(&config.shard_key)
                .and_then(|v| v.as_str())
                .unwrap_or(&key)
                .to_string()
        };

        let shard_id = ShardRouter::route(&shard_key_value, config.num_shards);
        shard_batches.entry(shard_id).or_default().push((key, doc));
    }

    let mut successful_keys: Vec<String> = Vec::new();

    // Process each shard batch
    let mut shard_count = 0;
    for (shard_id, batch) in shard_batches {
        if shard_count > 0 {
            // Small delay to prevent all shards from being processed simultaneously
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }
        shard_count += 1;
        let physical_coll = format!("{}_s{}", collection, shard_id);
        let batch_keys: Vec<String> = batch.iter().map(|(k, _)| k.clone()).collect();
        let batch_len = batch_keys.len();

        let assignment = match table.assignments.get(&shard_id) {
            Some(a) => a,
            None => {
                tracing::error!("UPSERT: No assignment found for shard {}", shard_id);
                continue;
            }
        };

        let primary_node = &assignment.primary_node;

        if primary_node == &local_id || primary_node == "local" {
            // Local upsert - use Collection::upsert_batch to prevent duplicates
            let db = storage.get_database(database)?;
            let coll = match db.get_collection(&physical_coll) {
                Ok(c) => c,
                Err(_) => {
                    // Create shard if missing
                    db.create_collection(physical_coll.clone(), None)?;
                    db.get_collection(&physical_coll)?
                }
            };

            match coll.upsert_batch(batch) {
                Ok(_) => {
                    // All docs in batch succeeded (local atomic batch)
                    successful_keys.extend(batch_keys);
                }
                Err(e) => {
                    tracing::error!("UPSERT: Local upsert failed for {}: {}", physical_coll, e);
                }
            }
        } else {
            // Remote upsert - forward via HTTP batch endpoint
            if let Some(mgr) = &cluster_manager {
                if let Some(addr) = mgr.get_node_api_address(primary_node) {
                    // Circuit breaker: skip nodes that recently failed
                    let recently_failed = recently_failed_nodes.read().unwrap();
                    if recently_failed.contains_key(primary_node) {
                        tracing::warn!(
                            "UPSERT: Skipping batch to recently failed node {} (circuit breaker)",
                            primary_node
                        );
                        continue;
                    }
                    drop(recently_failed);

                    let url = format!(
                        "http://{}/_api/database/{}/document/{}/_batch",
                        addr, database, physical_coll
                    );
                    let client = reqwest::Client::new();

                    // Extract just values for the remote call
                    let values: Vec<serde_json::Value> =
                        batch.into_iter().map(|(_, v)| v).collect();

                    // Retry logic with exponential backoff for remote batch operations
                    let mut retry_count = 0;
                    const MAX_RETRIES: u32 = 3;
                    let mut last_error = None;

                    loop {
                        let timeout_duration = if retry_count == 0 {
                            std::time::Duration::from_secs(30)
                        } else {
                            // Exponential backoff: 30s, 60s, 120s
                            std::time::Duration::from_secs(30 * (1 << retry_count))
                        };

                        match tokio::time::timeout(
                            timeout_duration,
                            client
                                .post(&url)
                                .header("X-Shard-Direct", "true")
                                .header("X-Migration", "true") // Prevent replica forwarding during resharding
                                .header("X-Cluster-Secret", cluster_secret)
                                .json(&values)
                                .send(),
                        )
                        .await
                        {
                            Ok(Ok(res)) => {
                                if res.status().is_success() {
                                    successful_keys.extend(batch_keys);
                                    break;
                                } else {
                                    let status = res.status();
                                    let err_msg = format!("HTTP {}", status);
                                    tracing::warn!("UPSERT: Remote batch request to {} failed: {} (attempt {}/{})",
                                        addr, err_msg, retry_count + 1, MAX_RETRIES + 1);
                                    last_error = Some(err_msg);

                                    if status.as_u16() >= 500 {
                                        if retry_count < MAX_RETRIES {
                                            retry_count += 1;
                                            tokio::time::sleep(std::time::Duration::from_millis(
                                                1000 * (1 << retry_count),
                                            ))
                                            .await;
                                            continue;
                                        }
                                    }
                                    break;
                                }
                            }
                            Ok(Err(e)) => {
                                tracing::warn!(
                                    "UPSERT: Remote batch request to {} failed: {} (attempt {}/{})",
                                    addr,
                                    e,
                                    retry_count + 1,
                                    MAX_RETRIES + 1
                                );
                                last_error = Some(e.to_string());

                                if retry_count < MAX_RETRIES {
                                    retry_count += 1;
                                    tokio::time::sleep(std::time::Duration::from_millis(
                                        1000 * (1 << retry_count),
                                    ))
                                    .await;
                                    continue;
                                }
                                break;
                            }
                            Err(_) => {
                                tracing::warn!("UPSERT: Remote batch request to {} timed out after {:?} (attempt {}/{})",
                                    addr, timeout_duration, retry_count + 1, MAX_RETRIES + 1);
                                last_error = Some(format!("timeout after {:?}", timeout_duration));

                                if retry_count < MAX_RETRIES {
                                    retry_count += 1;
                                    tokio::time::sleep(std::time::Duration::from_millis(
                                        1000 * (1 << retry_count),
                                    ))
                                    .await;
                                    continue;
                                }
                                break;
                            }
                        }
                    }

                    if successful_keys.len() < batch_len && batch_len > 0 {
                        // All retries failed, log final error and record node failure
                        tracing::error!("UPSERT: Remote batch request to {} failed after {} attempts. Last error: {}. Recording node as failed.",
                            addr, MAX_RETRIES + 1, last_error.unwrap_or("unknown".to_string()));
                        let mut failed = recently_failed_nodes.write().unwrap();
                        failed.insert(primary_node.clone(), std::time::Instant::now());
                    }
                } else {
                    tracing::error!("UPSERT: Primary node address unknown for {}", primary_node);
                }
            }
        }
    }

    Ok(successful_keys)
}
