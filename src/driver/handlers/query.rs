use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;
use crate::sdbql::QueryExecutor;
use crate::storage::query_cache;
use std::collections::HashMap;

/// Execute an SDBQL query over the native driver protocol.
///
/// This mirrors the HTTP `/cursor` handler's two caching layers, which it
/// previously had neither of:
///
/// * **Prepared-statement cache** — `parse_if_needed` instead of a bare
///   `parse`, so a repeated query is not re-parsed on every request.
/// * **Result cache** — read-only queries are memoized per (database, query,
///   bind vars), and mutating queries invalidate the collections they touch.
///
/// Missing both made the driver measurably slower than HTTP on multi-row reads:
/// 0.84x throughput on a 50-row projection, with SoliDB's CPU per request nearly
/// doubling (127 -> 216us), because the driver was executing the query for real
/// while HTTP replayed a memoized result. That reads as "the binary protocol is
/// slow at queries" and was really "one handler caches and the other does not".
/// `cache` — when false, skip the result-cache lookup and store (HTTP
/// `/cursor` `cache: false`). Prepared-statement caching still applies.
pub fn handle_query(
    handler: &DriverHandler,
    database: String,
    sdbql: String,
    bind_vars: Option<HashMap<String, serde_json::Value>>,
    cache: bool,
) -> Response {
    let bind_vars = bind_vars.unwrap_or_default();

    // Prepared-statement cache: skip the parse for a query we have seen before.
    let prepared = match crate::sdbql::get_prepared_statement_cache().parse_if_needed(&sdbql) {
        Ok(p) => p,
        Err(e) => {
            return Response::error(DriverError::DatabaseError(format!("Parse error: {}", e)))
        }
    };
    let query = prepared.query.as_ref();

    // The dispatch check only required Read for Query; upgrade to Write when the
    // parsed query mutates (same as the HTTP /cursor).
    let mutates = query.has_mutations();
    if mutates {
        if let Err(e) = crate::server::AuthorizationService::check_permission_raw(
            &handler.session_permissions,
            crate::server::PermissionAction::Write,
            Some(&database),
            handler.session_scoped_databases.as_deref(),
        ) {
            return Response::error(DriverError::AuthError(e.to_string()));
        }
    }

    // Result cache, read-only queries only (and only when the client opts in).
    // Keyed on the database too, so two databases running the same query text
    // cannot share entries.
    let cache_key = if mutates || !cache {
        None
    } else {
        Some(query_cache::hash_query(&database, &sdbql, &bind_vars))
    };
    if let Some(ref key) = cache_key {
        if let Some(hit) = query_cache::get_query_cache().get(key) {
            return Response::ok(serde_json::json!(hit.as_ref().clone()));
        }
    }

    // The executor gets the session's principal, same as the HTTP handler: the
    // dispatch check above only demands Read for a non-mutating query, so the
    // principal is what keeps a read-only session off the write-side query
    // paths (auto-index creation) and applies row policies here too.
    let principal = handler.query_principal(&database);
    let mut executor = if bind_vars.is_empty() {
        QueryExecutor::with_database(&handler.storage, database)
    } else {
        QueryExecutor::with_database_and_bind_vars(&handler.storage, database, bind_vars)
    }
    .with_principal(principal);
    // Mutating queries must reach the replication log, same as the HTTP handler.
    if let Some(ref log) = handler.replication {
        executor = executor.with_replication(log);
    }

    match executor.execute(query) {
        Ok(results) => {
            if let Some(key) = cache_key {
                query_cache::get_query_cache().put(key, results.clone());
            }
            if mutates {
                // Same invalidation rule as the HTTP handler: drop the touched
                // collections, or everything when the set cannot be determined.
                let collections = crate::server::handlers::query::mutated_collections(query);
                if collections.is_empty() {
                    query_cache::get_query_cache().invalidate_all();
                } else {
                    for collection in collections {
                        query_cache::get_query_cache().invalidate_collection(collection);
                    }
                }
            }
            Response::ok(serde_json::json!(results))
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}

pub fn handle_explain(
    handler: &DriverHandler,
    database: String,
    sdbql: String,
    bind_vars: Option<HashMap<String, serde_json::Value>>,
) -> Response {
    let bind_vars = bind_vars.unwrap_or_default();
    let prepared = match crate::sdbql::get_prepared_statement_cache().parse_if_needed(&sdbql) {
        Ok(p) => p,
        Err(e) => {
            return Response::error(DriverError::DatabaseError(format!("Parse error: {}", e)))
        }
    };

    // EXPLAIN reports what this caller's run would do, so it needs the same
    // principal that run would get.
    let principal = handler.query_principal(&database);
    let executor = if bind_vars.is_empty() {
        QueryExecutor::with_database(&handler.storage, database)
    } else {
        QueryExecutor::with_database_and_bind_vars(&handler.storage, database, bind_vars)
    }
    .with_principal(principal);

    match executor.explain(prepared.query.as_ref()) {
        Ok(explanation) => Response::ok(serde_json::to_value(explanation).unwrap_or_default()),
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
    }
}
