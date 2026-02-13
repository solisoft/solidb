//! Shard cleanup operations
//!
//! This module handles cleanup of orphaned shard collections when nodes
//leave the cluster or shards are reassigned.

#![allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::result_large_err
)]

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::cluster::manager::ClusterManager;
use crate::sharding::coordinator::ShardTable;
use crate::storage::http_client::get_http_client;
use crate::storage::StorageEngine;
use crate::DbError;

/// Clean up orphaned shard collections on this node
///
/// When a node restarts and its shards have been reassigned to other nodes,
/// this function removes the local physical shard collections that are no longer
/// assigned to this node (neither as primary nor replica).
pub async fn cleanup_orphaned_shards(
    storage: &Arc<StorageEngine>,
    shard_tables: &RwLock<HashMap<String, ShardTable>>,
    my_node_id: &str,
) -> Result<usize, DbError> {
    let mut cleaned_count = 0usize;

    // Iterate all databases
    for db_name in storage.list_databases() {
        let db = match storage.get_database(&db_name) {
            Ok(d) => d,
            Err(_) => continue,
        };

        // Get all collections in this database
        let collections = db.list_collections();

        for coll_name in collections {
            // Check if this looks like a physical shard collection (ends with _s<N>)
            if let Some(base_name) = coll_name.strip_suffix(|c: char| c.is_ascii_digit()) {
                if let Some(base) = base_name.strip_suffix("_s") {
                    // This is a physical shard collection
                    // Extract shard ID
                    let shard_suffix = &coll_name[base.len() + 2..];
                    let shard_id: u16 = match shard_suffix.parse() {
                        Ok(id) => id,
                        Err(_) => continue,
                    };

                    // Check if we should have this shard
                    let key = format!("{}.{}", db_name, base);
                    let tables = shard_tables
                        .read()
                        .map_err(|_| DbError::InternalError("Lock poisoned".to_string()))?;

                    let is_assigned_to_us = if let Some(table) = tables.get(&key) {
                        if let Some(assignment) = table.assignments.get(&shard_id) {
                            assignment.primary_node == my_node_id
                                || assignment.replica_nodes.iter().any(|n| n == my_node_id)
                        } else {
                            false
                        }
                    } else {
                        // No shard table means we can't determine - be safe and keep it
                        true
                    };

                    if !is_assigned_to_us {
                        // This shard is not assigned to us - clean it up
                        tracing::warn!(
                            "CLEANUP: Removing orphaned shard collection {}/{} (not assigned to node {})",
                            db_name, coll_name, my_node_id
                        );

                        if let Err(e) = db.delete_collection(&coll_name) {
                            tracing::error!(
                                "CLEANUP: Failed to delete orphaned shard {}/{}: {}",
                                db_name,
                                coll_name,
                                e
                            );
                        } else {
                            cleaned_count += 1;
                        }
                    }
                }
            }
        }
    }

    if cleaned_count > 0 {
        tracing::info!(
            "CLEANUP: Removed {} orphaned shard collections",
            cleaned_count
        );
    }

    Ok(cleaned_count)
}

/// Broadcast cleanup to all cluster nodes
/// This ensures all nodes remove their orphaned shard collections after contraction
pub async fn broadcast_cleanup_orphaned_shards(
    storage: &Arc<StorageEngine>,
    cluster_manager: &Option<Arc<ClusterManager>>,
    shard_tables: &RwLock<HashMap<String, ShardTable>>,
    my_node_id: &str,
    cluster_secret: &str,
) -> Result<(), DbError> {
    // First clean up locally
    if let Err(e) = cleanup_orphaned_shards(storage, shard_tables, my_node_id).await {
        tracing::error!("CLEANUP: Local cleanup failed: {}", e);
    }

    // Then broadcast to all remote nodes
    let mgr = match cluster_manager {
        Some(m) => m,
        None => return Ok(()), // Single node, local cleanup is enough
    };

    let client = get_http_client();

    // Collect all shard tables to broadcast
    let tables: Vec<ShardTable> = {
        let guard = shard_tables
            .read()
            .map_err(|_| DbError::InternalError("Lock poisoned".to_string()))?;
        guard.values().cloned().collect()
    };

    for member in mgr.state().get_all_members() {
        if member.node.id == my_node_id {
            continue; // Skip self
        }

        let addr = &member.node.api_address;
        let url = format!("http://{}/_api/cluster/cleanup", addr);

        tracing::info!(
            "CLEANUP: Broadcasting cleanup (with {} tables) to node {} at {}",
            tables.len(),
            member.node.id,
            addr
        );

        match client
            .post(&url)
            .header("X-Cluster-Secret", cluster_secret)
            .json(&tables) // Send tables in body
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
        {
            Ok(res) => {
                if !res.status().is_success() {
                    tracing::warn!(
                        "CLEANUP: Cleanup broadcast to {} failed with status {}",
                        addr,
                        res.status()
                    );
                } else {
                    tracing::info!("CLEANUP: Cleanup broadcast to {} successful", addr);
                }
            }
            Err(e) => {
                tracing::warn!("CLEANUP: Cleanup broadcast to {} failed: {}", addr, e);
            }
        }
    }

    Ok(())
}
