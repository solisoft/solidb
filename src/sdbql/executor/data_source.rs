//! Data source operations for SDBQL executor.
//!
//! This module contains data retrieval logic:
//! - get_for_source_docs: Get documents for FOR clause source
//! - scatter_gather_docs: Scatter-gather for sharded collections
//! - get_collection: Collection lookup with database context

use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

use super::types::Context;
use super::{to_bool, QueryExecutor};
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::ForClause;
use crate::storage::http_client::get_blocking_http_client;

/// A collection's row policy compiled for one principal (see
/// [`QueryExecutor::row_policy_gate`]).
pub(super) struct RowPolicyGate {
    /// `None` when the stored predicate does not parse: deny every row.
    expr: Option<crate::sdbql::ast::Expression>,
    binding: String,
    user: String,
}

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

    /// Resolve a collection a query is about to *write* to.
    ///
    /// Adds the write-only tier to [`Self::get_collection`]'s guard: `FOR d IN
    /// _scripts RETURN d` stays a documented read, but `INSERT {...} INTO
    /// _scripts` is how a principal with Write installed Lua that the service
    /// router then executed, and `INSERT ... INTO _views` is how a `REFRESH`
    /// was pointed at another tenant's collection.
    pub(super) fn get_collection_for_write(
        &self,
        name: &str,
    ) -> DbResult<crate::storage::Collection> {
        crate::storage::check_write_access(name, self.write_actor())?;
        self.get_collection(name)
    }

    /// Who this executor writes as: the principal a client handler attached,
    /// or the server itself when none was (queue worker, triggers, tests).
    pub(super) fn write_actor(&self) -> crate::storage::WriteActor {
        match &self.principal {
            Some(p) => crate::storage::WriteActor::client(p.can_admin),
            None => crate::storage::WriteActor::Server,
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

        // Search views (`CREATE_VIEW`) are aliases onto a backing collection.
        let source_coll =
            if let Some(backing) = self.resolve_search_view_collection(&for_clause.collection)? {
                backing
            } else {
                for_clause.collection.clone()
            };

        // Otherwise it's a collection - use scan with limit for optimization
        let collection = self.get_collection(&source_coll)?;

        // Use scatter-gather for sharded collections to get data from all nodes
        if let Some(shard_config) = collection.get_shard_config() {
            if shard_config.num_shards > 0 {
                if let Some(ref coordinator) = self.shard_coordinator {
                    tracing::debug!(
                        "[SDBQL] Using scatter-gather for sharded collection {} ({} shards)",
                        for_clause.collection,
                        shard_config.num_shards
                    );
                    // Audit H2: the gathered rows are filtered like a local
                    // scan. A LIMIT is applied after the policy, not pushed
                    // to the shards, or the policy would under-fill it.
                    if !self.row_policy_applies(&for_clause.collection) {
                        return self.scatter_gather_docs(&source_coll, coordinator, limit);
                    }
                    let docs = self.scatter_gather_docs(&source_coll, coordinator, None)?;
                    let mut docs = self.apply_row_policy(&for_clause.collection, docs, ctx);
                    if let Some(n) = limit {
                        docs.truncate(n);
                    }
                    return Ok(docs);
                }
            }
        }

        let filtered =
            for_clause.valid_time.is_some() || self.row_policy_applies(&for_clause.collection);

        if let Some(ts_expr) = &for_clause.system_time {
            let ts_val = self.evaluate_expr_with_context(ts_expr, ctx)?;
            let micros = super::evaluate::as_of_micros(&ts_val)?;
            let mut docs = collection.scan_as_of(micros)?;
            if !filtered {
                if let Some(n) = limit {
                    docs.truncate(n);
                }
            }
            let docs = self.apply_valid_time(for_clause, docs, ctx)?;
            let mut docs = self.apply_row_policy(&for_clause.collection, docs, ctx);
            if let Some(n) = limit {
                docs.truncate(n);
            }
            return Ok(docs);
        }

        // Local scan - for non-sharded collections or when no coordinator.
        // Use `scan_values` to skip the intermediate `Document` allocation and
        // go straight from the stored bytes to a `serde_json::Value`. This
        // is materially faster on large collections (no extra struct
        // construction or re-merging of metadata).
        //
        // The row ceiling is checked once the FOR has built its rows, which
        // is too late for a scan that materialises the whole collection into
        // a Vec first: `FOR d IN huge COLLECT ...` reached the OOM killer
        // before it reached `check_budget`. Stop the scan one past the ceiling
        // instead — the check that follows fails exactly as it would have on
        // the full scan, just without holding it. Only the plain path can do
        // this: valid-time and row-policy filtering happen after the scan, and
        // a capped scan would under-fill them.
        if !filtered {
            let cap = self.max_intermediate_rows.saturating_add(1);
            return Ok(collection.scan_values(Some(limit.map_or(cap, |n| n.min(cap)))));
        }
        // Filtered: the LIMIT applies to what survives the filters, so it
        // cannot be pushed into the scan.
        let docs = collection.scan_values(None);
        let docs = self.apply_valid_time(for_clause, docs, ctx)?;
        let mut docs = self.apply_row_policy(&for_clause.collection, docs, ctx);
        if let Some(n) = limit {
            docs.truncate(n);
        }
        Ok(docs)
    }

    fn apply_valid_time(
        &self,
        for_clause: &ForClause,
        docs: Vec<Value>,
        ctx: &Context,
    ) -> DbResult<Vec<Value>> {
        let Some(spec) = &for_clause.valid_time else {
            return Ok(docs);
        };
        let overlaps = |doc: &Value, from: i64, to: i64| -> bool {
            let vf = doc
                .get("valid_from")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .unwrap_or(i64::MIN);
            let vt = doc
                .get("valid_to")
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .unwrap_or(i64::MAX);
            vf <= to && vt >= from
        };
        match spec {
            crate::sdbql::ast::ValidTimeSpec::AsOf(e) => {
                let ts = self.evaluate_expr_with_context(e, ctx)?;
                let t = ts
                    .as_i64()
                    .or_else(|| ts.as_f64().map(|f| f as i64))
                    .unwrap_or(0);
                Ok(docs.into_iter().filter(|d| overlaps(d, t, t)).collect())
            }
            crate::sdbql::ast::ValidTimeSpec::Range { from, to } => {
                let a = self.evaluate_expr_with_context(from, ctx)?;
                let b = self.evaluate_expr_with_context(to, ctx)?;
                let fa = a
                    .as_i64()
                    .or_else(|| a.as_f64().map(|f| f as i64))
                    .unwrap_or(0);
                let tb = b
                    .as_i64()
                    .or_else(|| b.as_f64().map(|f| f as i64))
                    .unwrap_or(0);
                Ok(docs.into_iter().filter(|d| overlaps(d, fa, tb)).collect())
            }
        }
    }

    /// The row policy that binds this principal's reads of `collection`, if
    /// any. A search-view alias resolves to its backing collection's policy:
    /// looking the policy up under the view name found nothing and let every
    /// row through.
    ///
    /// Audit H2: every collection read path goes through this — scans filter
    /// with [`Self::apply_row_policy`], point and function reads with
    /// [`Self::row_policy_permits`], and the fast / index paths that cannot
    /// filter check [`Self::row_policy_applies`] and step aside for the scan.
    pub(super) fn row_policy_gate(&self, collection: &str) -> Option<Arc<RowPolicyGate>> {
        let principal = self.principal.as_ref()?;
        if principal.can_admin {
            return None;
        }
        if let Some(cached) = self.caches.row_policy_gates.lock().get(collection) {
            return cached.clone();
        }
        let coll = match self.get_collection(collection) {
            Ok(c) => c,
            Err(_) => {
                let backing = self.resolve_search_view_collection(collection).ok()??;
                self.get_collection(&backing).ok()?
            }
        };
        self.row_policy_gate_for(&coll, collection)
    }

    /// [`Self::row_policy_gate`] for a collection already resolved (e.g. a
    /// qualified `DOCUMENT("db:c/k")` name). `binding` is the name the
    /// predicate may use for the row, alongside `doc`.
    ///
    /// Audit P10: the policy used to be read from RocksDB and re-parsed on
    /// every call — per DOCUMENT id, per search hit. It is now compiled once
    /// per executor and binding name; `ROW_POLICY(c, pred)` drops the entry.
    pub(super) fn row_policy_gate_for(
        &self,
        coll: &crate::storage::Collection,
        binding: &str,
    ) -> Option<Arc<RowPolicyGate>> {
        let principal = self.principal.as_ref()?;
        if principal.can_admin {
            return None;
        }
        if let Some(cached) = self.caches.row_policy_gates.lock().get(binding) {
            return cached.clone();
        }
        let gate = coll.get_row_policy().map(|text| {
            // An unparseable policy denies every row rather than none.
            let expr = crate::sdbql::parser::Parser::new(&text)
                .and_then(|mut p| p.parse_expression())
                .ok();
            Arc::new(RowPolicyGate {
                expr,
                binding: binding.to_string(),
                user: principal.user.clone(),
            })
        });
        self.caches
            .row_policy_gates
            .lock()
            .insert(binding.to_string(), gate.clone());
        gate
    }

    /// Forget the compiled policies (after `ROW_POLICY` set or cleared one
    /// within this query). All of them: view aliases share a policy under
    /// other binding names.
    pub(super) fn invalidate_row_policy_gates(&self) {
        self.caches.row_policy_gates.lock().clear();
    }

    /// Whether `apply_row_policy` would filter this principal's scan of
    /// `collection`.
    pub(super) fn row_policy_applies(&self, collection: &str) -> bool {
        self.row_policy_gate(collection).is_some()
    }

    /// Evaluate a compiled row policy against one document.
    ///
    /// The predicate sees the row (as `doc` and under the collection's
    /// name) and `CURRENT_USER` — not the caller's query variables. It used
    /// to run in a clone of the caller's row context, which cost a full copy
    /// per document and let a query's own `LET`s shadow names the policy
    /// reads. `_ctx` is kept for the call sites.
    pub(super) fn row_policy_allows(
        &self,
        gate: &RowPolicyGate,
        doc: &Value,
        _ctx: &Context,
    ) -> bool {
        let Some(expr) = &gate.expr else {
            return false;
        };
        let mut row = Context::with_capacity(3);
        if gate.binding != "doc" {
            row.insert(gate.binding.clone(), doc.clone());
        }
        row.insert("doc".into(), doc.clone());
        row.insert("CURRENT_USER".into(), Value::String(gate.user.clone()));
        self.evaluate_expr_with_context(expr, &row)
            .map(|v| to_bool(&v))
            .unwrap_or(false)
    }

    /// Whether this principal may see `doc` from `collection`: the check for
    /// point reads (`DOCUMENT`, `DOC_AS_OF`, ...) and search functions.
    pub(super) fn row_policy_permits(&self, collection: &str, doc: &Value, ctx: &Context) -> bool {
        match self.row_policy_gate(collection) {
            Some(gate) => self.row_policy_allows(&gate, doc, ctx),
            None => true,
        }
    }

    /// A whole-collection read (JOIN sides): bounded by the row ceiling and
    /// filtered by the row policy, which the JOIN used to skip (audit H2).
    pub(super) fn scan_bounded_with_policy(
        &self,
        name: &str,
        collection: &crate::storage::Collection,
    ) -> DbResult<Vec<Value>> {
        let docs = self.scan_bounded(collection)?;
        Ok(self.apply_row_policy(name, docs, &Context::new()))
    }

    /// Point read through the row policy: `None` when the document is absent
    /// or hidden from this principal — the two are indistinguishable, so a
    /// hidden key cannot be probed for.
    pub(super) fn get_visible(
        &self,
        coll: &crate::storage::Collection,
        binding: &str,
        key: &str,
        ctx: &Context,
    ) -> Option<Value> {
        let doc = coll.get(key).ok()?.to_value();
        match self.row_policy_gate_for(coll, binding) {
            Some(gate) if !self.row_policy_allows(&gate, &doc, ctx) => None,
            _ => Some(doc),
        }
    }

    pub(super) fn apply_row_policy(
        &self,
        collection: &str,
        docs: Vec<Value>,
        ctx: &Context,
    ) -> Vec<Value> {
        let Some(gate) = self.row_policy_gate(collection) else {
            return docs;
        };
        docs.into_iter()
            .filter(|doc| self.row_policy_allows(&gate, doc, ctx))
            .collect()
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
        let scheme = crate::cluster::http::cluster_scheme().to_string();

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
