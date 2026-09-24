//! Materialized view operations for SDBQL executor.
//!
//! This module contains the implementation of materialized view operations:
//! - CREATE MATERIALIZED VIEW
//! - REFRESH MATERIALIZED VIEW

use serde_json::Value;

use super::types::{MutationStats, QueryExecutionResult};
use super::QueryExecutor;
use crate::error::{DbError, DbResult};
use crate::sdbql::ast::{CreateMaterializedViewClause, Query, RefreshMaterializedViewClause};

impl<'a> QueryExecutor<'a> {
    /// Validate a view name that came from query text.
    ///
    /// A view name is a collection name, and collections are column families
    /// named `"{database}:{collection}"`. Both MV clauses used to pass a name
    /// containing `:` straight through to `StorageEngine::create_collection` /
    /// `get_collection`, which open any column family on the instance by
    /// literal name — the same primitive SEC-178 removed from `DOCUMENT()`.
    /// The lexer accepts any character inside a backtick-quoted identifier, so
    /// ``CREATE MATERIALIZED VIEW `victim:secrets` `` created (and, via a
    /// planted `_views` row, truncated and overwrote) a collection in another
    /// tenant's database.
    ///
    /// Qualified names are refused outright: nothing needs them, the executor
    /// holds no `Claims` to authorize a second database against, and refusing
    /// them keeps the `_views` key space unambiguous. `CollectionNotFound` on
    /// the name as given, so the error cannot be used to probe which databases
    /// exist.
    fn view_target_name<'n>(&self, view_name: &'n str) -> DbResult<&'n str> {
        if view_name.contains(':') {
            return Err(DbError::CollectionNotFound(view_name.to_string()));
        }
        // A view is a writable collection: it must not shadow a credential,
        // authorization-state, or server-managed collection.
        crate::storage::check_write_access(view_name, self.write_actor())?;
        Ok(view_name)
    }

    /// The database this executor is bound to. Materialized views are always
    /// created in it; there is no cross-database form.
    fn view_database(&self) -> &str {
        self.database.as_deref().unwrap_or("_system")
    }

    /// Refuse a view definition that writes.
    ///
    /// Audit H4: a view query is re-run by the scheduled refresh worker long
    /// after its creator's request is gone, so an `INSERT ... INTO _jobs`
    /// inside one was a standing write on every interval. A materialized view
    /// is a read by definition; nothing documents a mutating one.
    fn reject_mutating_view_query(view_name: &str, query: &Query) -> DbResult<()> {
        if query.has_mutations() {
            return Err(DbError::BadRequest(format!(
                "Materialized view '{}': the view query must be read-only \
                 (no INSERT/UPDATE/UPSERT/REMOVE or catalog functions)",
                view_name
            )));
        }
        Ok(())
    }

    /// The creator recorded in the `_views` row, so the scheduled refresh can
    /// run under the same principal rather than as the server (audit H4).
    /// `null` when the executor has no principal (server-side code); the
    /// refresh worker treats that as a read-only principal.
    fn view_owner(&self) -> Value {
        match &self.principal {
            Some(p) => serde_json::json!({ "user": p.user, "roles": p.roles }),
            None => Value::Null,
        }
    }

    /// Execute CREATE MATERIALIZED VIEW
    pub(super) fn execute_create_materialized_view(
        &self,
        clause: &CreateMaterializedViewClause,
    ) -> DbResult<QueryExecutionResult> {
        let view_name = self.view_target_name(&clause.name)?;
        Self::reject_mutating_view_query(view_name, &clause.query)?;
        let db_name = self.view_database();

        // The view collection always lives in this executor's own database.
        let full_view_name = format!("{}:{}", db_name, view_name);

        // 1. An existing view (or collection) short-circuits before any work.
        if self.storage.get_collection(&full_view_name).is_ok() {
            if clause.if_not_exists {
                return Ok(QueryExecutionResult {
                    results: vec![],
                    mutations: MutationStats::default(),
                });
            }
            return Err(DbError::CollectionAlreadyExists(view_name.to_string()));
        }

        // 2. Run the inner query first: a definition that fails for its
        // creator must leave nothing behind for the refresh worker to re-run
        // later (audit H4 — the `_views` row used to be saved first).
        let results = self.execute_with_stats(&clause.query)?.results;

        // 3. Create the target collection for the view
        match self.storage.create_collection(full_view_name.clone(), None) {
            Ok(_) => {}
            Err(DbError::CollectionAlreadyExists(_)) if clause.if_not_exists => {
                // Lost a race with a concurrent CREATE.
                return Ok(QueryExecutionResult {
                    results: vec![],
                    mutations: MutationStats::default(),
                });
            }
            Err(e) => return Err(e),
        }

        // 4. Serialize the query for storage
        let query_json = serde_json::to_value(&clause.query).map_err(|e| {
            DbError::InternalError(format!("Failed to serialize view query: {}", e))
        })?;

        // 5. Store metadata in the per-database _views system collection
        let views_coll_name = format!("{}:_views", db_name);
        if self.storage.get_collection(&views_coll_name).is_err() {
            let _ = self
                .storage
                .create_collection(views_coll_name.clone(), None);
        }
        let views_coll = self.storage.get_collection(&views_coll_name)?;

        // Keyed by the simple name: `_views` is per database.
        let metadata = serde_json::json!({
            "_key": view_name,
            "type": "materialized",
            "query": query_json,
            // Optional auto-refresh cadence read by the background MV worker.
            "refresh_schedule": clause.refresh_schedule,
            // Who the background refresh runs as.
            "owner": self.view_owner(),
            "created_at": chrono::Utc::now().to_rfc3339()
        });
        views_coll.upsert_batch(vec![(view_name.to_string(), metadata)])?;

        // 6. Bulk insert results into the view collection
        let target_coll = self.storage.get_collection(&full_view_name)?;
        let inserted_count = results.len();
        if !results.is_empty() {
            target_coll.insert_batch(results)?;
        }
        crate::storage::query_cache::invalidate_collection(db_name, view_name);

        Ok(QueryExecutionResult {
            results: vec![Value::String(format!(
                "Materialized view '{}' created",
                view_name
            ))],
            mutations: MutationStats {
                documents_inserted: inserted_count,
                documents_updated: 0,
                documents_removed: 0,
            },
        })
    }

    /// Execute REFRESH MATERIALIZED VIEW
    pub(super) fn execute_refresh_materialized_view(
        &self,
        clause: &RefreshMaterializedViewClause,
    ) -> DbResult<QueryExecutionResult> {
        let view_name = self.view_target_name(&clause.name)?;
        let db_name = self.view_database();

        let views_coll_name = format!("{}:_views", db_name);

        // 1. Get metadata from _views
        let views_coll = self.storage.get_collection(&views_coll_name).map_err(|_| {
            DbError::CollectionNotFound(format!(
                "System collection _views not found. View '{}' probably doesn't exist.",
                view_name
            ))
        })?;

        // Simple name as key
        let metadata = views_coll.get(view_name).map_err(|_| {
            DbError::DocumentNotFound(format!(
                "Materialized view definition for '{}' not found",
                view_name
            ))
        })?;

        // 2. Deserialize query
        // metadata is Document. to_value()? Or access fields directly.
        // Document has Get.
        let query_val = metadata.get("query").ok_or_else(|| {
            DbError::InternalError("Corrupted view metadata: missing query field".to_string())
        })?;

        // Deserialize Value -> Query
        // Need to clone query_val because from_value consumes?
        // serde_json::from_value takes Value.
        let inner_query: Query = serde_json::from_value(query_val.clone()).map_err(|e| {
            DbError::InternalError(format!("Failed to deserialize view query: {}", e))
        })?;

        // A definition stored before mutating view queries were refused must
        // not run either: it would write as whoever triggers the refresh.
        Self::reject_mutating_view_query(view_name, &inner_query)?;

        // 3. Execute the query
        let execution_result = self.execute_with_stats(&inner_query)?;
        let results = execution_result.results;

        // 4. Truncate target collection. `view_target_name` has already
        // rejected a qualified name, so this can only address this
        // executor's own database.
        let full_view_name = format!("{}:{}", db_name, view_name);

        let target_coll = self.storage.get_collection(&full_view_name)?;
        let removed_count = target_coll.truncate()?;

        // Replicas only see this refresh through the replication log: log
        // the truncate (or they keep the stale view contents) and the
        // re-inserted rows.
        let (log_db, log_coll) = (db_name, view_name);
        if let Some(repl) = self.replication {
            repl.log_truncate(log_db, log_coll);
        }

        // 5. Bulk insert new results
        let inserted_count = results.len();
        if !results.is_empty() {
            let inserted = target_coll.insert_batch(results)?;
            if let Some(repl) = self.replication {
                let entries = inserted
                    .iter()
                    .map(|doc| {
                        crate::sync::log::LogEntry::new_op(
                            log_db,
                            log_coll,
                            crate::sync::protocol::Operation::Insert,
                            doc.key.clone(),
                            serde_json::to_vec(&doc.to_value()).ok(),
                        )
                    })
                    .collect();
                repl.append_batch(entries);
            }
        }

        crate::storage::query_cache::invalidate_collection(db_name, view_name);

        Ok(QueryExecutionResult {
            results: vec![Value::String(format!(
                "Materialized view '{}' refreshed",
                view_name
            ))],
            mutations: MutationStats {
                documents_inserted: inserted_count,
                documents_updated: 0,
                documents_removed: removed_count,
            },
        })
    }
}
