use crate::driver::protocol::{DriverError, Response};
use crate::driver::DriverHandler;
use crate::error::DbResult;
use crate::sdbql::executor::SmallRaw;
use crate::sdbql::{Query, QueryExecutor};
// Shared with the HTTP /cursor handler so both protocols cache, invalidate,
// and decide what needs the blocking pool on exactly the same terms.
use crate::server::handlers::query::{
    expression_is_heavy, invalidate_collections, is_long_running_query, is_small_local_scan,
    mutated_collections,
};
use crate::server::query_metrics::{Observation, QueryKind};
use crate::storage::query_cache;
use std::collections::HashMap;

/// Query execution timeout over the binary protocol, matching the HTTP
/// handler's limit. Without this, a long-running query sent over the native
/// port runs unbounded — the 30s HTTP cap only protected one of the two
/// protocols.
const QUERY_TIMEOUT_SECS: u64 = 30;

/// Most rows an index equality may name and still be answered on the async
/// worker (see `handle_query`): each is a point read of a document, so this
/// bounds the time the worker is held to a few dozen of them.
const INLINE_INDEX_ROWS: usize = 64;

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
/// A query's rows: copied from storage already encoded, or evaluated.
enum Rows {
    Raw(Vec<u8>),
    Values(Vec<serde_json::Value>),
}

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

    // The executor gets the session's principal, same as the HTTP handler: the
    // dispatch check above only demands Read for a non-mutating query, so the
    // principal is what keeps a read-only session off the write-side query
    // paths (auto-index creation) and applies row policies here too.
    let principal = handler.query_principal(&database);

    // Result cache, read-only queries only (and only when the client opts in).
    // Keyed on the database and the session's principal (audit H1), and `None`
    // when the collection set cannot be read off the AST (audit P2).
    let cache_generation = query_cache::current_generation();
    let cache_key = if mutates || !cache {
        None
    } else {
        query_cache::cache_key_for(
            &handler.storage,
            &database,
            &sdbql,
            query,
            &bind_vars,
            &principal,
        )
    };
    if let Some(ref key) = cache_key {
        if let Some(hit) = query_cache::get_query_cache().get(key) {
            return Response::ok_shared_rows(hit);
        }
    }

    // Collections this query invalidates, resolved before execution: the
    // timeout path below needs them after the executor is out of reach.
    let invalidated: Vec<String> = if mutates {
        mutated_collections(query).into_iter().collect()
    } else {
        Vec::new()
    };

    let storage = handler.storage.clone();
    let replication = handler.replication.clone();
    let try_raw = cache_key.is_none() && !mutates;

    // An equality read through an index that names a few documents is a
    // handful of point reads: answered here, as a single-document get is,
    // instead of on the blocking pool below, whose round trip (a pool
    // thread woken, then this task woken back) cost more than the reads.
    // The index is read first, and anything larger goes on to the pool
    // with nothing fetched.
    if !mutates && is_long_running_query(query) {
        let executor = if bind_vars.is_empty() {
            QueryExecutor::with_database(&storage, database.clone())
        } else {
            QueryExecutor::with_database_and_bind_vars(
                &storage,
                database.clone(),
                bind_vars.clone(),
            )
        }
        .with_principal(principal.clone())
        .with_timeout(std::time::Duration::from_secs(QUERY_TIMEOUT_SECS));
        let observed = Observation::start(QueryKind::of(mutates));
        if try_raw {
            match executor.execute_raw_msgpack_small(query, INLINE_INDEX_ROWS) {
                Ok(SmallRaw::Rows(rows)) => {
                    observed.finish();
                    return Response::raw_rows(rows);
                }
                Ok(SmallRaw::NotSmall | SmallRaw::Ordinary) => {}
                Err(e) => {
                    observed.finish();
                    return Response::error(DriverError::DatabaseError(e.to_string()));
                }
            }
        }
        // The same for a SORT read in index order whose rows are evaluated
        // (`LET c = DOCUMENT(…) RETURN {…}`), when what each row evaluates
        // is itself cheap.
        if per_row_work_is_light(query) {
            match executor.execute_small_sorted(query, INLINE_INDEX_ROWS) {
                Ok(Some(rows)) => {
                    observed.finish();
                    return respond(
                        Ok(Rows::Values(rows)),
                        cache_key,
                        cache_generation,
                        mutates,
                        &invalidated,
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    observed.finish();
                    return Response::error(DriverError::DatabaseError(e.to_string()));
                }
            }
        }
        observed.cancel();
    }

    // Point reads run inline. The HTTP handler gates `spawn_blocking` on the
    // same predicate: a thread handoff costs more than the query itself on a
    // single-document read, and this handler exists to close a CPU-per-request
    // gap, not to widen it. Everything that scans, loops, or mutates takes the
    // blocking pool under a timeout.
    if !is_long_running_query(query) || is_small_local_scan(query, &storage, &database) {
        let mut executor = if bind_vars.is_empty() {
            QueryExecutor::with_database(&storage, database)
        } else {
            QueryExecutor::with_database_and_bind_vars(&storage, database, bind_vars)
        }
        .with_principal(principal)
        .with_timeout(std::time::Duration::from_secs(QUERY_TIMEOUT_SECS));
        if let Some(ref log) = replication {
            executor = executor.with_replication(log);
        }
        // Rows copied straight from storage when the query allows it (and
        // nothing is to be cached: the cache holds decoded rows).
        let observed = Observation::start(QueryKind::of(mutates));
        let outcome = run(&executor, query, try_raw);
        observed.finish();
        return respond(outcome, cache_key, cache_generation, mutates, &invalidated);
    }

    // Execution runs on the blocking pool under a timeout, mirroring the HTTP
    // /cursor handler: the executor is synchronous CPU work and must not pin
    // the async runtime thread or run without an upper time bound. The tree
    // goes with it shared, not copied.
    let exec_query = prepared.query.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let mut executor = if bind_vars.is_empty() {
            QueryExecutor::with_database(&storage, database)
        } else {
            QueryExecutor::with_database_and_bind_vars(&storage, database, bind_vars)
        }
        .with_principal(principal)
        .with_timeout(std::time::Duration::from_secs(QUERY_TIMEOUT_SECS));
        // Mutating queries must reach the replication log, same as HTTP.
        if let Some(ref log) = replication {
            executor = executor.with_replication(log);
        }
        // Measured here, on the thread that runs it: a panic, or a run past
        // the client's timeout, is still recorded once.
        let observed = Observation::start(QueryKind::of(mutates));
        let outcome = run(&executor, &exec_query, try_raw);
        observed.finish();
        outcome
    });

    // `&mut task` so the handle survives a timeout and can still be observed.
    match tokio::time::timeout(
        std::time::Duration::from_secs(QUERY_TIMEOUT_SECS),
        &mut task,
    )
    .await
    {
        Ok(Ok(outcome)) => respond(outcome, cache_key, cache_generation, mutates, &invalidated),
        Ok(Err(e)) => Response::error(DriverError::DatabaseError(format!(
            "Task join error: {}",
            e
        ))),
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

/// Whether everything `query` evaluates per row — its FILTERs and LETs, the
/// LETs after its LIMIT, its RETURN — is cheap: no subquery, and none of the
/// functions that scan or call out ([`expression_is_heavy`]).
fn per_row_work_is_light(query: &Query) -> bool {
    use crate::sdbql::BodyClause;
    let light = |e: &crate::sdbql::Expression| !expression_is_heavy(e);
    query.body_clauses.iter().skip(1).all(|c| match c {
        BodyClause::Filter(f) => light(&f.expression),
        BodyClause::Let(l) => light(&l.expression),
        _ => false,
    }) && query.post_limit_lets.iter().all(|l| light(&l.expression))
        && query
            .return_clause
            .as_ref()
            .is_some_and(|r| light(&r.expression))
}

/// Run `query`, copying the rows straight from storage when `try_raw` and the
/// planner allows it.
fn run(executor: &QueryExecutor, query: &Query, try_raw: bool) -> DbResult<Rows> {
    if try_raw {
        if let Some(rows) = executor.execute_raw_msgpack(query)? {
            return Ok(Rows::Raw(rows));
        }
    }
    executor.execute(query).map(Rows::Values)
}

/// Turn an execution outcome into the response, caching a read's rows and
/// invalidating what a write touched.
fn respond(
    outcome: DbResult<Rows>,
    cache_key: Option<String>,
    cache_generation: u64,
    mutates: bool,
    invalidated: &[String],
) -> Response {
    match outcome {
        Ok(Rows::Raw(rows)) => Response::raw_rows(rows),
        Ok(Rows::Values(results)) => {
            if let Some(key) = cache_key {
                if results.len() <= query_cache::MAX_CACHED_ROWS {
                    query_cache::get_query_cache().put_if_current(
                        key,
                        results.clone(),
                        cache_generation,
                    );
                }
            }
            if mutates {
                // Same invalidation rule as the HTTP handler: drop the touched
                // collections, or everything when the set cannot be determined.
                invalidate_collections(invalidated);
            }
            Response::ok(serde_json::Value::Array(results))
        }
        Err(e) => Response::error(DriverError::DatabaseError(e.to_string())),
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
    let exec_query = prepared.query.clone();

    let task = tokio::task::spawn_blocking(move || {
        let executor = if bind_vars.is_empty() {
            QueryExecutor::with_database(&storage, database)
        } else {
            QueryExecutor::with_database_and_bind_vars(&storage, database, bind_vars)
        }
        .with_principal(principal)
        .with_timeout(std::time::Duration::from_secs(QUERY_TIMEOUT_SECS));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::protocol::encode_response;
    use crate::storage::{IndexType, StorageEngine};
    use serde_json::{json, Value};
    use std::sync::Arc;

    /// An index equality naming at most `INLINE_INDEX_ROWS` documents is
    /// answered on the async worker, one naming more on the blocking pool;
    /// either way the client receives the bytes the ordinary path's rows
    /// encode to.
    #[tokio::test]
    async fn small_and_large_index_reads_send_the_ordinary_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
        engine.create_database("d".to_string()).unwrap();
        let db = engine.get_database("d").unwrap();
        db.create_collection("c".to_string(), None).unwrap();
        let coll = db.get_collection("c").unwrap();
        coll.create_index(
            "idx_g".into(),
            vec!["g".into()],
            IndexType::Persistent,
            false,
        )
        .unwrap();
        // More documents than a scan may read inline, so only the index
        // read decides.
        let docs = (0..300)
            .map(|i| json!({"_key": format!("k{i:03}"), "g": if i < 3 { "few" } else { "many" }, "n": i}))
            .collect();
        coll.insert_batch(docs).unwrap();
        let h = DriverHandler::new(engine.clone(), None);
        let q = "FOR d IN c FILTER d.g == @g RETURN d";
        for (g, n) in [("few", 3), ("many", 297), ("none", 0)] {
            let vars = HashMap::from([("g".to_string(), json!(g))]);
            let resp = handle_query(&h, "d".into(), q.into(), Some(vars.clone()), false).await;
            assert!(matches!(resp, Response::RawRows { .. }), "{g}: {resp:?}");
            let ordinary = QueryExecutor::with_database_and_bind_vars(&engine, "d".into(), vars)
                .execute(&crate::sdbql::parse(q).unwrap())
                .unwrap();
            assert_eq!(ordinary.len(), n, "{g}");
            assert_eq!(
                encode_response(&resp).unwrap(),
                encode_response(&Response::ok(Value::Array(ordinary))).unwrap(),
                "{g}"
            );
        }
    }

    /// The SORT … LIMIT reads in index order of `benchmarks/compare` —
    /// `recent`, copied from the stored bytes, and `join`, evaluated with a
    /// DOCUMENT per row (one customer missing) — are answered on the async
    /// worker when the LIMIT keeps at most `INLINE_INDEX_ROWS` rows, and on
    /// the blocking pool past that; the client receives the ordinary path's
    /// bytes either way.
    #[tokio::test]
    async fn sorted_reads_and_their_joins_send_the_ordinary_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let engine = Arc::new(StorageEngine::new(tmp.path().to_str().unwrap()).unwrap());
        engine.create_database("d".to_string()).unwrap();
        let db = engine.get_database("d").unwrap();
        for name in ["orders", "customers"] {
            db.create_collection(name.to_string(), None).unwrap();
        }
        let orders = db.get_collection("orders").unwrap();
        orders
            .create_index(
                "idx_t".into(),
                vec!["t".into()],
                IndexType::Persistent,
                false,
            )
            .unwrap();
        let docs = (0..400)
            .map(|i| {
                json!({
                    "_key": format!("o{i:03}"),
                    "t": format!("2025-01-{:02}T{:02}", 1 + i % 3, i % 24),
                    "c": format!("c{}", i % 7),
                    "amount": i as f64 / 4.0,
                    "items": [{"sku": i, "qty": 1}],
                })
            })
            .collect();
        orders.insert_batch(docs).unwrap();
        let customers = (0..6)
            .map(|c| json!({"_key": format!("c{c}"), "name": format!("Customer {c}"), "tier": c % 2}))
            .collect();
        db.get_collection("customers")
            .unwrap()
            .insert_batch(customers)
            .unwrap();

        let h = DriverHandler::new(engine.clone(), None);
        let vars = HashMap::from([
            ("lo".to_string(), json!("2025-01-02")),
            ("hi".to_string(), json!("2025-01-03")),
        ]);
        let window = "FOR o IN orders FILTER o.t >= @lo AND o.t < @hi SORT o.t DESC";
        let join = "LET c = DOCUMENT(CONCAT('customers/', o.c)) \
                    RETURN {order: o._key, amount: o.amount, name: c.name, tier: c.tier}";
        for (limit, inline) in [(20, true), (64, true), (100, false)] {
            for (q, raw) in [
                (format!("{window} LIMIT {limit} RETURN o"), true),
                (format!("{window} LIMIT {limit} {join}"), false),
            ] {
                let query = crate::sdbql::parse(&q).unwrap();
                let executor =
                    QueryExecutor::with_database_and_bind_vars(&engine, "d".into(), vars.clone());
                // What `handle_query` decides on.
                let answered_inline = if raw {
                    matches!(
                        executor.execute_raw_msgpack_small(&query, INLINE_INDEX_ROWS),
                        Ok(SmallRaw::Rows(_))
                    )
                } else {
                    per_row_work_is_light(&query)
                        && matches!(
                            executor.execute_small_sorted(&query, INLINE_INDEX_ROWS),
                            Ok(Some(_))
                        )
                };
                assert_eq!(answered_inline, inline, "{q}");

                let resp = handle_query(&h, "d".into(), q.clone(), Some(vars.clone()), false).await;
                assert_eq!(matches!(resp, Response::RawRows { .. }), raw, "{q}");
                let ordinary = executor.execute(&query).unwrap();
                assert_eq!(ordinary.len(), limit.min(133), "{q}");
                assert_eq!(
                    encode_response(&resp).unwrap(),
                    encode_response(&Response::ok(Value::Array(ordinary))).unwrap(),
                    "{q}"
                );
            }
        }
    }
}
