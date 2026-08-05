//! Data source operations for SDBQL executor.
//!
//! This module contains data retrieval logic:
//! - get_for_source_docs: Get documents for FOR clause source
//! - scatter_gather_docs: Scatter-gather for sharded collections
//! - get_collection: Collection lookup with database context

use serde_json::Value;
use std::collections::HashSet;

use super::types::Context;
use super::QueryExecutor;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::ForClause;
use crate::storage::http_client::get_blocking_http_client;

impl<'a> QueryExecutor<'a> {
    pub(super) fn get_collection(&self, name: &str) -> DbResult<crate::storage::Collection> {
        // Credential collections (`_env`, `_admins`, `_api_keys`) are ordinary
        // column families, so without this a `FOR d IN _env RETURN d` hands
        // provider API keys to anyone with Read on the database. Server-side
        // readers of these collections use the storage API directly and do not
        // come through here. See tasks/review/SEC-176.
        if crate::storage::is_protected_collection(name) {
            return Err(crate::storage::protected_collection_error(name));
        }

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

    /// Resolve a fully-qualified `{database}:{collection}` name supplied by a
    /// query (only `DOCUMENT()` accepts this form).
    ///
    /// SEC-178: the qualified form resolves **only inside the executor's own
    /// database**. Collections are column families named `"{db}:{collection}"`,
    /// so handing a caller-supplied qualified name to the storage engine opened
    /// any column family on the instance by name — a read-only key scoped to one
    /// database could read every other tenant's documents, plus
    /// `_system:_admins` password hashes, via
    /// `DOCUMENT("victim:secrets/k1")`. Per-database authorization is enforced
    /// once, against the `{db}` path parameter, and `DOCUMENT()` never touches
    /// that parameter.
    ///
    /// This is fixed by removing the cross-database capability rather than by
    /// permission-checking it: the executor holds no `Claims` to check a second
    /// database against, and nothing needs the capability.
    ///
    /// A foreign database is reported as `CollectionNotFound` on the name as
    /// given — deliberately the same answer as a genuinely absent collection, so
    /// the error cannot be used to probe which databases exist.
    pub(super) fn qualified_collection(&self, name: &str) -> DbResult<crate::storage::Collection> {
        let Some((database, collection)) = name.split_once(':') else {
            // Callers only reach here when the name contains ':'.
            return Err(DbError::CollectionNotFound(name.to_string()));
        };

        // An executor with no database context has nothing to authorize
        // against, so the qualified form is unusable there too.
        if self.database.as_deref() != Some(database) {
            return Err(DbError::CollectionNotFound(name.to_string()));
        }

        // Resolve the bare name through the ordinary context path, which
        // applies the credential-collection guard and reuses the handlers'
        // cached `Collection` instances.
        self.get_collection(collection)
    }

    /// Read rows from a columnar collection as a FOR source.
    ///
    /// Returns `Ok(None)` when `name` is not a columnar collection, so the
    /// caller falls through to the ordinary document path.
    ///
    /// Every column is materialised. A narrower read is possible — the storage
    /// layer supports column pruning via `read_columns` and index-aware chunk
    /// skipping via `scan_filtered` — but the projection and filter live in
    /// clauses this function cannot see. Pushing them down is the obvious next
    /// step and is what turns this from "columnar is queryable" into "columnar
    /// is fast to query".
    pub(crate) fn columnar_source_rows(
        &self,
        name: &str,
        limit: Option<usize>,
    ) -> DbResult<Option<Vec<Value>>> {
        // This runs before the document path's guard, so keep the invariant
        // uniform: a credential name is never served from a query, whatever
        // storage layout happens to sit behind it. (A columnar collection
        // named `_env` is a different column family from the real `_env`, so
        // this shadows rather than leaks — but the shadow is confusing.)
        if crate::storage::is_protected_collection(name) {
            return Err(crate::storage::protected_collection_error(name));
        }
        let Some(ref db_name) = self.database else {
            return Ok(None);
        };
        let Ok(database) = self.storage.get_database(db_name) else {
            return Ok(None);
        };
        if !database.is_columnar_collection(name) {
            return Ok(None);
        }

        let columnar =
            crate::storage::ColumnarCollection::load(name.to_string(), db_name, database.db_arc())?;

        let meta = columnar.metadata()?;
        let column_names: Vec<&str> = meta.columns.iter().map(|c| c.name.as_str()).collect();

        let mut rows = columnar.read_columns(&column_names, None)?;
        if let Some(n) = limit {
            rows.truncate(n);
        }
        Ok(Some(rows))
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

        tracing::debug!(
            "get_for_source_docs: source_name='{}', collection='{}'",
            source_name,
            for_clause.collection
        );

        // Check if source is a LET variable in current context
        if let Some(value) = ctx.get(source_name) {
            tracing::debug!("Found source '{}' in context: {:?}", source_name, value);
            return match value {
                Value::Array(arr) => {
                    tracing::debug!("Returning {} items from array", arr.len());
                    if let Some(n) = limit {
                        Ok(arr.iter().take(n).cloned().collect())
                    } else {
                        Ok(arr.clone())
                    }
                }
                other => Ok(vec![other.clone()]),
            };
        } else {
            tracing::debug!(
                "Source '{}' NOT found in context, checking if it's a collection",
                source_name
            );
        }

        // Columnar collections are a separate storage layout, not documents, so
        // the document scan below finds nothing under the `doc:` prefix and
        // `get_collection` reports CollectionNotFound. Before this, a columnar
        // collection was only reachable from SDBQL through one hard-coded
        // shape (`FOR x IN c COLLECT AGGREGATE ...`); adding a FILTER, SORT or
        // LIMIT made the same collection appear not to exist.
        if let Some(rows) = self.columnar_source_rows(&for_clause.collection, limit)? {
            return Ok(rows);
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

        // Local scan - for non-sharded collections or when no coordinator.
        // Use `scan_values` to skip the intermediate `Document` allocation and
        // go straight from the stored bytes to a `serde_json::Value`. This
        // is materially faster on large collections (no extra struct
        // construction or re-merging of metadata).
        Ok(collection.scan_values(limit))
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

        let Some(table) = coordinator.get_shard_table(db_name, collection_name) else {
            tracing::debug!(
                "[SCATTER-GATHER] No shard table found for {}, falling back to local scan",
                collection_name
            );
            let collection = self.get_collection(collection_name)?;
            return Ok(collection.scan_values(limit));
        };

        let my_node_id = coordinator.my_node_id();
        let cluster_secret = coordinator.cluster_secret();
        let scheme = std::env::var("SOLIDB_CLUSTER_SCHEME").unwrap_or_else(|_| "http".to_string());

        // Process local shards first (sequential, but fast)
        let mut local_docs: Vec<(String, Value)> = Vec::new();
        for shard_id in 0..table.num_shards {
            let physical_coll = format!("{}_s{}", collection_name, shard_id);

            if let Some(assignment) = table.assignments.get(&shard_id) {
                let is_primary =
                    assignment.primary_node == my_node_id || assignment.primary_node == "local";
                let is_replica = assignment.replica_nodes.contains(&my_node_id);

                if is_primary || is_replica {
                    if let Ok(coll) = self
                        .storage
                        .get_database(db_name)
                        .and_then(|db| db.get_collection(&physical_coll))
                    {
                        for value in coll.scan_values(limit) {
                            if let Some(key) = value.get("_key").and_then(|k| k.as_str()) {
                                local_docs.push((key.to_string(), value));
                            }
                        }
                    }
                }
            }
        }

        // Prepare remote shard queries for parallel execution
        let remote_queries: Vec<_> = {
            let mut queries = Vec::new();
            for shard_id in 0..table.num_shards {
                let physical_coll = format!("{}_s{}", collection_name, shard_id);

                if let Some(assignment) = table.assignments.get(&shard_id) {
                    let is_primary =
                        assignment.primary_node == my_node_id || assignment.primary_node == "local";
                    let is_replica = assignment.replica_nodes.contains(&my_node_id);

                    if !is_primary && !is_replica {
                        let mut nodes_to_try = vec![assignment.primary_node.clone()];
                        nodes_to_try.extend(assignment.replica_nodes.clone());

                        let query = if let Some(n) = limit {
                            format!("FOR doc IN `{}` LIMIT {} RETURN doc", physical_coll, n)
                        } else {
                            format!("FOR doc IN `{}` RETURN doc", physical_coll)
                        };

                        queries.push((
                            shard_id,
                            nodes_to_try,
                            query,
                            scheme.clone(),
                            db_name.clone(),
                            cluster_secret.clone(),
                        ));
                    }
                }
            }
            queries
        };

        // Execute remote queries in parallel using rayon
        // Clone client for each parallel task since reqwest::blocking::Client is not Sync
        let remote_results: Vec<Vec<Value>> = if remote_queries.is_empty() {
            Vec::new()
        } else {
            use rayon::prelude::*;
            let client = get_blocking_http_client();
            remote_queries
                .into_par_iter()
                .map(|(shard_id, nodes_to_try, query, scheme, db_name, cluster_secret)| {
                    let client = client.clone();
                    let mut all_values = Vec::new();
                    let mut found = false;

                    for node_id in nodes_to_try {
                        if let Some(addr) = coordinator.get_node_api_address(&node_id) {
                            let url = format!(
                                "{}://{}/_api/database/{}/cursor",
                                scheme, addr, db_name
                            );

                            let response = client
                                .post(&url)
                                .header("X-Scatter-Gather", "true")
                                .header("X-Cluster-Secret", cluster_secret.clone())
                                .json(&serde_json::json!({ "query": query }))
                                .send();

                            match response {
                                Ok(resp) => {
                                    if let Ok(body) = resp.json::<serde_json::Value>() {
                                        if let Some(results) =
                                            body.get("result").and_then(|r| r.as_array())
                                        {
                                            for doc in results {
                                                all_values.push(doc.clone());
                                            }
                                            found = true;
                                            break;
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "[SCATTER-GATHER] Failed to query shard {} from {}: {}",
                                        shard_id,
                                        node_id,
                                        e
                                    );
                                }
                            }
                        }
                    }

                    if !found {
                        tracing::error!(
                            "[SCATTER-GATHER] CRITICAL: Could not get data for shard {} from any node",
                            shard_id
                        );
                    }
                    all_values
                })
                .collect()
        };

        // Combine local and remote results with deduplication
        let mut seen_keys: HashSet<String> = HashSet::new();
        let mut all_docs: Vec<Value> = Vec::new();

        for (key, value) in local_docs {
            if seen_keys.insert(key) {
                all_docs.push(value);
            }
        }

        for remote_batch in remote_results {
            for doc in remote_batch {
                if let Some(key) = doc.get("_key").and_then(|k| k.as_str()) {
                    if seen_keys.insert(key.to_string()) {
                        all_docs.push(doc);
                    }
                }
            }
        }

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
