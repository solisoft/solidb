//! Data source operations for SDBQL executor.
//!
//! This module contains data retrieval logic:
//! - get_for_source_docs: Get documents for FOR clause source
//! - scatter_gather_docs: Scatter-gather for sharded collections
//! - get_collection: Collection lookup with database context

use serde_json::Value;

use super::types::Context;
use super::QueryExecutor;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::ForClause;
use crate::storage::Collection;

impl<'a> QueryExecutor<'a> {
    pub(super) fn get_collection(&self, name: &str) -> DbResult<crate::storage::Collection> {
        // If we have a database context, get collection through the database
        // This ensures we use the same cached Collection instances as the handlers
        if let Some(ref db_name) = self.database {
            let database = self.storage.get_database(db_name)?;
            database.get_collection(name)
        } else {
            // No database context - fall back to legacy storage method
            self.storage.get_collection(name)
        }
    }

    /// Try to optimize columnar aggregation queries
    /// Pattern: FOR x IN columnar_collection COLLECT AGGREGATE sum = SUM(x.field) RETURN ...
    pub(super) fn get_for_source_docs(
        &self,
        for_clause: &ForClause,
        ctx: &Context,
        limit: Option<usize>,
    ) -> DbResult<Vec<Value>> {
        // Check if source is an expression (e.g., range 1..5)
        if let Some(expr) = &for_clause.source_expression {
            let value = self.evaluate_expr_with_context(expr, ctx)?;
            return match value {
                Value::Array(arr) => {
                    if let Some(n) = limit {
                        Ok(arr.into_iter().take(n).collect())
                    } else {
                        Ok(arr)
                    }
                }
                other => Ok(vec![other]),
            };
        }

        let source_name = for_clause
            .source_variable
            .as_ref()
            .unwrap_or(&for_clause.collection);

        // Check if source is a LET variable in current context
        if let Some(value) = ctx.get(source_name) {
            return match value {
                Value::Array(arr) => {
                    if let Some(n) = limit {
                        Ok(arr.iter().take(n).cloned().collect())
                    } else {
                        Ok(arr.clone())
                    }
                }
                other => Ok(vec![other.clone()]),
            };
        }

        // Otherwise it's a collection - use scan with limit for optimization
        let collection = self.get_collection(&for_clause.collection)?;

        // Use scatter-gather for sharded collections to get data from all nodes
        if let Some(shard_config) = collection.get_shard_config() {
            if shard_config.num_shards > 0 {
                if let Some(ref coordinator) = self.shard_coordinator {
                    tracing::debug!(
                        "[SDBQL] Using scatter-gather for sharded collection {} ({} shards)",
                        for_clause.collection,
                        shard_config.num_shards
                    );
                    return self.scatter_gather_docs(&for_clause.collection, coordinator, limit);
                }
            }
        }

        // Local scan - for non-sharded collections or when no coordinator
        Ok(collection
            .scan(limit)
            .into_iter()
            .map(|d| d.to_value())
            .collect())
    }
    pub(super) fn scatter_gather_docs(
        &self,
        collection_name: &str,
        coordinator: &crate::sharding::ShardCoordinator,
        limit: Option<usize>,
    ) -> DbResult<Vec<Value>> {
        let db_name = self.database.as_ref().ok_or_else(|| {
            DbError::ExecutionError("No database context for scatter-gather".to_string())
        })?;

        // Get shard table to know which node owns each shard
        let Some(table) = coordinator.get_shard_table(db_name, collection_name) else {
            tracing::debug!(
                "[SCATTER-GATHER] No shard table found for {}, falling back to local scan",
                collection_name
            );
            let collection = self.get_collection(collection_name)?;
            return Ok(collection
                .scan(limit)
                .into_iter()
                .map(|d| d.to_value())
                .collect());
        };

        let my_node_id = coordinator.my_node_id();
        let mut all_docs: Vec<Value> = Vec::new();
        let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Build client for remote queries
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        let cluster_secret = coordinator.cluster_secret();

        // Query each shard's primary node
        for shard_id in 0..table.num_shards {
            let physical_coll = format!("{}_s{}", collection_name, shard_id);

            if let Some(assignment) = table.assignments.get(&shard_id) {
                // Check if we have this shard locally (either as primary or replica)
                let is_primary =
                    assignment.primary_node == my_node_id || assignment.primary_node == "local";
                let is_replica = assignment.replica_nodes.contains(&my_node_id);

                if is_primary || is_replica {
                    // This shard is local - scan it directly
                    if let Ok(coll) = self
                        .storage
                        .get_database(db_name)
                        .and_then(|db| db.get_collection(&physical_coll))
                    {
                        for doc in coll.scan(limit) {
                            let value = doc.to_value();
                            if let Some(key) = value.get("_key").and_then(|k| k.as_str()) {
                                if seen_keys.insert(key.to_string()) {
                                    all_docs.push(value);
                                }
                            }
                        }
                    }
                } else {
                    // This shard is remote - try primary first, then replicas
                    let mut nodes_to_try = vec![assignment.primary_node.clone()];
                    nodes_to_try.extend(assignment.replica_nodes.clone());

                    let mut found = false;
                    for node_id in &nodes_to_try {
                        if let Some(addr) = coordinator.get_node_api_address(node_id) {
                            // Query physical shard collection directly via SDBQL
                            let scheme = std::env::var("SOLIDB_CLUSTER_SCHEME")
                                .unwrap_or_else(|_| "http".to_string());
                            let url =
                                format!("{}://{}/_api/database/{}/cursor", scheme, addr, db_name);
                            let query = if let Some(n) = limit {
                                format!("FOR doc IN `{}` LIMIT {} RETURN doc", physical_coll, n)
                            } else {
                                format!("FOR doc IN `{}` RETURN doc", physical_coll)
                            };

                            let response = client
                                .post(&url)
                                .header("X-Scatter-Gather", "true")
                                .header("X-Cluster-Secret", &cluster_secret)
                                .json(&serde_json::json!({ "query": query }))
                                .send();

                            match response {
                                Ok(resp) => {
                                    if let Ok(body) = resp.json::<serde_json::Value>() {
                                        if let Some(results) =
                                            body.get("result").and_then(|r| r.as_array())
                                        {
                                            for doc in results {
                                                if let Some(key) =
                                                    doc.get("_key").and_then(|k| k.as_str())
                                                {
                                                    if seen_keys.insert(key.to_string()) {
                                                        all_docs.push(doc.clone());
                                                    }
                                                }
                                            }
                                            found = true;
                                            break; // Got data, no need to try other nodes
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("[SCATTER-GATHER] Failed to query shard {} from {}: {}, trying next", 
                                        shard_id, node_id, e);
                                }
                            }
                        }
                    }

                    if !found {
                        tracing::error!("[SCATTER-GATHER] CRITICAL: Could not get data for shard {} from any node. Data may be missing!", shard_id);
                    }
                }
            }
        }

        // Apply final limit
        if let Some(n) = limit {
            if all_docs.len() > n {
                all_docs.truncate(n);
            }
        }

        tracing::info!(
            "[SCATTER-GATHER] Collection {}: gathered {} unique docs from {} shards",
            collection_name,
            all_docs.len(),
            table.num_shards
        );

        Ok(all_docs)
    }
}
