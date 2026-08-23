use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;
use crate::sdbql::QueryExecutor;
// Shared with the HTTP /cursor handler so both protocols cache, invalidate,
// and decide what needs the blocking pool on exactly the same terms.
use crate::server::handlers::query::{
    invalidate_collections, is_long_running_query, mutated_collections,
};
use crate::storage::query_cache;
use std::collections::HashMap;

/// Query execution timeout over the binary protocol, matching the HTTP
/// handler's limit. Without this, a long-running query sent over the native
/// port runs unbounded — the 30s HTTP cap only protected one of the two
/// protocols.
const QUERY_TIMEOUT_SECS: u64 = 30;

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
pub async fn handle_query(
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

    // Collections this query invalidates, resolved before execution: the
    // timeout path below needs them after the executor is out of reach.
    let invalidated: Vec<String> = if mutates {
        mutated_collections(query)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        Vec::new()
    };

    let storage = handler.storage.clone();
    let replication = handler.replication.clone();

    // Point reads run inline. The HTTP handler gates `spawn_blocking` on the
    // same predicate: a thread handoff costs more than the query itself on a
    // single-document read, and this handler exists to close a CPU-per-request
    // gap, not to widen it. Everything that scans, loops, or mutates takes the
    // blocking pool under a timeout.
    if !is_long_running_query(query) {
        let mut executor = if bind_vars.is_empty() {
            QueryExecutor::with_database(&storage, database)
        } else {
            QueryExecutor::with_database_and_bind_vars(&storage, database, bind_vars)
        }
        .with_principal(principal);
        if let Some(ref log) = replication {
            executor = executor.with_replication(log);
        }
        return match executor.execute(query) {
            Ok(results) => {
                if let Some(key) = cache_key {
                    query_cache::get_query_cache().put(key, results.clone());
                }
                if mutates {
                    invalidate_collections(&invalidated);
                }
                Response::ok(serde_json::json!(results))
            }
            Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
        };
    }

    // Execution runs on the blocking pool under a timeout, mirroring the HTTP
    // /cursor handler: the executor is synchronous CPU work and must not pin
    // the async runtime thread or run without an upper time bound.
    let exec_query = (*query).clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let mut executor = if bind_vars.is_empty() {
            QueryExecutor::with_database(&storage, database)
        } else {
            QueryExecutor::with_database_and_bind_vars(&storage, database, bind_vars)
        }
        .with_principal(principal);
        // Mutating queries must reach the replication log, same as HTTP.
        if let Some(ref log) = replication {
            executor = executor.with_replication(log);
        }
        executor.execute(&exec_query)
    });

    // `&mut task` so the handle survives a timeout and can still be observed.
    match tokio::time::timeout(
        std::time::Duration::from_secs(QUERY_TIMEOUT_SECS),
        &mut task,
    )
    .await
    {
        Ok(join_result) => match join_result {
            Ok(Ok(results)) => {
                if let Some(key) = cache_key {
                    query_cache::get_query_cache().put(key, results.clone());
                }
                if mutates {
                    // Same invalidation rule as the HTTP handler: drop the
                    // touched collections, or everything when the set cannot
                    // be determined.
                    invalidate_collections(&invalidated);
                }
                Response::ok(serde_json::json!(results))
            }
            Ok(Err(e)) => Response::error(DriverError::DatabaseError(e.to_string())),
            Err(e) => Response::error(DriverError::DatabaseError(format!(
                "Task join error: {}",
                e
            ))),
        },
        Err(_) => {
            // A blocking task is not cancellable: dropping the handle does not
            // stop the executor, so a mutating query that overruns the timeout
            // still commits (and still reaches the replication log). Drop the
            // cached rows now, and again once it really finishes, so no reader
            // is served a pre-mutation result while the write lands.
            if mutates {
                invalidate_collections(&invalidated);
                tokio::spawn(async move {
                    let _ = task.await;
                    invalidate_collections(&invalidated);
                });
            }
            Response::error(DriverError::DatabaseError(format!(
                "Query execution timeout: exceeded {} seconds",
                QUERY_TIMEOUT_SECS
            )))
        }
    }
}

pub async fn handle_explain(
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
    let storage = handler.storage.clone();
    let exec_query = (*prepared.query).clone();

    let task = tokio::task::spawn_blocking(move || {
        let executor = if bind_vars.is_empty() {
            QueryExecutor::with_database(&storage, database)
        } else {
            QueryExecutor::with_database_and_bind_vars(&storage, database, bind_vars)
        }
        .with_principal(principal);
        executor.explain(&exec_query)
    });

    match tokio::time::timeout(std::time::Duration::from_secs(QUERY_TIMEOUT_SECS), task).await {
        Ok(join_result) => match join_result {
            Ok(Ok(explanation)) => {
                Response::ok(serde_json::to_value(explanation).unwrap_or_default())
            }
            Ok(Err(e)) => Response::error(DriverError::DatabaseError(e.to_string())),
            Err(e) => Response::error(DriverError::DatabaseError(format!(
                "Task join error: {}",
                e
            ))),
        },
        Err(_) => Response::error(DriverError::DatabaseError(format!(
            "Explain timeout: exceeded {} seconds",
            QUERY_TIMEOUT_SECS
        ))),
    }
}
